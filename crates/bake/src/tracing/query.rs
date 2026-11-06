use texturecraft_geom::glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Query {
    pub pos: Vec3,
    pub nrm: Vec3,
    pub geo: Vec3,
    pub rot: [f32; 2],
    pub key: u32,
    pub mesh: u32,
    /// A texel's antialiasing subsamples split its `Hemisphere::rays` into `subsets` interleaved
    /// subsets: this query casts rays `subset, subset + subsets, …`. One query per texel: 0 of 1.
    pub subset: u32,
    pub subsets: u32,
}
