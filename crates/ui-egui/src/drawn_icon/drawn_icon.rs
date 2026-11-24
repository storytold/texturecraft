use super::{Point, invert, pipette};

/// The largest side an icon is rendered at, in pixels
const MAX_SIDE: u32 = 256;

/// An icon drawn in code: rendered once per pixel size into a white texture and tinted where it is drawn, so it is as sharp as a font glyph at any scale
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DrawnIcon {
    /// The eyedropper: a pipette, tip at the bottom left
    Pipette,
    /// A square half clear, half solid, an arrow out of each half
    Invert,
}

impl DrawnIcon {
    /// The distance from `point` (in the icon's unit square, y down) to the icon's ink, negative inside
    pub(crate) fn distance(self, point: Point) -> f32 {
        match self {
            DrawnIcon::Pipette => pipette::glyph(point),
            DrawnIcon::Invert => invert::glyph(point),
        }
    }

    /// How much of each pixel the icon covers when it is `side` pixels square, rows top first
    pub(crate) fn coverage(self, side: u32) -> Vec<u8> {
        let scale = side as f32;
        (0..side * side)
            .map(|index| {
                let point = [((index % side) as f32 + 0.5) / scale, ((index / side) as f32 + 0.5) / scale];
                ((0.5 - self.distance(point) * scale).clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect()
    }

    /// Draws the icon `size` points square centred on `center` in `color`, on whole screen pixels
    pub(crate) fn paint(self, painter: &egui::Painter, center: egui::Pos2, size: f32, color: egui::Color32) {
        let pixels_per_point = painter.ctx().pixels_per_point();
        let side = ((size * pixels_per_point).round() as u32).clamp(1, MAX_SIDE);
        let pixels = side as f32;
        let min = ((center.to_vec2() * pixels_per_point) - egui::Vec2::splat(pixels / 2.0)).round() / pixels_per_point;
        let rect = egui::Rect::from_min_size(min.to_pos2(), egui::Vec2::splat(pixels / pixels_per_point));

        let whole = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
        painter.image(self.texture(painter.ctx(), side).id(), rect, whole, color);
    }

    /// The icon `side` pixels square as a white texture with its coverage as alpha, rendered once per size
    fn texture(self, ctx: &egui::Context, side: u32) -> egui::TextureHandle {
        let id = egui::Id::unique(("drawn_icon", self, side));

        if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
            return texture;
        }

        let rgba: Vec<u8> = self.coverage(side).into_iter().flat_map(|alpha| [u8::MAX, u8::MAX, u8::MAX, alpha]).collect();
        let image = egui::ColorImage::from_rgba_unmultiplied([side as usize; 2], &rgba);
        let texture = ctx.load_texture(format!("drawn_icon_{self:?}_{side}"), image, egui::TextureOptions::LINEAR);
        ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_is_painted_as_one_tinted_texture_on_whole_pixels() {
        let ctx = egui::Context::default();

        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            DrawnIcon::Invert.paint(painter, egui::pos2(10.3, 20.6), 14.0, egui::Color32::RED);
            DrawnIcon::Invert.paint(painter, egui::pos2(40.0, 20.0), 14.0, egui::Color32::GREEN);
        });

        let shapes: Vec<egui::Shape> = output.shapes.iter().map(|clipped| clipped.shape.clone()).collect();
        output.drop_without_applying_deltas();
        let meshes: Vec<_> = shapes.iter().filter_map(|shape| if let egui::Shape::Mesh(mesh) = shape { Some(mesh.clone()) } else { None }).collect();

        assert_eq!(meshes.len(), 2, "one textured quad per icon: {shapes:?}");

        let [first, second] = [&meshes[0], &meshes[1]];
        assert_eq!(first.texture_id, second.texture_id, "the texture is rendered once per size");

        let bounds = first.calc_bounds();
        assert_eq!((bounds.min, bounds.size()), (egui::pos2(3.0, 14.0), egui::vec2(14.0, 14.0)), "snapped to whole pixels");
        assert!(first.vertices.iter().all(|vertex| vertex.color == egui::Color32::RED), "tinted");
    }

    #[test]
    fn coverage_is_full_inside_the_ink_and_clear_outside() {
        let side = 32;
        let invert = DrawnIcon::Invert.coverage(side);
        let at = |x: f32, y: f32| invert.get(((y * side as f32) as u32 * side + (x * side as f32) as u32) as usize).copied();

        assert_eq!(invert.len(), (side * side) as usize);
        assert_eq!(at(0.7, 0.25), Some(u8::MAX), "the solid half");
        assert_eq!(at(0.3, 0.25), Some(0), "the clear half");
        assert_eq!(at(0.02, 0.02), Some(0), "outside the frame");
    }
}
