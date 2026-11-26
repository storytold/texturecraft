//! What a wear generator reads at a texel

/// What a wear generator reads at a texel
pub(crate) struct WearInputs {
    /// The curvature map minus its flat 0.5 (above 0 convex, below 0 concave)
    pub curvature: f32,
    /// The curvature deviation blurred to the sigma the kind asks for ([`crate::wear::blur_sigma`])
    pub blurred_curvature: f32,
    /// Ambient occlusion (1 open, 0 occluded)
    pub occlusion: f32,
    /// The grunge pattern normalised to mean 0 and spread 1
    pub grunge: f32,
}
