//! The preview camera laid on the square: where a point shows and whether the shape hides it

use egui::{Pos2, Rect, vec2};
use texturecraft_engine::geom::glam::Vec3;

use super::{CAMERA_DISTANCE, PreviewShape, View, intersect};

/// The preview's view laid on the square `rect`, `half` mesh radii each way from its middle
pub(super) struct Screen {
    pub(super) view: View,
    pub(super) rect: Rect,
    pub(super) half: f32,
}

/// A hit this close in front of the point is the point's own surface
const OWN_SURFACE_DEPTH: f32 = 1e-3;

impl Screen {
    /// Where `point` shows on the square
    pub(super) fn at(&self, point: Vec3) -> Pos2 {
        let per_radius = self.rect.width() * 0.5 / self.half;
        self.rect.center() + vec2(point.dot(self.view.right), -point.dot(self.view.up)) * per_radius
    }

    /// Whether `shape` hides `point` from the camera
    pub(super) fn hidden(&self, shape: PreviewShape, point: Vec3) -> bool {
        let camera_offset = CAMERA_DISTANCE - point.dot(self.view.eye);
        let origin = point + self.view.eye * camera_offset;
        intersect(shape, origin, self.view.forward).is_some_and(|hit| (hit.pos - point).dot(self.view.eye) > OWN_SURFACE_DEPTH)
    }
}
