use texturecraft_geom::Surface;

pub(crate) struct Sample {
    pub x: usize,
    pub surface: Surface,
    pub bary: [f32; 2],
    /// The texel's seed, shared by its subsamples
    pub seed: u64,
    /// The subsample's index among its texel's covered subsamples, and their count
    pub subset: u32,
    pub subsets: u32,
}
