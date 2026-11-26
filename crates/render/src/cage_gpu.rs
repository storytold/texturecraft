/// One texture set's cage on the GPU: three corners per triangle and the wireframe's corner pairs
pub(crate) struct CageGpu {
    /// Identifies the corners' order and the edges: a new distance with the same topology only rewrites `corners`
    pub(crate) topology: u64,
    pub(crate) corners: wgpu::Buffer,
    pub(crate) corner_count: u32,
    pub(crate) edges: wgpu::Buffer,
    pub(crate) edge_index_count: u32,
    /// Corner pairs along the low poly's hard edges without a UV seam
    pub(crate) missing_seams: wgpu::Buffer,
    pub(crate) missing_seam_index_count: u32,
}
