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
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).map_err(|e| format!("no GPU adapter: {e}"))?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).map_err(|e| format!("no GPU device: {e}"))?;
        let renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        Ok(Headless { device, queue, renderer, adapter_name })
    }

    /// Render a frame and read it back as RGBA8 (sRGB-encoded).
    pub fn render(&mut self, frame: &crate::Frame) -> Option<(u32, u32, Vec<u8>)> {
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_headless") });
        self.renderer.prepare(&self.device, &self.queue, &mut enc, frame);
        if frame.shade.custom_shader.is_some() && frame.shade.view_mode == 0 && matches!(frame.kind, crate::ViewKind::Scene) && !self.renderer.shader_ready() {
            return None;
        }
        self.queue.submit([enc.finish()]);
        self.renderer.read_view(&self.device, &self.queue, frame.view_id)
    }
}
