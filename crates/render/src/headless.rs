//! A windowless wgpu device for CLI renders, screenshots and GPU tests.

use crate::Renderer;

pub struct Headless {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub renderer: Renderer,
    pub adapter_name: String,
}

impl Headless {
    /// Create a device on the default adapter. Errors when no GPU (or software) adapter exists.
    pub fn new() -> Result<Headless, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).map_err(|e| format!("no GPU adapter: {e}"))?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).map_err(|e| format!("no GPU device: {e}"))?;
        let renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        Ok(Headless { device, queue, renderer, adapter_name })
    }

    /// Hands what the queue holds (texture uploads) to the device and waits until it is taken, so that the uploads'
    /// staging memory is free again.
    pub fn flush(&self) {
        self.queue.submit(std::iter::empty::<wgpu::CommandBuffer>());
        let _ = self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
    }

    /// Render a frame and read it back as RGBA8 (sRGB-encoded).
    pub fn render(&mut self, frame: &crate::Frame) -> Option<(u32, u32, Vec<u8>)> {
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_headless") });
        self.renderer.prepare(&self.device, &self.queue, &mut enc, frame);
        self.queue.submit([enc.finish()]);
        self.renderer.read_view(&self.device, &self.queue, frame.view_id)
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
