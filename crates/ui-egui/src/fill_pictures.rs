//! Pictures of what a fill's settings do: one per projection, filtering and UV wrap choice, the window preview and the ring of radial copies

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use texturecraft_engine::core::{FillSymmetry, Filtering, ShapeCrop, UvWrap};
use texturecraft_engine::paint::FULL_TURN_DEGREES;

use crate::placement::Placement;
use crate::projection_kind::ProjectionKind;
use crate::projection_preview::{edge_markers, passed_sides};
use crate::warp_checker::warp_checker;

use crate::theme::{Tokens, lerp_color};

/// The projection glyph's height, as a share of its picture's
const PROJECTION_GLYPH_SHARE: f32 = 0.85;

/// The shape a projection wraps around, as an icon in the accent colour
pub(crate) fn projection_picture(painter: &Painter, rect: Rect, kind: ProjectionKind, tokens: &Tokens) {
    use egui_phosphor::regular as icon;
    let glyph = match kind {
        ProjectionKind::UvTile | ProjectionKind::Uv => icon::SQUARE,
        ProjectionKind::Triplanar => icon::CUBE,
        ProjectionKind::Planar | ProjectionKind::Warp => icon::SELECTION,
        ProjectionKind::Spherical => icon::GLOBE,
        ProjectionKind::Cylindrical => icon::CYLINDER,
    };
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(rect.height() * PROJECTION_GLYPH_SHARE), tokens.accent);
}

/// Cells across a magnified edge in the Filtering picture
const EDGE_CELLS: usize = 6;

/// How steeply Bilinear Sharp ramps across the magnified edge: nearly a step
const SHARP_EDGE_RAMP: f32 = 3.0;

/// How steeply Bilinear HQ ramps across the magnified edge: a soft blend
const HQ_EDGE_RAMP: f32 = 1.2;

/// A magnified diagonal edge: smooth (HQ), crisp (Sharp) or stair-stepped (Nearest)
pub(crate) fn filtering_picture(painter: &Painter, rect: Rect, filtering: Filtering, tokens: &Tokens) {
    let cell = rect.width() / EDGE_CELLS as f32;
    for i in 0..EDGE_CELLS {
        for j in 0..EDGE_CELLS {
            let across = (i as f32 - j as f32) / EDGE_CELLS as f32;
            let amount = match filtering {
                Filtering::Nearest => f32::from(u8::from(i >= j)),
                Filtering::BilinearSharp => (0.5 + across * SHARP_EDGE_RAMP).clamp(0.0, 1.0),
                Filtering::BilinearHq => (0.5 + across * HQ_EDGE_RAMP).clamp(0.0, 1.0),
            };
            let color = lerp_color(tokens.field, tokens.accent, amount);
            let min = pos2(rect.left() + i as f32 * cell, rect.top() + j as f32 * cell);
            painter.rect_filled(Rect::from_min_size(min, vec2(cell, cell)), 0, color);
        }
    }
}

/// Cells of the UV Wrap picture's grid
const WRAP_CELLS: usize = 3;

/// The gap kept around each Wrap tile, in points
const WRAP_TILE_INSET: f32 = 0.75;

/// A repeated tile's accent strength beside the own tile's
const WRAP_REPEAT_SHADE: f32 = 0.55;

/// A 3 × 3 grid: the middle tile is the image's own; filled tiles are where it repeats
pub(crate) fn wrap_picture(painter: &Painter, rect: Rect, wrap: UvWrap, tokens: &Tokens) {
    let (across, down) = wrap.repeats();
    let cell = rect.width() / WRAP_CELLS as f32;
    let middle = WRAP_CELLS / 2;
    for i in 0..WRAP_CELLS {
        for j in 0..WRAP_CELLS {
            let own = i == middle && j == middle;
            let repeated = own || (across && j == middle) || (down && i == middle) || (across && down);
            let tile = Rect::from_min_size(pos2(rect.left() + i as f32 * cell, rect.top() + j as f32 * cell), vec2(cell, cell)).shrink(WRAP_TILE_INSET);
            if own {
                painter.rect_filled(tile, 1, tokens.accent);
            } else if repeated {
                painter.rect_filled(tile, 1, tokens.accent.gamma_multiply(WRAP_REPEAT_SHADE));
            } else {
                painter.rect_stroke(tile, 1, Stroke::new(LINE_WIDTH, tokens.separator), egui::StrokeKind::Inside);
            }
        }
    }
}

/// Most tiles drawn across the window preview: beyond this the pattern is too fine to read
const PREVIEW_MAX_TILES: f32 = 24.0;

/// The fewest tiles a preview shows, so a zero Tiling cannot divide by zero
pub(crate) const PREVIEW_MIN_TILES: f32 = 1e-3;

/// Most tiles drawn along one direction of a lattice preview, whatever its map
const PREVIEW_MAX_SPAN: i32 = 64;

/// The preview checker's dark cells, as a fraction of the way from `field` to `text`
const PREVIEW_DARK: f32 = 0.18;

/// The preview checker's light cells, as a fraction of the way from `field` to `text`
const PREVIEW_LIGHT: f32 = 0.55;

/// Where the mean of the checker's dark and light cells sits between them
const CHECKER_MEAN: f32 = 0.5;

/// The width of every outline, border and ring line in these pictures
const LINE_WIDTH: f32 = 1.0;

/// A lattice preview tile's checker cells, each half a tile wide, by the corner they start at in halves of a tile; even ones light
const TILE_CELLS: [(u8, u8); 4] = [(0, 0), (1, 0), (0, 1), (1, 1)];

/// A checker cell's side in tiles: two cells across a tile
const CELL_SIDE: f32 = 0.5;

/// The window's centre (0..1) before any slide
const WINDOW_MIDDLE: f32 = 0.5;

/// A flatter map shows nothing: the window could not be undone from it
const MIN_MAP_DETERMINANT: f32 = 1e-9;

/// A window on the surface that never moves: a UV checker (2 × 2 cells per tile, accent tile borders) tiled, turned and slid as the fill is, the ring in the middle where Offset 0 puts the projection, the filled dot where its Offset carried that point
pub(crate) fn window_preview(painter: &Painter, rect: Rect, placement: Placement, tokens: &Tokens) {
    let (columns, carried) = slid_window(placement);
    lattice_preview(painter, rect, (columns, placement.offset), true, tokens);
    let position = pos2(rect.left() + carried[0] * rect.width(), rect.top() + carried[1] * rect.height());
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    crate::preview_dots::preview_dots(&painter, rect.center(), position, tokens);
    edge_markers(&painter, rect, passed_sides(std::iter::once(position), rect), tokens);
}

/// The window preview's map, and where the Offset carried the window's middle (0..1): the checker slides with it, so a drag right moves both right
fn slid_window(placement: Placement) -> ([[f32; 2]; 2], [f32; 2]) {
    let tiling = placement.tiling.map(|tiles| tiles.clamp(PREVIEW_MIN_TILES, PREVIEW_MAX_TILES));
    let (sin, cos) = placement.rotation.to_radians().sin_cos();

    // Turn, scale, then offset: one step across the window moves (cos, sin) scaled in the texture
    let columns = [[cos * tiling[0], sin * tiling[1]], [-sin * tiling[0], cos * tiling[1]]];
    let [[a, b], [c, d]] = columns;
    let det = a * d - c * b;
    let [x, y] = placement.offset;

    // The window step the Offset is worth: the map undone; the image moves against the Offset it samples at
    let moved = [(d * x - c * y) / det, (a * y - b * x) / det];
    let carried = if moved.iter().all(|value| value.is_finite()) { [WINDOW_MIDDLE - moved[0], WINDOW_MIDDLE - moved[1]] } else { [WINDOW_MIDDLE; 2] };
    (columns, carried)
}

/// The window preview for any linear map: window point (u, v) shows the texture at `columns[0] · u + columns[1] · v + shift`; without `repeat` only tile 0..1
pub(crate) fn lattice_preview(painter: &Painter, rect: Rect, (columns, shift): ([[f32; 2]; 2], [f32; 2]), repeat: bool, tokens: &Tokens) {
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect, 0, tokens.field);
    let [[a, b], [c, d]] = columns;
    let det = a * d - c * b;
    if det.abs() > MIN_MAP_DETERMINANT && det.is_finite() && shift.iter().all(|value| value.is_finite()) {
        // Texture coordinates → the window: undo the shift, then the map
        let to_window = |t: [f32; 2]| -> Pos2 {
            let (x, y) = (t[0] - shift[0], t[1] - shift[1]);
            let (u, v) = ((d * x - c * y) / det, (a * y - b * x) / det);
            pos2(rect.left() + u * rect.width(), rect.top() + v * rect.height())
        };
        let to_texture = |u: f32, v: f32| -> [f32; 2] { [a * u + c * v + shift[0], b * u + d * v + shift[1]] };
        let corners = [to_texture(0.0, 0.0), to_texture(1.0, 0.0), to_texture(0.0, 1.0), to_texture(1.0, 1.0)];
        let low = |axis: usize| corners.iter().map(|corner| corner[axis]).fold(f32::INFINITY, f32::min).floor() as i32;
        let high = |axis: usize| corners.iter().map(|corner| corner[axis]).fold(f32::NEG_INFINITY, f32::max).ceil() as i32;
        let span = |first: i32, last: i32| if repeat { first..last.min(first.saturating_add(PREVIEW_MAX_SPAN)) } else { first.max(0)..last.min(1) };
        let (dark, light) = (lerp_color(tokens.field, tokens.text, PREVIEW_DARK), lerp_color(tokens.field, tokens.text, PREVIEW_LIGHT));
        let border = Stroke::new(LINE_WIDTH, tokens.accent);

        // Subpixel repeats use the mean checker shade across the window
        if repeat && (0..2).any(|axis| high(axis).saturating_sub(low(axis)) > PREVIEW_MAX_SPAN) {
            painter.rect_filled(rect, 0, lerp_color(dark, light, CHECKER_MEAN));
        }
        for tile_x in span(low(0), high(0)) {
            for tile_y in span(low(1), high(1)) {
                for (cell_x, cell_y) in TILE_CELLS {
                    let (x0, y0) = (tile_x as f32 + f32::from(cell_x) * CELL_SIDE, tile_y as f32 + f32::from(cell_y) * CELL_SIDE);
                    let corners = [[x0, y0], [x0 + CELL_SIDE, y0], [x0 + CELL_SIDE, y0 + CELL_SIDE], [x0, y0 + CELL_SIDE]].map(to_window);
                    let color = if (cell_x + cell_y) % 2 == 0 { light } else { dark };
                    painter.add(Shape::convex_polygon(corners.to_vec(), color, Stroke::NONE));
                }
                let (x, y) = (tile_x as f32, tile_y as f32);
                painter.add(Shape::closed_line([[x, y], [x + 1.0, y], [x + 1.0, y + 1.0], [x, y + 1.0]].map(to_window).to_vec(), border));
            }
        }
    }
    painter.rect_stroke(rect, 0, Stroke::new(LINE_WIDTH, tokens.field_border), egui::StrokeKind::Inside);
}

/// How far the darker accent sits from the accent towards black
const ACCENT_SHADE: f32 = 0.36;

/// The darker accent pictures fill with, beside the accent itself
fn accent_shade(tokens: &Tokens) -> Color32 {
    lerp_color(tokens.accent, Color32::BLACK, ACCENT_SHADE)
}

/// The margin inside a small setting picture, in points
const PICTURE_INSET: f32 = 3.0;

/// Depth culling's hidden part, as a share of its picture's height
const DEPTH_UNDER_SHARE: f32 = 0.45;

/// The arrow glyph's size in the Depth picture
const DEPTH_ARROW_SIZE: f32 = 9.0;

/// The Angle picture's radius, as a share of its width
const ANGLE_RADIUS_SHARE: f32 = 0.45;

/// The warp preview's outline width
const WARP_EDGE_WIDTH: f32 = 1.5;

/// Corner radius of the crop picture's shape
const CROP_SHAPE_CORNER_RADIUS: u8 = 3;

/// Corner radius of a fill that reaches a picture's edges
const PICTURE_CORNER_RADIUS: u8 = 2;

/// Where the depth picture's surface sits, as a share of its height from the top
const DEPTH_SURFACE_SHARE: f32 = 0.25;

/// Points what lies under the surface reaches past its share, so it still reads at small sizes
const DEPTH_UNDER_EXTRA: f32 = 2.0;

/// How far the backface picture's face stays inside the picture's sides
const BACKFACE_SIDE_INSET: f32 = 2.0;

/// Shape crop: the projection inside its shape only, or past it too
pub(crate) fn crop_picture(painter: &Painter, rect: Rect, crop: ShapeCrop, tokens: &Tokens) {
    let inner = rect.shrink(PICTURE_INSET);
    match crop {
        ShapeCrop::Cropped => {
            painter.rect_filled(inner, CROP_SHAPE_CORNER_RADIUS, accent_shade(tokens));
        }
        ShapeCrop::ExtendsOutside => {
            painter.rect_filled(rect, PICTURE_CORNER_RADIUS, accent_shade(tokens));
        }
    }
    painter.rect_stroke(inner, CROP_SHAPE_CORNER_RADIUS, Stroke::new(LINE_WIDTH, tokens.text), egui::StrokeKind::Inside);
}

/// Depth culling, seen from the side: the surface the projection lands on and what lies under it
pub(crate) fn depth_picture(painter: &Painter, rect: Rect, tokens: &Tokens) {
    let top = rect.top() + rect.height() * DEPTH_SURFACE_SHARE;
    let under = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), top + rect.height() * DEPTH_UNDER_SHARE + DEPTH_UNDER_EXTRA));
    painter.rect_filled(under, PICTURE_CORNER_RADIUS, accent_shade(tokens));
    painter.line_segment([pos2(rect.left(), top), pos2(rect.right(), top)], Stroke::new(LINE_WIDTH, tokens.text));
    painter.text(
        pos2(rect.center().x, rect.top() + PICTURE_INSET),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::regular::ARROW_DOWN,
        egui::FontId::proportional(DEPTH_ARROW_SIZE),
        tokens.accent,
    );
}

/// Backface culling: a face turned away, darker while culled
pub(crate) fn backface_picture(painter: &Painter, rect: Rect, on: bool, tokens: &Tokens) {
    let (left, apex, right) = (
        pos2(rect.left() + BACKFACE_SIDE_INSET, rect.bottom() - PICTURE_INSET),
        pos2(rect.center().x, rect.top() + PICTURE_INSET),
        pos2(rect.right() - BACKFACE_SIDE_INSET, rect.bottom() - PICTURE_INSET),
    );
    painter.add(Shape::convex_polygon(vec![left, apex, right], if on { accent_shade(tokens) } else { tokens.accent }, Stroke::NONE));
    painter.line_segment([apex, right], Stroke::new(LINE_WIDTH, tokens.text));
}

/// Points around the Angle picture's arc per full turn
const ARC_POINTS: usize = 48;

/// The angle and ring pictures start at the top: a quarter turn back from +x (egui's y points down), in turns
const TOP_START_TURNS: f32 = 0.25;

/// A cylindrical projection's Angle seen from above: the slice of a turn one repeat spans
pub(crate) fn angle_picture(painter: &Painter, rect: Rect, degrees: f32, tokens: &Tokens) {
    let center = rect.center();
    let radius = rect.width() * ANGLE_RADIUS_SHARE;
    let sweep = degrees.clamp(0.0, FULL_TURN_DEGREES) / FULL_TURN_DEGREES;
    if sweep >= 1.0 {
        painter.circle_filled(center, radius, tokens.accent);
    } else {
        let steps = ((ARC_POINTS as f32 * sweep).ceil() as usize).max(1);
        let mut points = vec![center];
        for step in 0..=steps {
            let turn = (step as f32 / steps as f32 * sweep - TOP_START_TURNS) * std::f32::consts::TAU;
            points.push(pos2(center.x + radius * turn.cos(), center.y + radius * turn.sin()));
        }

        // A slice wider than half a turn is not convex: fill it as a fan from the centre
        for pair in points[1..].windows(2) {
            painter.add(Shape::convex_polygon(vec![center, pair[0], pair[1]], tokens.accent, Stroke::NONE));
        }
    }
    painter.circle_stroke(center, radius, Stroke::new(LINE_WIDTH, tokens.button_border));
}

/// How much of the preview an unpulled frame spans per mesh diagonal of its side (a new warp's frame round a ball spans about 0.62)
const WARP_SPAN_PER_DIAGONAL: f32 = 1.07;
/// The least and most of the preview a frame's side spans
const WARP_SPAN_RANGE: std::ops::RangeInclusive<f32> = 0.05..=0.95;

/// Radius of a warp preview's corner handle
const WARP_HANDLE_RADIUS: f32 = 3.5;

/// A warp's frame with its corners where they are pulled, the checker bending with them and laid as `placement` lays the image (tiling, rotation, offset); `size` is the frame's width and height in mesh diagonals, `turn` its turn round its normal (degrees), so resizing or turning the frame shows
pub(crate) fn warp_preview(painter: &Painter, rect: Rect, corners: [[f32; 2]; 4], size: [f32; 2], turn: f32, placement: Placement, tokens: &Tokens) {
    let painter = painter.with_clip_rect(rect);
    painter.rect_filled(rect, 0, tokens.field);
    let span = size.map(|side| (side * WARP_SPAN_PER_DIAGONAL).clamp(*WARP_SPAN_RANGE.start(), *WARP_SPAN_RANGE.end()));
    let (sin, cos) = turn.to_radians().sin_cos();

    // Across and up in frame sizes from its centre -> turned -> the square
    let [top_left, top_right, bottom_right, bottom_left] = corners.map(|[x, y]| {
        let (x, y) = (x * span[0], y * span[1]);
        let (x, y) = (x * cos - y * sin, x * sin + y * cos);
        pos2(rect.center().x + x * rect.width(), rect.center().y - y * rect.height())
    });
    let (dark, light) = (lerp_color(tokens.field, tokens.text, PREVIEW_DARK), lerp_color(tokens.field, tokens.text, PREVIEW_LIGHT));
    for (outline, lit) in warp_checker([top_left, top_right, bottom_right, bottom_left], placement) {
        painter.add(Shape::convex_polygon(outline, if lit { light } else { dark }, Stroke::NONE));
    }
    let edge = [top_left, top_right, bottom_right, bottom_left];
    painter.add(Shape::closed_line(edge.to_vec(), Stroke::new(WARP_EDGE_WIDTH, tokens.accent)));
    for corner in edge {
        painter.circle(corner, WARP_HANDLE_RADIUS, tokens.text, Stroke::new(LINE_WIDTH, tokens.accent));
    }
    painter.rect_stroke(rect, 0, Stroke::new(LINE_WIDTH, tokens.field_border), egui::StrokeKind::Inside);
}

/// The ring's radius, in the picture's widths
const RING_RADIUS: f32 = 0.34;

/// The radius of the axis's dot at the ring's centre
const RING_AXIS_DOT: f32 = 2.0;

/// The radius of each copy's dot on the ring
const RING_COPY_DOT: f32 = 3.5;

/// The straight pieces the ring is drawn with
const RING_SEGMENTS: usize = 64;

/// Radial copies seen along their axis: the axis at the centre (its colour), the original (white) at the top and the other copies (accent) round the ring, spaced as the engine spaces them
pub(crate) fn ring_picture(painter: &Painter, rect: Rect, symmetry: &FillSymmetry, axis_color: Color32, tokens: &Tokens) {
    painter.rect_filled(rect, 0, tokens.field);
    let center = rect.center();
    let radius = rect.width() * RING_RADIUS;
    let at = |degrees: f32| {
        let turn = (degrees - TOP_START_TURNS * FULL_TURN_DEGREES).to_radians();
        center + vec2(turn.cos(), turn.sin()) * radius
    };

    // Use the engine's clamped span for the radial ring
    let ring_end = symmetry.span.clamp(0.0, FULL_TURN_DEGREES);
    let ring: Vec<Pos2> = (0..=RING_SEGMENTS).map(|k| at(ring_end * k as f32 / RING_SEGMENTS as f32)).collect();
    painter.add(Shape::line(ring, Stroke::new(LINE_WIDTH, tokens.separator)));
    painter.circle_filled(center, RING_AXIS_DOT, axis_color);
    for (k, degrees) in copy_angles(symmetry).enumerate() {
        let color = if k == 0 { tokens.text } else { tokens.accent };
        painter.circle_filled(at(degrees), RING_COPY_DOT, color);
    }
}

/// The degrees each radial copy of `symmetry` turns, the original's 0 first, from the engine's count and spacing
fn copy_angles(symmetry: &FillSymmetry) -> impl Iterator<Item = f32> {
    let step = symmetry.step();
    (0..symmetry.copies()).map(move |k| k as f32 * step)
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::core::SymmetryKind;

    use super::*;

    /// A repeating map far denser than the tiles drawn (Scaling 64 turned 45°) still fills the whole window, its far corner too
    #[test]
    fn a_dense_repeating_preview_leaves_no_bare_field() {
        use texturecraft_engine::core::TileTransform;

        let tokens = Tokens::for_brightness(crate::theme::Brightness::MediumDark);
        let transform = TileTransform { scaling: [64.0, 64.0], rotation: 45.0, ..TileTransform::LIBRARY };
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(100.0, 100.0));
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| lattice_preview(ui.painter(), rect, transform.affine(), true, &tokens));

        // Whether a filled shape other than the bare field covers `point`
        let covered = |point: Pos2| {
            output.shapes.iter().any(|clipped| match &clipped.shape {
                Shape::Rect(shape) => shape.fill != tokens.field && shape.fill != Color32::TRANSPARENT && shape.rect.contains(point),
                Shape::Path(path) if path.fill != Color32::TRANSPARENT => {
                    let corners = &path.points;
                    let side = |index: usize| {
                        let (from, to) = (corners[index], corners[(index + 1) % corners.len()]);
                        (to - from).x * (point - from).y - (to - from).y * (point - from).x
                    };
                    let sides: Vec<f32> = (0..corners.len()).map(side).collect();
                    sides.iter().all(|side| *side >= 0.0) || sides.iter().all(|side| *side <= 0.0)
                }
                _ => false,
            })
        };
        for corner in [pos2(1.0, 1.0), pos2(99.0, 1.0), pos2(1.0, 99.0), pos2(99.0, 99.0)] {
            assert!(covered(corner), "the preview fills {corner:?}");
        }

        // No renderer takes the uploads here
        output.drop_without_applying_deltas();
    }

    /// A hand-edited span past a full turn spaces the ring's dots as the engine spaces the copies: six copies 60° apart, none on top of another
    #[test]
    fn the_ring_spaces_its_copies_as_the_engine_does() {
        let symmetry = FillSymmetry { on: true, kind: SymmetryKind::Radial, count: 6, span: 720.0, ..FillSymmetry::OFF };

        let angles: Vec<f32> = copy_angles(&symmetry).collect();

        assert_eq!(angles, [0.0, 60.0, 120.0, 180.0, 240.0, 300.0]);
    }

    /// The UV view stands still: the checker slides with the Offset as on the model, and the filled dot (the window's middle, carried) slides with it, the way a drag moves them
    #[test]
    fn the_uv_view_stands_still_and_the_dot_slides_with_the_checker() {
        let placement = |offset: [f32; 2]| Placement { tiling: [2.0, 2.0], rotation: 30.0, offset };
        let (still, rest) = slid_window(placement([0.0, 0.0]));

        assert_eq!(rest, [0.5, 0.5], "at Offset 0 the dot sits in the ring");

        let slid = placement([-0.25, 0.0]);
        let (columns, dot) = slid_window(slid);
        assert_eq!(columns, still, "the view never zooms or turns for the Offset");

        // A pad drag right lowers the Offset: the dot goes right, an eighth of the window along the turned U
        let expected = [0.5 + 0.125 * 30f32.to_radians().cos(), 0.5 - 0.125 * 30f32.to_radians().sin()];
        assert!((dot[0] - expected[0]).abs() < 1e-5 && (dot[1] - expected[1]).abs() < 1e-5, "{dot:?} vs {expected:?}");

        // The checker point under the middle at Offset 0 shows under the dot now
        let [[a, b], [c, d]] = columns;
        let texture_at = |[u, v]: [f32; 2], offset: [f32; 2]| [a * u + c * v + offset[0], b * u + d * v + offset[1]];
        let (before, after) = (texture_at([0.5, 0.5], [0.0, 0.0]), texture_at(dot, slid.offset));
        assert!((before[0] - after[0]).abs() < 1e-5 && (before[1] - after[1]).abs() < 1e-5, "{before:?} vs {after:?}");
    }
}
