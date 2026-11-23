//! Where a fill's tiles land, as the window preview draws them

/// How the fill's image is laid on its tile: tiles across and down, rotation in degrees, offset in tiles (rotate, scale, then offset)
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub tiling: [f32; 2],
    pub rotation: f32,
    pub offset: [f32; 2],
}
