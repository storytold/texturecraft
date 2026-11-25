//! A windowless wgpu device for CLI renders, screenshots and GPU tests.

use crate::Renderer;

pub struct Headless {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub renderer: Renderer,
    pub adapter_name: String,
    /// A test's turn on the GPU ([`Headless::for_tests`]), dropped after the device
    _serial: Option<serial::Turn>,
    /// Whether this is the device the tests of the process share ([`Headless::for_tests`]),
    /// whose compiled pipelines they share too
    #[cfg(all(test, not(target_arch = "wasm32")))]
    pub(crate) shared_test_device: bool,
}

impl Headless {
    /// Create a device on the default adapter with every limit it reports. Errors when no GPU
    /// (or software) adapter exists.
    pub fn new() -> Result<Headless, String> {
        // The render crate's tests take turns and share a software adapter's device (`for_tests`)
        #[cfg(all(test, not(target_arch = "wasm32")))]
        return Self::for_tests();
        #[allow(unreachable_code)]
        Self::with_limits(crate::device_limits)
    }

    /// [`Headless::new`] for a test: the device lives in a turn of its own, so no two tests of
    /// the process touch a GPU at once. Wgpu's GL backend gives up on a context another thread
    /// holds for long (llvmpipe on few cores), and the NVIDIA driver can deadlock when one test
    /// destroys its device while another records commands on its own. A thread that already
    /// holds a turn takes another at once.
    ///
    /// In the render crate's tests and with the `test-support` feature, the tests of a process
    /// also share one device and its compiled pipelines on a software adapter (llvmpipe, lavapipe:
    /// compiling dominates their time, and llvmpipe keeps what it compiled until the process ends).
    /// A hardware adapter gives each test a device of its own, which its end destroys.
    pub fn for_tests() -> Result<Headless, String> {
        let turn = serial::Turn::take();
        #[cfg(all(any(test, feature = "test-support"), not(target_arch = "wasm32")))]
        if let Some((device, queue, adapter_name)) = shared_test_device()? {
            let mut renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
            renderer.share_test_pipelines();
            return Ok(Headless {
                device,
                queue,
                renderer,
                adapter_name,
                _serial: Some(turn),
                #[cfg(all(test, not(target_arch = "wasm32")))]
                shared_test_device: true,
            });
        }
        let mut headless = Self::with_limits(crate::device_limits)?;
        headless._serial = Some(turn);
        Ok(headless)
    }

    /// [`Headless::new`] with the limits `limits` picks for the adapter (a test of a smaller GPU).
    pub fn with_limits(limits: impl FnOnce(&wgpu::Adapter) -> wgpu::Limits) -> Result<Headless, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).map_err(|e| format!("no GPU adapter: {e}"))?;
        let adapter_name = adapter.get_info().name;
        // A device of the render crate's tests lives in a turn (`for_tests`)
        #[cfg(all(test, not(target_arch = "wasm32")))]
        let serial = Some(serial::Turn::take());
        #[cfg(not(all(test, not(target_arch = "wasm32"))))]
        let serial = None;
        let descriptor = wgpu::DeviceDescriptor { required_limits: limits(&adapter), ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).map_err(|e| format!("no GPU device: {e}"))?;
        let renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        Ok(Headless {
            device,
            queue,
            renderer,
            adapter_name,
            _serial: serial,
            #[cfg(all(test, not(target_arch = "wasm32")))]
            shared_test_device: false,
        })
    }

    /// Render a frame and read it back as RGBA8 (sRGB-encoded).
    pub fn render(&mut self, frame: &crate::Frame) -> Option<(u32, u32, Vec<u8>)> {
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_headless") });
        self.renderer.prepare(&self.device, &self.queue, &mut enc, frame);
        self.queue.submit([enc.finish()]);
        self.renderer.read_view(&self.device, &self.queue, frame.view_id)
    }
}

/// The device of [`shared_test_device`]
#[cfg(all(any(test, feature = "test-support"), not(target_arch = "wasm32")))]
static SHARED_TEST_DEVICE: std::sync::OnceLock<Result<Option<SharedTestDevice>, String>> = std::sync::OnceLock::new();

/// A device, its queue and its adapter's name
#[cfg(all(any(test, feature = "test-support"), not(target_arch = "wasm32")))]
type SharedTestDevice = (wgpu::Device, wgpu::Queue, String);

/// The device a process's tests share on a software adapter ([`Headless::for_tests`]); None on
/// a hardware adapter
#[cfg(all(any(test, feature = "test-support"), not(target_arch = "wasm32")))]
fn shared_test_device() -> Result<Option<SharedTestDevice>, String> {
    SHARED_TEST_DEVICE
        .get_or_init(|| {
            let gpu = Headless::with_limits(crate::device_limits)?;
            let shares = shares_test_device(&gpu.device.adapter_info());
            Ok(shares.then_some((gpu.device, gpu.queue, gpu.adapter_name)))
        })
        .clone()
}

/// Whether tests share one device on this adapter: a software one (GL here is llvmpipe or a
/// desktop driver behind EGL, which keeps its compiled programs as long, so GL always shares)
#[cfg(all(any(test, feature = "test-support"), not(target_arch = "wasm32")))]
pub(crate) fn shares_test_device(info: &wgpu::AdapterInfo) -> bool {
    info.backend == wgpu::Backend::Gl || info.device_type == wgpu::DeviceType::Cpu
}

/// Turns of tests on the GPU ([`Headless::for_tests`]). A thread already holding a turn (a test
/// with two devices) takes another at once.
#[doc(hidden)]
pub mod serial {
    use std::cell::Cell;
    use std::sync::{Condvar, Mutex, PoisonError};

    static BUSY: Mutex<bool> = Mutex::new(false);
    static FREED: Condvar = Condvar::new();
    thread_local! {
        static HELD: Cell<u32> = const { Cell::new(0) };
    }

    /// One turn; the first one a thread takes waits for the device of another thread to go.
    pub struct Turn {
        first: bool,
    }

    impl Turn {
        pub fn take() -> Turn {
            let first = HELD.get() == 0;
            if first {
                let mut busy = BUSY.lock().unwrap_or_else(PoisonError::into_inner);
                while *busy {
                    busy = FREED.wait(busy).unwrap_or_else(PoisonError::into_inner);
                }
                *busy = true;
            }
            HELD.set(HELD.get() + 1);
            Turn { first }
        }
    }

    impl Drop for Turn {
        fn drop(&mut self) {
            HELD.set(HELD.get().saturating_sub(1));
            if self.first {
                *BUSY.lock().unwrap_or_else(PoisonError::into_inner) = false;
                FREED.notify_all();
            }
        }
    }
}

#[cfg(test)]
mod limits_tests {
    /// The device gets every limit the adapter reports (`device_limits`), so the compositor
    /// plans against the adapter's full range.
    #[test]
    fn the_device_gets_the_adapters_limits() {
        let Some(gpu) = crate::gpu_composite::oracle::headless() else { return };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
        assert_eq!(adapter.get_info().name, gpu.adapter_name);
        assert_eq!(gpu.device.limits(), adapter.limits());
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::Headless;

    #[test]
    fn windows_ci_uses_lavapipe() {
        if std::env::var_os("TC_CI_LAVAPIPE").is_none() {
            return;
        }
        let gpu = Headless::new().expect("the Windows CI Vulkan adapter must be available");
        let adapter = gpu.device.adapter_info();
        assert_eq!(adapter.backend, wgpu::Backend::Vulkan, "{adapter:?}");
        assert_eq!(adapter.device_type, wgpu::DeviceType::Cpu, "{adapter:?}");
        assert!(adapter.name.starts_with("llvmpipe"), "{adapter:?}");
        assert!(crate::gpu_composite::GpuCompositor::supports(&gpu.device), "{adapter:?}");
        eprintln!("Windows CI renderer adapter: {adapter:?}");
    }
}
