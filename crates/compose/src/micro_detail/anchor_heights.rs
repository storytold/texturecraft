use texturecraft_core::BlockScales;

/// An anchor point's height over the whole texture set, in the working atlas's layout
pub struct AnchorHeights {
    data: Vec<f32>,
    working_size: (u32, u32),
    scales: BlockScales,
}

impl AnchorHeights {
    pub fn new(data: Vec<f32>, working_size: (u32, u32), scales: BlockScales) -> AnchorHeights {
        AnchorHeights { data, working_size, scales }
    }

    pub(super) fn matches_layout(&self, size: (u32, u32), scales: &BlockScales) -> bool {
        self.working_size == size && &self.scales == scales
    }

    pub(super) fn bytes(&self) -> usize {
        self.data.len().saturating_mul(size_of::<f32>())
    }

    /// The height under texel (`x`, `y`) of a `layout` sized mesh map (0 outside the working atlas)
    pub(super) fn at(&self, (x, y): (u32, u32), layout: (u32, u32)) -> f32 {
        let baked_uv = [(x as f32 + 0.5) / layout.0.max(1) as f32, (y as f32 + 0.5) / layout.1.max(1) as f32];
        let Some(uv) = self.scales.working_uv(baked_uv, self.working_size) else { return 0.0 };

        let column = ((uv[0] * self.working_size.0 as f32) as u32).min(self.working_size.0.saturating_sub(1));
        let row = ((uv[1] * self.working_size.1 as f32) as u32).min(self.working_size.1.saturating_sub(1));
        self.data.get((row * self.working_size.0 + column) as usize).copied().unwrap_or(0.0)
    }
}
