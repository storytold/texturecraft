//! One side of a UV seam

use glam::Vec2;

/// One side of a [`UvSeam`](crate::UvSeam): an island edge in UV space and the third corner of its triangle (which side the island is on)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvSeamSide {
    /// Mesh triangle incident to this side of the seam
    pub triangle: u32,
    pub start: Vec2,
    pub end: Vec2,
    pub interior: Vec2,
}
