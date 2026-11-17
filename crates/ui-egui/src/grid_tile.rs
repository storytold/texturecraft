//! A grid tile's measurements in the shelf's small, medium and large views

/// A grid tile's size and its thumbnail's size and center below the tile's top, in points
pub(crate) struct GridTile {
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) thumbnail_size: f32,
    pub(crate) thumbnail_center_y: f32,
}
