//! A windowless wgpu device for CLI renders, screenshots and GPU tests.

use crate::Renderer;

pub struct Headless {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub renderer: Renderer,
    pub adapter_name: String,
}

/// Limits needed by the headless viewport renderer: use full compute-capable limits when
/// `adapter_limits` supports them, otherwise fall back to fragment-safe downlevel limits.
pub fn device_limits(adapter_limits: &wgpu::Limits) -> wgpu::Limits {
    let defaults = wgpu::Limits::default();
    let has_compute = adapter_limits.max_compute_workgroups_per_dimension >= defaults.max_compute_workgroups_per_dimension
        && adapter_limits.max_compute_workgroup_storage_size >= defaults.max_compute_workgroup_storage_size
        && adapter_limits.max_compute_invocations_per_workgroup >= defaults.max_compute_invocations_per_workgroup
        && adapter_limits.max_storage_buffers_per_shader_stage >= defaults.max_storage_buffers_per_shader_stage;
    let base = if has_compute { defaults } else { wgpu::Limits::downlevel_webgl2_defaults() };
    base.using_resolution(adapter_limits.clone())
}

impl Headless {
    /// Create a device on the default adapter. Errors when no GPU (or software) adapter exists.
    pub fn new() -> Result<Headless, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let options = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::from_env().unwrap_or_default(), ..Default::default() };
        let adapter = pollster::block_on(instance.request_adapter(&options)).map_err(|e| format!("no GPU adapter: {e}"))?;
        let adapter_name = adapter.get_info().name;
        let desc = wgpu::DeviceDescriptor { required_limits: device_limits(&adapter.limits()), ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&desc)).map_err(|e| format!("no GPU device: {e}"))?;
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
mod tests {
    use super::device_limits;

    #[test]
    fn headless_limits_do_not_require_compute_shaders_on_downlevel_adapters() {
        let no_compute = wgpu::Limits::downlevel_webgl2_defaults();
        let resolved = device_limits(&no_compute);
        assert_eq!(resolved.max_compute_workgroups_per_dimension, 0);
        assert_eq!(resolved.max_compute_workgroup_storage_size, 0);

        let full = wgpu::Limits::default();
        let full_resolved = device_limits(&full);
        assert!(full_resolved.max_compute_workgroups_per_dimension >= full.max_compute_workgroups_per_dimension);
    }
}
