//! The 2D view's mapping between UV space and screen points. The texture is fitted to the view,
//! zoomed, turned about its centre (in pixels, so it stays square) and then panned.

use egui::{Pos2, Rect, Vec2, pos2};

/// How the 2D view lays the texture set out in its `rect`
#[derive(Clone, Copy, Debug)]
pub(crate) struct UvViewMap {
    pub rect: Rect,
    /// Clip-space half extents of the texture at the current zoom (`texturecraft_render::uv_view_scale`)
    pub scale: [f32; 2],
    /// Clip-space offset after the rotation
    pub pan: [f32; 2],
    /// Counter-clockwise on screen, in radians
    pub rotation: f32,
}

impl UvViewMap {
    /// The screen point of texture coordinates (`u`, `v`), `v` pointing down
    pub fn to_screen(self, u: f32, v: f32) -> Pos2 {
        let half = self.rect.size() * 0.5;
        let (sin, cos) = self.rotation.sin_cos();
        let (x, y) = ((u * 2.0 - 1.0) * self.scale[0] * half.x, (1.0 - v * 2.0) * self.scale[1] * half.y);
        let (turned_x, turned_y) = (cos * x - sin * y, sin * x + cos * y);
        let center = self.rect.center();
        pos2(center.x + turned_x + self.pan[0] * half.x, center.y - (turned_y + self.pan[1] * half.y))
    }

    /// The texture coordinates under screen point `pointer`
    pub fn to_uv(self, pointer: Pos2) -> texturecraft_engine::geom::glam::Vec2 {
        let half = self.rect.size() * 0.5;
        let (sin, cos) = self.rotation.sin_cos();
        let center = self.rect.center();
        let (x, y) = (pointer.x - center.x - self.pan[0] * half.x, center.y - pointer.y - self.pan[1] * half.y);
        let (unturned_x, unturned_y) = (cos * x + sin * y, -sin * x + cos * y);
        texturecraft_engine::geom::glam::Vec2::new((unturned_x / (self.scale[0] * half.x) + 1.0) * 0.5, (1.0 - unturned_y / (self.scale[1] * half.y)) * 0.5)
    }
}

/// Turns the view by `radians` about the view's centre: the rotation changes and the pan, an offset of the texture from that centre, turns with it
pub(crate) fn turn_about_center(rotation: &mut f32, pan: &mut [f32; 2], size: Vec2, radians: f32) {
    let half = size * 0.5;
    let (sin, cos) = radians.sin_cos();
    let (x, y) = (pan[0] * half.x, pan[1] * half.y);
    *pan = [(cos * x - sin * y) / half.x.max(1e-6), (sin * x + cos * y) / half.y.max(1e-6)];
    *rotation = (*rotation + radians).rem_euclid(std::f32::consts::TAU);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(rotation: f32) -> UvViewMap {
        UvViewMap { rect: Rect::from_min_size(pos2(10.0, 20.0), Vec2::new(400.0, 200.0)), scale: [0.5, 0.8], pan: [0.1, -0.2], rotation }
    }

    #[test]
    fn screen_and_uv_are_inverses_at_any_rotation() {
        for rotation in [0.0, 0.7, 2.0, -1.3] {
            let view = map(rotation);
            let uv = view.to_uv(view.to_screen(0.3, 0.8));
            assert!((uv.x - 0.3).abs() < 1e-4 && (uv.y - 0.8).abs() < 1e-4, "{rotation}: {uv:?}");
        }
    }

    #[test]
    fn a_turn_keeps_the_texture_edges_their_length_and_at_right_angles() {
        let view = map(0.9);
        let (a, b, c) = (view.to_screen(0.0, 0.0), view.to_screen(1.0, 0.0), view.to_screen(0.0, 1.0));
        assert!(((b - a).length() - 200.0).abs() < 1e-3 && ((c - a).length() - 160.0).abs() < 1e-3, "{a:?} {b:?} {c:?}");
        assert!((b - a).dot(c - a).abs() < 1e-2);
    }

    #[test]
    fn turning_about_the_center_moves_the_texture_like_the_view_turn() {
        let size = Vec2::new(400.0, 200.0);
        let (mut rotation, mut pan) = (0.0, [0.5, 0.0]);
        turn_about_center(&mut rotation, &mut pan, size, std::f32::consts::FRAC_PI_2);
        let view = UvViewMap { rect: Rect::from_min_size(pos2(0.0, 0.0), size), scale: [0.5, 0.5], pan, rotation };

        // The texture's center started 100 points right of the view's center and now sits 100 points above it
        let center = view.to_screen(0.5, 0.5);
        assert!((center.x - 200.0).abs() < 1e-3 && center.y.abs() < 1e-3, "{center:?}");
    }
}
