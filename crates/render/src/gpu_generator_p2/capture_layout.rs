/// Where a capture program's anchor heights lie, as compose's `AnchorHeights::at` reads them:
/// the working atlas's size, its block side, each block's tile scale row by row (empty: every
/// tile at its block's scale) and the UV tile grid's columns
pub(super) struct CaptureLayout {
    pub(super) working: [u32; 2],
    pub(super) block: u32,
    pub(super) scales: Vec<f32>,
    pub(super) columns: u32,
}

impl CaptureLayout {
    /// The tile scale `BlockScales::working_uv` reads for the bake block starting at texel
    /// `origin` of a `map` sized bake (0: none, the working UV is the bake UV)
    pub(super) fn scale_at(&self, origin: [u32; 2], map: [u32; 2]) -> f32 {
        if self.scales.is_empty() {
            return 0.0;
        }

        let block = self.block.max(1) as f32;
        let uv = [(origin[0] as f32 + 0.5) / map[0].max(1) as f32, (origin[1] as f32 + 0.5) / map[1].max(1) as f32];
        let column = (uv[0] * self.working[0] as f32 / block).floor() as usize;
        let row = (uv[1] * self.working[1] as f32 / block).floor() as usize;
        self.scales.get(row * self.columns as usize + column).copied().unwrap_or(1.0)
    }
}
