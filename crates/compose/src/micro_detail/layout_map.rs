use texturecraft_core::BlockScales;

use super::cached_bytes::CachedBytes;

/// A map in a mesh map's layout: `size` texels, each UV tile one block
pub struct LayoutMap {
    pub(super) data: Vec<f32>,
    pub(super) size: (u32, u32),
}

impl LayoutMap {
    /// The value under working texel (`x`, `y`) of a `working_size` atlas laid out by `scales` (None outside the layout)
    pub fn at(&self, (x, y): (u32, u32), working_size: (u32, u32), scales: &BlockScales) -> Option<f32> {
        let uv = [(x as f32 + 0.5) / working_size.0.max(1) as f32, (y as f32 + 0.5) / working_size.1.max(1) as f32];
        let baked_uv = scales.bake_uv(uv, working_size)?;
        let column = ((baked_uv[0] * self.size.0 as f32) as u32).min(self.size.0.saturating_sub(1));
        let row = ((baked_uv[1] * self.size.1 as f32) as u32).min(self.size.1.saturating_sub(1));
        self.data.get((row * self.size.0 + column) as usize).copied()
    }
}

impl CachedBytes for LayoutMap {
    fn bytes(&self) -> usize {
        self.data.len().saturating_mul(size_of::<f32>())
    }
}
