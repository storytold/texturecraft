use super::field_buffers::FieldBuffers;
use super::texture;

/// Placeholder buffers and images for the bindings a pass does not read
pub(super) struct Dummies {
    pub(super) buffers: [wgpu::Buffer; 7],
    pub(super) maps_out: wgpu::TextureView,
    pub(super) maps_in: wgpu::TextureView,
    pub(super) bake: wgpu::TextureView,
    pub(super) bake_out: wgpu::TextureView,
    pub(super) capture: wgpu::TextureView,
}

impl Dummies {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let buffer = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tc_micro_unused"),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let array = wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() };
        let storage = wgpu::TextureUsages::STORAGE_BINDING;
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
        Self {
            buffers: std::array::from_fn(|_| buffer()),
            maps_out: texture(device, "tc_micro_unused_out", [1, 1, 1], wgpu::TextureFormat::R32Float, storage).create_view(&array),
            maps_in: texture(device, "tc_micro_unused_in", [1, 1, 1], wgpu::TextureFormat::R32Float, sampled).create_view(&array),
            bake: texture(device, "tc_micro_unused_bake", [1, 1, 1], wgpu::TextureFormat::Rgba32Uint, sampled).create_view(&Default::default()),
            bake_out: texture(device, "tc_micro_unused_bake_out", [1, 1, 1], wgpu::TextureFormat::Rgba32Uint, storage).create_view(&Default::default()),
            capture: texture(device, "tc_micro_unused_capture", [1, 1, 1], wgpu::TextureFormat::Rg32Float, sampled).create_view(&Default::default()),
        }
    }
}

impl FieldBuffers for Dummies {
    fn buffers(&self) -> [&wgpu::Buffer; 7] {
        self.buffers.each_ref()
    }
}
