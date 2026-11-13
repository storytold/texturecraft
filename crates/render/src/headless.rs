//! A windowless wgpu device for CLI renders, screenshots and GPU tests.

use crate::Renderer;

pub struct Headless {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub renderer: Renderer,
    pub adapter_name: String,
}

/// The adapter for work without a window (renders, path tracing, GPU baking). Like the desktop app,
/// it honours `WGPU_ADAPTER_NAME` (part of the adapter's name, any case) and `WGPU_POWER_PREF`
/// (`high`, the default, `low` or `none`); the instance reads wgpu's other variables
/// (`WGPU_BACKEND`, `WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER`, ...).
pub fn adapter() -> Result<wgpu::Adapter, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let power = wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance);
    if let Ok(name) = std::env::var("WGPU_ADAPTER_NAME") {
        let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
        let infos: Vec<wgpu::AdapterInfo> = adapters.iter().map(wgpu::Adapter::get_info).collect();
        if let Some(a) = pick_adapter(&infos, &name, power).and_then(|i| adapters.into_iter().nth(i)) {
            return Ok(a);
        }
        log::warn!("no GPU adapter matches WGPU_ADAPTER_NAME `{name}`; choosing by power preference");
    }
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: power, ..Default::default() }))
        .map_err(|e| format!("no GPU adapter: {e}"))
}

/// Which of `infos` `WGPU_ADAPTER_NAME=name` picks. Several adapters may match (the same GPU on two
/// backends, two GPUs of one vendor): rank them as the desktop app's selector does, hardware before
/// software, then `power`, then native backends before OpenGL, then the listing order.
fn pick_adapter(infos: &[wgpu::AdapterInfo], name: &str, power: wgpu::PowerPreference) -> Option<usize> {
    use wgpu::{Backend, DeviceType, PowerPreference};
    let wanted = name.to_lowercase();
    infos
        .iter()
        .enumerate()
        .filter(|(_, info)| info.name.to_lowercase().contains(&wanted))
        .min_by_key(|(index, info)| {
            let lower = info.name.to_lowercase();
            let software = info.device_type == DeviceType::Cpu || ["llvmpipe", "lavapipe", "warp"].iter().any(|s| lower.contains(s));
            let power_rank = match (power, info.device_type) {
                (PowerPreference::None, DeviceType::Cpu) => 1,
                (PowerPreference::None, _) => 0,
                (PowerPreference::LowPower, DeviceType::IntegratedGpu) | (PowerPreference::HighPerformance, DeviceType::DiscreteGpu) => 0,
                (PowerPreference::LowPower, DeviceType::DiscreteGpu) | (PowerPreference::HighPerformance, DeviceType::IntegratedGpu) => 1,
                (_, DeviceType::Other) => 2,
                (_, DeviceType::VirtualGpu) => 3,
                (_, DeviceType::Cpu) => 4,
            };
            let backend_rank = match (power, info.backend) {
                (PowerPreference::None, _) | (_, Backend::Vulkan | Backend::Metal | Backend::Dx12) => 0,
                (_, Backend::Gl) => 1,
                _ => 2,
            };
            (software, power_rank, backend_rank, *index)
        })
        .map(|(index, _)| index)
}

/// A device on `adapter` with all its limits. The viewport draws with fragment passes only, so an
/// adapter without compute shaders (WebGL2-class, or the GL driver WSL2 offers on an integrated GPU)
/// still renders it; the GPU path tracer checks the device's limits itself.
fn device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), String> {
    let desc = wgpu::DeviceDescriptor { label: Some("tc_headless"), required_limits: adapter.limits(), ..Default::default() };
    pollster::block_on(adapter.request_device(&desc)).map_err(|e| format!("no GPU device on {}: {e}", adapter.get_info().name))
}

impl Headless {
    /// Create a device on [`adapter`]. Errors when no GPU (or software) adapter exists.
    pub fn new() -> Result<Headless, String> {
        let adapter = adapter()?;
        let (device, queue) = device(&adapter)?;
        let renderer = Renderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        Ok(Headless { device, queue, renderer, adapter_name: adapter.get_info().name })
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
    use super::*;

    /// An adapter without compute shaders still gets a device for the viewport. wgpu's noop backend
    /// stands in for one (WSL2's GL driver on an Intel GPU, a WebGL2-class GPU): asking it for
    /// `Limits::default()` failed with "Limit 'max_compute_workgroups_per_dimension' value 65535 is
    /// better than allowed 0".
    #[test]
    fn an_adapter_without_compute_shaders_gets_a_device() {
        let noop = wgpu::NoopBackendOptions { enable: true, limits: Some(wgpu::Limits::downlevel_webgl2_defaults()), ..Default::default() };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions { noop, ..Default::default() },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).unwrap();
        assert_eq!(adapter.limits().max_compute_workgroups_per_dimension, 0);
        device(&adapter).unwrap();
    }

    fn info(name: &str, device_type: wgpu::DeviceType, backend: wgpu::Backend) -> wgpu::AdapterInfo {
        wgpu::AdapterInfo { name: name.into(), ..wgpu::AdapterInfo::new(device_type, backend) }
    }

    /// A `WGPU_ADAPTER_NAME` that several adapters match still follows `WGPU_POWER_PREF`, and an
    /// empty one matches them all.
    #[test]
    fn a_name_several_adapters_match_follows_the_power_preference() {
        use wgpu::{Backend::*, DeviceType::*, PowerPreference as P};
        let gpus = [
            info("Intel(R) Iris(R) Xe Graphics", IntegratedGpu, Vulkan),
            info("Intel(R) Arc(TM) A770", DiscreteGpu, Vulkan),
            info("llvmpipe (LLVM 15)", Cpu, Gl),
        ];
        assert_eq!(pick_adapter(&gpus, "intel", P::HighPerformance), Some(1));
        assert_eq!(pick_adapter(&gpus, "intel", P::LowPower), Some(0));
        assert_eq!(pick_adapter(&gpus, "INTEL", P::None), Some(0));
        assert_eq!(pick_adapter(&gpus, "", P::HighPerformance), Some(1));
        assert_eq!(pick_adapter(&gpus, "llvm", P::HighPerformance), Some(2));
        assert_eq!(pick_adapter(&gpus, "nvidia", P::HighPerformance), None);
        // One GPU on two backends: the native one before OpenGL.
        let twice = [info("D3D12 (NVIDIA GeForce RTX 4050)", DiscreteGpu, Gl), info("Microsoft Direct3D12 (NVIDIA GeForce RTX 4050)", DiscreteGpu, Vulkan)];
        assert_eq!(pick_adapter(&twice, "rtx 4050", P::HighPerformance), Some(1));
    }
}
