//! A windowless wgpu device for CLI renders, screenshots and GPU tests.

use crate::Renderer;

pub struct Headless {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub renderer: Renderer,
    pub adapter_name: String,
}

impl Headless {
    /// Create a device on the default adapter with every limit it reports. Errors when no GPU
    /// (or software) adapter exists.
    pub fn new() -> Result<Headless, String> {
        // The render crate's tests on GL share one device (and its compiled pipelines,
        // `gpu_composite`): llvmpipe keeps what it compiled until the process ends. Other backends
        // get a device per test, which the test's end destroys
        #[cfg(all(test, not(target_arch = "wasm32")))]
        {
            type Shared = Option<(wgpu::Device, wgpu::Queue, String)>;
            static SHARED: std::sync::OnceLock<Result<Shared, String>> = std::sync::OnceLock::new();
            let shared = SHARED.get_or_init(|| {
                let gpu = Self::with_limits(crate::device_limits)?;
                Ok((gpu.device.adapter_info().backend == wgpu::Backend::Gl).then_some((gpu.device, gpu.queue, gpu.adapter_name)))
            });
            if let Some((device, queue, adapter_name)) = shared.clone()? {
                let renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
                return Ok(Headless { device, queue, renderer, adapter_name });
            }
        }
        Self::with_limits(crate::device_limits)
    }

    /// [`Headless::new`] with the limits `limits` picks for the adapter (a test of a smaller GPU).
    pub fn with_limits(limits: impl FnOnce(&wgpu::Adapter) -> wgpu::Limits) -> Result<Headless, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).map_err(|e| format!("no GPU adapter: {e}"))?;
        let adapter_name = adapter.get_info().name;
        let descriptor = wgpu::DeviceDescriptor { required_limits: limits(&adapter), ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).map_err(|e| format!("no GPU device: {e}"))?;
        let renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        Ok(Headless { device, queue, renderer, adapter_name })
    }

    /// Render a frame and read it back as RGBA8 (sRGB-encoded).
    pub fn render(&mut self, frame: &crate::Frame) -> Option<(u32, u32, Vec<u8>)> {
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_headless") });
        self.renderer.prepare(&self.device, &self.queue, &mut enc, frame);
        self.queue.submit([enc.finish()]);
        self.renderer.read_view(&self.device, &self.queue, frame.view_id)
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
