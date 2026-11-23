// P1's constants that material_image.wgsl and composite_patterns.wgsl read too. The Rust module
// compiles them before composite_projection.wgsl and before its stub, so they always exist once.

// Operation.ext.y: how the fill samples its images (FLAG_* in gpu_composite_projection.rs)
const P1_NEAREST: u32 = 2u;
const P1_HQ: u32 = 4u;
const P1_CLAMP_U: u32 = 8u;
const P1_CLAMP_V: u32 = 16u;

// Operation.procedural.w of the reads P1 hands to the image and pattern code: one image point
// already projected, or up to three pattern points already placed, with their weights
const P1_POINT_PROJECTION: u32 = 17u;
const P1_PLANES_PROJECTION: u32 = 18u;

// Operation.ids.w of an image read that returns the image's alpha in every component
const P1_ALPHA_KIND: u32 = 16u;
