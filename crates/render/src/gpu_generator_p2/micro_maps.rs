/// The maps one compose reads: the micro map layers, and the bake with the claimed micro
/// curvature added for the detail sizes (None: they blur the bake itself)
pub(crate) struct MicroMaps {
    pub(crate) maps: wgpu::TextureView,
    pub(crate) bake: Option<wgpu::TextureView>,
}
