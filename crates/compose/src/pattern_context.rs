//! What evaluating a pattern needs besides the texel: the resolution it is drawn at and the images it reads

use crate::store::ImageStore;

/// What a pattern reads beyond the texel's coordinates (fields can be added; build it with `..PatternContext::default()`)
#[derive(Clone, Copy, Default)]
pub struct PatternContext<'a> {
    /// Texels per side of the image the pattern is drawn as over one repeat of the projection (see [`crate::pattern::pattern_pixels`]); None draws it exactly
    pub pixels: Option<u32>,
    /// The image resources a pattern's inputs name; None reads them as empty
    pub images: Option<&'a ImageStore>,
}
