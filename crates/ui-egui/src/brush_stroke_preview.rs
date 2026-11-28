//! The live stroke at the top of the paint tool's properties: the current brush's own stamps along a hand-drawn gesture

use std::sync::{Arc, Weak};

use egui::{Color32, ColorImage, Context, Id, Painter, Rect, TextureHandle, TextureOptions, pos2, vec2};
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::paint::BrushSettings;

use crate::preview_image::stroke_strip;

/// The stroke's margin from the edges of its half of the preview, in points
const STROKE_MARGIN: f32 = 6.0;

/// The gesture the stroke follows, as fractions of its box: a small hook, a dip, a tall arch and a long tail down
const STROKE_PATH: [[f32; 2]; 5] = [[0.0, 0.6], [0.18, 0.3], [0.4, 0.8], [0.65, 0.0], [1.0, 1.0]];

/// The gesture's widest width for its height, so a wide preview centres it instead of flattening its arch
const STROKE_ASPECT: f32 = 1.4;

/// The pen pressure the stroke is drawn with, from its start to its end: it presses harder as it goes, as a hand does
const STROKE_PRESSURE: std::ops::RangeInclusive<f32> = 0.35..=1.0;

/// The stroke's width: [`STROKE_WIDTH_PER_ROOT_SIZE`] points times the root of the brush size, kept inside [`STROKE_WIDTH`]; the root lets small brushes read and big ones still grow
const STROKE_WIDTH_PER_ROOT_SIZE: f32 = 70.0;
const STROKE_WIDTH: std::ops::RangeInclusive<f32> = 2.0..=32.0;

/// The rendered stroke and what it was rendered from, kept in egui's memory behind an `Arc` so a frame with the same brush reuses it without copying it
pub(crate) struct BrushStrokePreview {
    brush: BrushSettings,
    // A hidden picture keeps its texture without owning the decoded alpha
    image: Option<Weak<ImageRes>>,
    tints: Vec<[u8; 3]>,
    size: [usize; 2],
    pixels_per_point: f32,
    texture: TextureHandle,
}

impl BrushStrokePreview {
    /// Paint the brush's stroke across `rect`, each pixel column coloured by `tint` of its position 0..1; it renders again only on a new brush, alpha image, colour or size
    pub(crate) fn paint(ctx: &Context, painter: &Painter, rect: Rect, brush: &BrushSettings, image: Option<&Arc<ImageRes>>, tint: impl Fn(f32) -> [u8; 3]) {
        Self::paint_picture(ctx, painter, rect, "brush_stroke_preview", (brush, image), tint, stroke_image);
    }

    /// Paint `render`'s picture of the brush across `rect`, kept under `key` until the brush, alpha image, colour or size change
    pub(crate) fn paint_picture(
        ctx: &Context,
        painter: &Painter,
        rect: Rect,
        key: &'static str,
        (brush, image): (&BrushSettings, Option<&Arc<ImageRes>>),
        tint: impl Fn(f32) -> [u8; 3],
        render: impl FnOnce(&BrushSettings, Option<&ImageRes>, [usize; 2], f32, &[[u8; 3]]) -> ColorImage,
    ) {
        let pixels_per_point = ctx.pixels_per_point();
        let size = [(rect.width() * pixels_per_point).round().max(1.0) as usize, (rect.height() * pixels_per_point).round().max(1.0) as usize];
        let column_tint = |x: usize| tint((x as f32 + 0.5) / size[0] as f32);

        // The docked panel and the right-click popup draw the same picture at their own widths: one cache each
        let id = Id::unique(key).with(painter.layer_id());
        let cached = ctx.data(|data| data.get_temp::<Arc<BrushStrokePreview>>(id)).filter(|preview| {
            preview.brush == *brush
                && preview.size == size
                && preview.pixels_per_point == pixels_per_point
                && preview.image.as_ref().map(Weak::as_ptr) == image.map(Arc::as_ptr)
                && preview.tints.iter().enumerate().all(|(x, shown)| *shown == column_tint(x))
        });

        let texture = match cached {
            Some(preview) => preview.texture.id(),
            None => {
                let tints: Vec<[u8; 3]> = (0..size[0]).map(column_tint).collect();
                let picture = render(brush, image.map(|image| &**image), size, pixels_per_point, &tints);
                let texture = ctx.load_texture(key, picture, TextureOptions::LINEAR);
                let texture_id = texture.id();
                let preview = BrushStrokePreview { brush: brush.clone(), image: image.map(Arc::downgrade), tints, size, pixels_per_point, texture };
                ctx.data_mut(|data| data.insert_temp(id, Arc::new(preview)));
                texture_id
            }
        };
        painter.image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// The brush's stroke as a `size` image at `pixels_per_point`: a hand-drawn gesture as wide as the brush size, narrowed towards Minimum Size at its light start; column `x` in `tints[x]`
pub(crate) fn stroke_image(brush: &BrushSettings, image: Option<&ImageRes>, size: [usize; 2], pixels_per_point: f32, tints: &[[u8; 3]]) -> ColorImage {
    let (width, height) = (size[0] as f32 / pixels_per_point, size[1] as f32 / pixels_per_point);
    let stroke_width = (brush.size.max(0.0).sqrt() * STROKE_WIDTH_PER_ROOT_SIZE).clamp(*STROKE_WIDTH.start(), *STROKE_WIDTH.end());
    let inset = STROKE_MARGIN + stroke_width * 0.5;
    let span_y = (height - 2.0 * inset).max(0.0);
    let span = vec2((width - 2.0 * inset).clamp(0.0, span_y * STROKE_ASPECT), span_y);
    let left = (width - span.x) * 0.5;
    let curve = |t: f32| {
        let [x, y] = path_point(t);
        ((left + x * span.x) * pixels_per_point, (inset + y * span.y) * pixels_per_point)
    };
    let radius = |t: f32| {
        let pressure = STROKE_PRESSURE.start() + (STROKE_PRESSURE.end() - STROKE_PRESSURE.start()) * t * (2.0 - t);
        stroke_width * 0.5 * pixels_per_point * brush.uv_stamp_radius(pressure) / brush.uv_stamp_radius(1.0).max(f32::MIN_POSITIVE)
    };
    let tint = |x: usize| tints.get(x).copied().unwrap_or([u8::MAX; 3]);
    stroke_strip(brush, image, size, radius, curve, tint)
}

/// The point at `t` (0..1) along [`STROKE_PATH`], a Catmull-Rom spline through its points
fn path_point(t: f32) -> [f32; 2] {
    let last = STROKE_PATH.len() - 1;
    let at = t.clamp(0.0, 1.0) * last as f32;
    let segment = (at as usize).min(last - 1);
    let u = at - segment as f32;
    let point = |index: isize| STROKE_PATH[index.clamp(0, last as isize) as usize];
    let [p0, p1, p2, p3] = [-1, 0, 1, 2].map(|offset| point(segment as isize + offset));
    let blend = |axis: usize| {
        let (a, b, c, d) = (p0[axis], p1[axis], p2[axis], p3[axis]);
        0.5 * (2.0 * b + (c - a) * u + (2.0 * a - 5.0 * b + 4.0 * c - d) * u * u + (3.0 * b - a - 3.0 * c + d) * u * u * u)
    };
    [blend(0), blend(1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: [usize; 2] = [160, 88];

    fn paint(brush: &BrushSettings) -> ColorImage {
        stroke_image(brush, None, SIZE, 1.0, &[[u8::MAX; 3]; SIZE[0]])
    }

    fn painted_pixels(picture: &ColorImage) -> usize {
        picture.pixels.iter().filter(|pixel| pixel.a() > 0).count()
    }

    fn total_paint(picture: &ColorImage) -> u64 {
        picture.pixels.iter().map(|pixel| u64::from(pixel.a())).sum()
    }

    #[test]
    fn wider_spacing_leaves_gaps_between_dabs() {
        let dense = BrushSettings { hardness: 1.0, spacing: 0.1, ..BrushSettings::default() };
        let sparse = BrushSettings { spacing: 3.0, ..dense.clone() };

        assert!(painted_pixels(&paint(&sparse)) < painted_pixels(&paint(&dense)));
    }

    #[test]
    fn a_softer_brush_lays_down_less_paint() {
        let hard = BrushSettings { hardness: 1.0, ..BrushSettings::default() };
        let soft = BrushSettings { hardness: 0.0, ..hard.clone() };

        assert!(total_paint(&paint(&soft)) < total_paint(&paint(&hard)));
    }

    #[test]
    fn a_bigger_brush_paints_a_wider_stroke() {
        let small = BrushSettings { size: 0.02, ..BrushSettings::default() };
        let big = BrushSettings { size: 0.1, ..small.clone() };

        assert!(painted_pixels(&paint(&small)) < painted_pixels(&paint(&big)));
    }

    /// Paint in the first and last quarter of the columns the stroke covers
    fn paint_at_ends(picture: &ColorImage) -> (u64, u64) {
        let [width, _] = picture.size;
        let painted = |index: &usize| picture.pixels.iter().skip(*index).step_by(width).any(|pixel| pixel.a() > 0);
        let (first, last) = ((0..width).find(painted).unwrap_or(0), (0..width).rfind(painted).unwrap_or(width));
        let quarter = (last - first) / 4;
        let paint_in = |columns: std::ops::Range<usize>| {
            picture.pixels.iter().enumerate().filter(|(index, _)| columns.contains(&(index % width))).map(|(_, pixel)| u64::from(pixel.a())).sum()
        };
        (paint_in(first..first + quarter), paint_in(last - quarter..last + 1))
    }

    #[test]
    fn the_stroke_starts_light_and_a_lower_minimum_size_thins_its_start() {
        let brush = BrushSettings { hardness: 1.0, min_size: 0.05, ..BrushSettings::default() };
        let full = BrushSettings { min_size: 1.0, ..brush.clone() };
        let (start, end) = paint_at_ends(&paint(&brush));
        let (full_start, _) = paint_at_ends(&paint(&full));

        assert!(start < end, "the stroke widens as the pen presses harder");
        assert!(start < full_start, "Minimum Size bounds how thin the light start gets");
    }

    #[test]
    fn without_size_pressure_the_stroke_keeps_its_width() {
        let brush = BrushSettings { hardness: 1.0, min_size: 0.05, size_pressure: false, ..BrushSettings::default() };
        let full = BrushSettings { min_size: 1.0, size_pressure: true, ..brush.clone() };

        assert!(paint(&brush) == paint(&full), "a mouse brush draws no taper");
    }

    #[test]
    fn changing_display_scale_redraws_a_stroke_at_the_same_pixel_size() {
        let ctx = Context::default();
        let brush = BrushSettings::default();
        let draw = |scale: f32| {
            let mut input = egui::RawInput::default();
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
            let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(SIZE[0] as f32, SIZE[1] as f32) / scale);
            let mut frame = ctx.run_ui(input, |ui| BrushStrokePreview::paint(&ctx, ui.painter(), rect, &brush, None, |_| [255; 3]));
            let upload = frame.textures_delta.set.values().flatten().find(|delta| delta.image.size() == SIZE).map(|delta| delta.image.clone());
            frame.textures_delta.clear();
            upload
        };

        let first = draw(1.0).expect("first picture");
        let scaled = draw(2.0).expect("redrawn picture");
        let reused = draw(2.0);

        let (egui::ImageData::Color(first), egui::ImageData::Color(scaled)) = (first, scaled);
        assert_ne!(first.pixels, scaled.pixels, "stroke width and margins follow the display scale");
        assert!(reused.is_none(), "an unchanged display reuses the picture");
    }

    #[test]
    fn cached_pictures_release_each_decoded_alpha_when_its_owner_drops_it() {
        const ALPHA_SIDE_PX: u32 = 1024;
        let ctx = Context::default();
        let brush = BrushSettings::default();
        let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(48.0, 32.0));
        let mut former_alphas = Vec::new();

        for (layer_name, gray) in [("docked", 0.25), ("popup", 0.75)] {
            let pixels = vec![[gray, gray, gray, 1.0]; (ALPHA_SIDE_PX * ALPHA_SIDE_PX) as usize];
            let alpha = Arc::new(ImageRes::new(ALPHA_SIDE_PX, ALPHA_SIDE_PX, pixels, false));
            let layer = egui::LayerId::new(egui::Order::Middle, Id::unique(layer_name));
            let painter = ctx.layer_painter(layer);

            BrushStrokePreview::paint(&ctx, &painter, rect, &brush, Some(&alpha), |_| [255; 3]);
            former_alphas.push(Arc::downgrade(&alpha));
        }

        let retained_bytes: usize = former_alphas.iter().filter_map(|alpha| alpha.upgrade()).map(|alpha| std::mem::size_of_val(alpha.rgba.as_slice())).sum();
        eprintln!("cached pictures retain {retained_bytes} decoded alpha bytes");
        assert_eq!(retained_bytes, 0, "pictures must not own decoded alpha pixels");
    }
}
