use std::sync::OnceLock;

/// The maps one compose reads: the micro map layers, each micro curvature the detail sizes read
/// added to the bake (by request), and the atlas of those detail sizes once built
pub(crate) struct MicroMaps {
    pub(crate) maps: wgpu::TextureView,
    pub(super) bakes: Vec<(usize, wgpu::TextureView)>,
    pub(super) levels: OnceLock<wgpu::TextureView>,
}

impl MicroMaps {
    /// The bakes whose detail sizes the atlas still needs (None once built, or with none to build)
    pub(crate) fn level_bakes(&self) -> Option<impl Iterator<Item = &wgpu::TextureView>> {
        (!self.bakes.is_empty() && self.levels.get().is_none()).then(|| self.bakes.iter().map(|(_, bake)| bake))
    }

    /// The detail sizes atlas of the micro curvatures (`generator_level_at` in WGSL), once built
    pub(crate) fn levels(&self) -> Option<&wgpu::TextureView> {
        self.levels.get()
    }
}
