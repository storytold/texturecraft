/// The orb's values at one point of its surface (linear)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OrbSample {
    pub(crate) base: [f32; 3],
    pub(crate) metallic: f32,
    pub(crate) roughness: f32,
    /// Tangent-space normal, OpenGL (+Y up), 0..1 per component
    pub(crate) normal: [f32; 3],
    pub(crate) emissive: [f32; 3],
    pub(crate) opacity: f32,
}
