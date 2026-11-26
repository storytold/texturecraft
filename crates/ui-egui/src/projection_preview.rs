//! The preview square of a 3D projection: a checker laid on a sphere, cylinder or cube through the fill's own projection
//!
//! Each pixel casts a ray at the shape and samples the checker with the compositor's projection math
//! ([`image_samples`] and [`projector_cover`]), so tiling, offset, rotation, Hardness, Angle, the 3D projection
//! settings, Shape crop and Backface culling show as they will on the model. Depth culling needs the model's depth
//! map and is not previewed. The projector is drawn round the shape as the 3D view's gizmo draws it round the model, and the
//! view zooms out until both fit, down to half the shape's size; a projector bigger than that runs off the square, marked on
//! each side it passes

use egui::{Color32, ColorImage, Painter, Pos2, Rect, Shape, Stroke, TextureOptions, pos2, vec2};
use texturecraft_engine::compose::frame_placement::{folded, upright_turn};
use texturecraft_engine::compose::pattern::{Coords, decal_axes, image_samples, place_frame_uv};
use texturecraft_engine::compose::projector_cover::{placed_cover, projector_cover};
use texturecraft_engine::core::{Projection, Projector};
use texturecraft_engine::geom::glam::{Vec2, Vec3, Vec4};

use rayon::prelude::*;

use crate::projector_outline::ProjectorOutline;
use crate::slot_thumbnail::FULL_UV;
use crate::theme::Tokens;

mod preview_shape;
mod rendered_preview;
mod screen;
mod view;

use preview_shape::PreviewShape;
use rendered_preview::RenderedPreview;
use screen::Screen;
use view::View;

/// Largest preview side in pixels; the square is never drawn bigger than this
const MAX_SIDE_PX: usize = 256;

/// Rays per pixel along each axis (smooth outline)
const SUPERSAMPLE: usize = 2;

/// Rays per pixel along each axis while a drag is held: a quarter of the rays, so the square keeps up with the pointer; the full picture follows on release
const DRAG_SUPERSAMPLE: usize = 1;

/// Checker cells across one repeat of the source along each axis, in this square and in a warp's frame alike (an untiled, unturned image shows 4 × 4 cells), so a fill looks the same in both
pub(crate) const CHECKER_CELLS: f32 = 4.0;

/// Half the view's width and height, in mesh radii: as near as the view comes
const VIEW_HALF: f32 = 1.0;

/// Half the view's width and height as far as it zooms out, in mesh radii (the shape at half its size)
const MAX_VIEW_HALF: f32 = 2.0;

/// Room left round the projector when the view zooms out to it
const FIT_MARGIN: f32 = 1.12;

/// How far in front of the middle the rays start, in mesh radii (past every shape and every outline point tested for cover)
const CAMERA_DISTANCE: f32 = 3.0;

/// Screen length of the pieces an outline is split into to find its hidden parts (points)
const OUTLINE_STEP: f32 = 3.0;

/// The most pieces one outline segment is split into, however long it runs on screen
const MAX_OUTLINE_PIECES: f32 = 256.0;

/// The outline's dashes where the shape hides it (points)
const HIDDEN_DASH: f32 = 3.0;

/// The edge marker's dashes (points)
const EDGE_DASH: f32 = 6.0;

/// The edge marker's inset from the square's side (points)
const EDGE_INSET: f32 = 2.0;

/// The edge marker's chevron arm (points)
const CHEVRON_ARM: f32 = 5.0;

/// The clear margin round the chevron, so the dashes and the shape stop short of it (points)
const CHEVRON_PATCH_MARGIN: f32 = 1.5;

/// The dot at a planar's direction tip (points)
const TIP_DOT: f32 = 2.2;

/// How far the field-coloured rim under the tip dot reaches past it (points)
const TIP_DOT_RIM: f32 = 0.8;

/// The width of the field-coloured line under the outline, which keeps it readable on the checker (points)
const OUTLINE_UNDER_WIDTH: f32 = 2.5;

/// The width of the outline itself (points)
const OUTLINE_WIDTH: f32 = 1.2;

/// How strong the line under the outline and the outline show where the camera sees them
const SEEN_SHADES: (f32, f32) = (0.55, 1.0);

/// How strong the line under the outline and the outline show where the shape hides them: fainter
const HIDDEN_SHADES: (f32, f32) = (0.35, 0.65);

/// Points a point may lie past a side of the square and still count as inside it (a point on the side's pixel rounds either way)
const PAST_SIDE_SLACK: f32 = 0.5;

/// Camera turn around the up axis (degrees): three faces of the cube and the cylinder's top show
const CAMERA_YAW: f32 = 35.0;

/// Camera turn down from level (degrees)
const CAMERA_PITCH: f32 = 25.0;

/// Where the light comes from, along the view's right, up and towards the eye: above and to the left of the camera
const LIGHT_FROM: [f32; 3] = [-0.5, 0.6, 0.6];

/// Light that reaches faces turned away from the light
const AMBIENT: f32 = 0.3;

/// The sphere's radius, in mesh radii, inside the unit sphere the compositor's coordinates span
const SPHERE_RADIUS: f32 = 0.85;

/// The cylinder's radius, in mesh radii
const CYLINDER_RADIUS: f32 = 0.55;

/// Half the cylinder's height, in mesh radii
const CYLINDER_HALF_HEIGHT: f32 = 0.7;

/// Half the cube's side, in mesh radii
const CUBE_HALF: f32 = 0.55;

/// The checker's dark cells, as a fraction of the way from `field` to `text`
const CHECKER_DARK: f32 = 0.12;

/// The checker's light cells, as a fraction of the way from `field` to `text`
const CHECKER_LIGHT: f32 = 1.0;

/// The shape where the projection does not reach, as a fraction of the way from `field` to `text`
const UNCOVERED: f32 = 0.45;

/// How much of the projection reaches a ray past which it counts as covered, for the edge of its reach
const HALF_COVERED: f32 = 0.5;

/// A missed ray has no coverage value in the 0..1 range
const MISSED_RAY_COVERAGE: f32 = -1.0;

/// How far a ray on the edge of the projection's reach is drawn from the accent towards the shape's colour
const EDGE_BLEND: f32 = 0.5;

/// The smallest model radius and frame half size the preview divides by, in mesh units and mesh radii
const MIN_SIZE: f32 = 1e-4;

/// A ray this close to parallel to a face or the cylinder's axis never meets it
const PARALLEL_TOLERANCE: f32 = 1e-6;

type PreviewTextures = egui::cache::FramePublisher<egui::Id, RenderedPreview>;

/// Draw the preview of `projection` on a mesh within `bounds` into `rect`, rendered once per projection, size and theme, with a quarter of the rays while a drag is held (None for projections without one: UV, warp); each `owner` (the docked panel's square, the popup's) keeps its own picture
pub(crate) fn projection_preview(painter: &Painter, rect: Rect, projection: &Projection, bounds: (Vec3, Vec3), tokens: &Tokens, owner: egui::Id) -> Option<()> {
    let shown = shown_projection(projection, bounds)?;
    let ctx = painter.ctx();
    let side = ((rect.width() * ctx.pixels_per_point()).round() as usize).clamp(1, MAX_SIDE_PX);
    let shape = shape_for(&shown);
    let outline = ProjectorOutline::of(&shown, shape.half_size());
    let (resting, half) = resting_fit(&shown, shape);
    let key = format!("{side}{half}{shown:?}{:?}{:?}{:?}", tokens.field, tokens.text, tokens.accent);
    let held = ctx.input(|input| input.pointer.any_down());
    #[cfg(test)]
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique(LAST_OWNER), owner));

    // Upgrade a drag's lower-resolution sampling when the pointer is released
    let cached = cached_preview(ctx, owner);
    let up_to_date = cached.as_ref().is_some_and(|cached| cached.key == key && (held || cached.supersample == SUPERSAMPLE));
    let rendered = match cached {
        Some(cached) if up_to_date => cached,
        cached => {
            let supersample = if held { DRAG_SUPERSAMPLE } else { SUPERSAMPLE };
            let image = render(&shown, side, half, supersample, tokens);
            let texture = match cached {
                Some(RenderedPreview { mut texture, .. }) => {
                    texture.set(image, TextureOptions::LINEAR);
                    texture
                }
                None => ctx.load_texture("projection-preview", image, TextureOptions::LINEAR),
            };
            RenderedPreview { key, supersample, texture }
        }
    };
    painter.image(rendered.texture.id(), rect, FULL_UV, Color32::WHITE);
    ctx.memory_mut(|memory| memory.caches.cache::<PreviewTextures>().set(owner, rendered));

    if let Some(outline) = &outline {
        let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
        let screen = Screen { view: View::new(), rect, half };
        draw_outline(&painter, &screen, shape, outline, tokens);
        let center = resting.as_ref().map_or(rect.center(), |resting| screen.at(resting.center));
        crate::preview_dots::preview_dots(&painter, center, screen.at(outline.center), tokens);
        edge_markers(&painter, rect, passed_sides(outline.points().chain([outline.center]).map(|point| screen.at(point)), rect), tokens);
    }
    painter.rect_stroke(rect, 0, egui::Stroke::new(1.0, tokens.field_border), egui::StrokeKind::Inside);
    Some(())
}

fn cached_preview(ctx: &egui::Context, owner: egui::Id) -> Option<RenderedPreview> {
    ctx.memory_mut(|memory| memory.caches.cache::<PreviewTextures>().get(&owner).cloned())
}

/// Where tests find the owner of the square drawn last
#[cfg(test)]
const LAST_OWNER: &str = "projection_preview_last_owner";

/// The picture of the square drawn last (tests)
#[cfg(test)]
fn last_rendered(ctx: &egui::Context) -> Option<RenderedPreview> {
    let owner = ctx.data(|data| data.get_temp::<egui::Id>(egui::Id::unique(LAST_OWNER)))?;
    cached_preview(ctx, owner)
}

/// What the preview square last rendered (tests: a drag re-renders it)
#[cfg(test)]
pub(crate) fn shown_key(ctx: &egui::Context) -> Option<String> {
    last_rendered(ctx).map(|rendered| rendered.key)
}

/// How the preview square was last rendered, with rays per pixel (tests: a drag renders it lighter, the release in full)
#[cfg(test)]
pub(crate) fn shown_supersample(ctx: &egui::Context) -> Option<usize> {
    last_rendered(ctx).map(|rendered| rendered.supersample)
}

/// Map a placed planar to the front of the preview sphere while preserving its crop and image mapping.
fn shown_projection(projection: &Projection, bounds: (Vec3, Vec3)) -> Option<Projection> {
    match projection {
        Projection::Planar { placed: Some(_), uv, .. } => {
            let radius = ((bounds.1 - bounds.0) * 0.5).max_element().max(MIN_SIZE);
            let Some(landed) = folded(projection) else {
                let mut shown = projection.clone();
                let Projection::Planar { placed: Some(frame), .. } = &mut shown else { return None };
                let world_to_preview = SPHERE_RADIUS / radius;

                // Keep the original projector transform when a fold would change culling
                frame.center = ((Vec3::from(frame.center) - (bounds.0 + bounds.1) * 0.5) * world_to_preview).to_array();
                frame.size = frame.size.map(|side| side * world_to_preview);
                frame.offset = frame.offset.map(|offset| offset * world_to_preview);
                frame.depth *= world_to_preview;
                decal_axes(frame)?;
                return Some(shown);
            };

            let Projection::Planar { placed: Some(frame), projector, .. } = &landed else { return None };

            // Normalize frame lengths by the model's largest half size
            let frame_half = frame.size.map(|side| (SPHERE_RADIUS * side / (2.0 * radius)).max(MIN_SIZE));
            let turn = upright_turn(frame).unwrap_or(0.0);

            let (origin, across, up, _) = decal_axes(frame)?;
            let from_center = (origin - (bounds.0 + bounds.1) * 0.5) / radius * SPHERE_RADIUS;
            let offset = [-from_center.dot(across) / frame_half[0], -from_center.dot(up) / frame_half[1], 0.0];

            // A single reflection reverses the turn around the image centre
            let [flip_u, flip_v] = frame.flip.map(|flip| if flip { -1.0 } else { 1.0 });
            let image_origin = Vec2::new(f32::from(u8::from(frame.flip[0])), f32::from(u8::from(frame.flip[1])));
            let uv = texturecraft_engine::core::UvTransform {
                tiling: [uv.tiling[0] * flip_u, uv.tiling[1] * flip_v],
                rotation: uv.rotation * flip_u * flip_v,
                offset: place_frame_uv(image_origin, uv).to_array(),
            };

            let projector = Projector { rotation: [0.0, 0.0, turn], scale: [frame_half[0], frame_half[1], 1.0], ..*projector };
            offset.iter().all(|value| value.is_finite()).then_some(Projection::Planar { scale: 1.0, axis: 2, offset, uv, projector, placed: None })
        }
        Projection::Planar { .. } | Projection::Spherical { .. } | Projection::Cylindrical { .. } | Projection::Triplanar { .. } => Some(projection.clone()),
        Projection::Warp { .. } | Projection::Uv { .. } | Projection::UvTile => None,
    }
}

/// The projector `shown` round `shape` back at Offset 0, and how far the view reaches to fit it there: sliding the projector never zooms
fn resting_fit(shown: &Projection, shape: PreviewShape) -> (Option<ProjectorOutline>, f32) {
    let resting = ProjectorOutline::of(&at_rest(shown), shape.half_size());
    let half = view_half(resting.as_ref());
    (resting, half)
}

/// `projection` with its projector back at Offset 0 (its turn and size kept)
fn at_rest(projection: &Projection) -> Projection {
    let mut resting = projection.clone();
    match &mut resting {
        Projection::Planar { offset, .. }
        | Projection::Triplanar { offset, .. }
        | Projection::Spherical { offset, .. }
        | Projection::Cylindrical { offset, .. } => {
            *offset = [0.0; 3];
        }
        Projection::Warp { .. } | Projection::Uv { .. } | Projection::UvTile => {}
    }
    resting
}

/// Which way the preview camera looks
pub(crate) fn view_direction() -> Vec3 {
    View::new().forward
}

/// How far the view reaches each way from the middle so the shape and the projector `outline` both fit: never nearer than today's framing, never so far the shape drops below half its size
fn view_half(outline: Option<&ProjectorOutline>) -> f32 {
    let view = View::new();
    let reach =
        outline.into_iter().flat_map(ProjectorOutline::points).map(|point| point.dot(view.right).abs().max(point.dot(view.up).abs())).fold(0.0, f32::max);
    if reach.is_finite() { (reach * FIT_MARGIN).clamp(VIEW_HALF, MAX_VIEW_HALF) } else { MAX_VIEW_HALF }
}

/// The outline's line in the text colour and the field-coloured line under it that keeps it readable on the checker, as the 3D view's gizmo draws its white outline on a dark shadow (the square's field is light in the light themes); fainter where the shape hides it
fn outline_strokes(hidden: bool, tokens: &Tokens) -> [Stroke; 2] {
    let (under, over) = if hidden { HIDDEN_SHADES } else { SEEN_SHADES };
    [Stroke::new(OUTLINE_UNDER_WIDTH, tokens.field.gamma_multiply(under)), Stroke::new(OUTLINE_WIDTH, tokens.text.gamma_multiply(over))]
}

/// The projector outline over the preview: solid where the camera sees it, dashed where the shape hides it, a dot where a planar's image travels to
fn draw_outline(painter: &Painter, screen: &Screen, shape: PreviewShape, outline: &ProjectorOutline, tokens: &Tokens) {
    for line in &outline.lines {
        // Runs of the line that are all seen or all hidden
        let mut runs: Vec<(bool, Vec<Pos2>)> = Vec::new();
        for pair in line.windows(2) {
            let [from, to] = [pair[0], pair[1]];
            let pieces = ((screen.at(to) - screen.at(from)).length() / OUTLINE_STEP).ceil().clamp(1.0, MAX_OUTLINE_PIECES) as usize;
            for piece in 0..pieces {
                let (start, end) = (from.lerp(to, piece as f32 / pieces as f32), from.lerp(to, (piece + 1) as f32 / pieces as f32));
                let hidden = screen.hidden(shape, start.lerp(end, 0.5));
                match runs.last_mut() {
                    Some((run_hidden, points)) if *run_hidden == hidden => points.push(screen.at(end)),
                    _ => runs.push((hidden, vec![screen.at(start), screen.at(end)])),
                }
            }
        }
        for (hidden, points) in runs {
            let strokes = outline_strokes(hidden, tokens);
            if hidden {
                crate::viewport::paint_dashed_outline(painter, &points, HIDDEN_DASH, HIDDEN_DASH, strokes);
            } else {
                let [under, over] = strokes;
                painter.add(Shape::line(points.clone(), under));
                painter.add(Shape::line(points, over));
            }
        }
    }
    if let Some(tip) = outline.tip {
        let [under, over] = outline_strokes(false, tokens);
        painter.circle_filled(screen.at(tip), TIP_DOT + TIP_DOT_RIM, under.color);
        painter.circle_filled(screen.at(tip), TIP_DOT, over.color);
    }
}

/// The sides of `rect` some of `points` lie past: left, right, top, bottom
pub(crate) fn passed_sides(points: impl Iterator<Item = Pos2>, rect: Rect) -> [bool; 4] {
    let outer = rect.expand(PAST_SIDE_SLACK);
    points.fold([false; 4], |[left, right, top, bottom], point| {
        [left || point.x < outer.left(), right || point.x > outer.right(), top || point.y < outer.top(), bottom || point.y > outer.bottom()]
    })
}

/// A dashed line just inside each side of `rect` the projector runs past, with a chevron pointing out at its middle
pub(crate) fn edge_markers(painter: &Painter, rect: Rect, sides: [bool; 4], tokens: &Tokens) {
    let inner = rect.shrink(EDGE_INSET);
    let center = rect.center();

    // Per side: its two ends, the chevron's point and the way into the square
    let edges = [
        ([inner.left_top(), inner.left_bottom()], pos2(inner.left(), center.y), vec2(1.0, 0.0)),
        ([inner.right_top(), inner.right_bottom()], pos2(inner.right(), center.y), vec2(-1.0, 0.0)),
        ([inner.left_top(), inner.right_top()], pos2(center.x, inner.top()), vec2(0.0, 1.0)),
        ([inner.left_bottom(), inner.right_bottom()], pos2(center.x, inner.bottom()), vec2(0.0, -1.0)),
    ];
    for (passed, (ends, point, inward)) in sides.into_iter().zip(edges) {
        if !passed {
            continue;
        }

        let along = vec2(inward.y, inward.x);
        let chevron = vec![point + (inward - along) * CHEVRON_ARM, point, point + (inward + along) * CHEVRON_ARM];
        for stroke in outline_strokes(false, tokens) {
            painter.extend(Shape::dashed_line(&ends, stroke, EDGE_DASH, EDGE_DASH));
        }

        // The chevron on a clear patch, so the dashes and the shape do not run through it
        let patch = Rect::from_points(&chevron).expand(CHEVRON_PATCH_MARGIN);
        painter.rect_filled(patch, 0, tokens.field);
        for stroke in outline_strokes(false, tokens) {
            painter.add(Shape::line(chevron.clone(), stroke));
        }
    }
}

/// The shape the preview shows `projection` on: a cylinder for a cylindrical, a cube for a tri-planar, else a sphere
fn shape_for(projection: &Projection) -> PreviewShape {
    match projection {
        Projection::Cylindrical { .. } => PreviewShape::Cylinder,
        Projection::Triplanar { .. } => PreviewShape::Cube,
        _ => PreviewShape::Sphere,
    }
}

/// Output rows per batch to bound ray storage while leaving enough rays for parallel work
const PREVIEW_BATCH_ROWS: usize = 64;

/// `side²` pixels of the checker on the projection's shape over the `field` background, the view `half` mesh radii across each way from the middle, `supersample²` rays per pixel cast on every core
fn render(projection: &Projection, side: usize, half: f32, supersample: usize, tokens: &Tokens) -> ColorImage {
    let shape = shape_for(projection);
    let View { eye, forward, right, up } = View::new();
    let [light_right, light_up, light_eye] = LIGHT_FROM;
    let light = (light_right * right + light_up * up + light_eye * eye).normalize_or_zero();

    let field = tokens.field;
    let (background, text) = (linear(field), linear(tokens.text));
    let shades = [CHECKER_DARK, CHECKER_LIGHT, UNCOVERED].map(|t| background.lerp(text, t));
    let supersample = supersample.max(1);
    let rays = side * supersample;

    let cast_ray = |ray: usize| {
        let (x, y) = (ray % rays, ray / rays);
        let across = ((x as f32 + 0.5) / rays as f32 * 2.0 - 1.0) * half;
        let down = ((y as f32 + 0.5) / rays as f32 * 2.0 - 1.0) * half;
        let origin = right * across - up * down + eye * CAMERA_DISTANCE;
        let Some(hit) = intersect(shape, origin, forward) else { return Vec4::new(0.0, 0.0, 0.0, MISSED_RAY_COVERAGE) };
        let shade = AMBIENT + (1.0 - AMBIENT) * hit.nrm.dot(light).max(0.0);
        let (color, cover) = surface(projection, &hit, shades);
        (color * shade).extend(cover)
    };

    let edge = linear(tokens.accent);
    let mut image = ColorImage::new([side, side], vec![field; side * side]);
    let batch_ray_rows = (PREVIEW_BATCH_ROWS * supersample + 2).min(rays);
    let mut samples = Vec::with_capacity(batch_ray_rows * rays);
    for (batch, pixels) in image.pixels.chunks_mut((side * PREVIEW_BATCH_ROWS).max(1)).enumerate() {
        let first_pixel_row = batch * PREVIEW_BATCH_ROWS;

        // Include the adjacent ray on both sides so coverage edges agree across batches
        let first_ray_row = (first_pixel_row * supersample).saturating_sub(1);
        let past_ray_row = ((first_pixel_row + pixels.len() / side) * supersample + 1).min(rays);
        (first_ray_row * rays..past_ray_row * rays).into_par_iter().map(cast_ray).collect_into_vec(&mut samples);

        pixels.par_iter_mut().enumerate().for_each(|(pixel, out)| {
            let (x, y) = (pixel % side, first_pixel_row + pixel / side);
            let mut sum = Vec3::ZERO;
            for sub in 0..supersample * supersample {
                let (rx, ry) = (x * supersample + sub % supersample, y * supersample + sub / supersample - first_ray_row);
                let sample = samples[ry * rays + rx];
                let color = if sample.w == MISSED_RAY_COVERAGE {
                    background
                } else if on_edge(&samples, rays, rx, ry, sample.w) {
                    edge.lerp(sample.truncate(), EDGE_BLEND)
                } else {
                    sample.truncate()
                };
                sum += color;
            }
            *out = from_linear(sum / (supersample * supersample) as f32);
        });
    }
    image
}

/// Where the projection's reach ends on the shape: covered here, not on a neighbouring ray of the shape
fn on_edge(samples: &[Vec4], rays: usize, x: usize, y: usize, cover: f32) -> bool {
    if cover <= HALF_COVERED {
        return false;
    }

    let neighbours = [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)];
    neighbours
        .into_iter()
        .any(|(nx, ny)| nx < rays && ny < rays && samples.get(ny * rays + nx).is_some_and(|sample| sample.w != MISSED_RAY_COVERAGE && sample.w <= HALF_COVERED))
}

/// The unlit colour at a hit and how much of the projection reaches it, from the dark and light checker and the uncovered shape `shades` in linear light
fn surface(projection: &Projection, hit: &Coords, shades: [Vec3; 3]) -> (Vec3, f32) {
    let mut coords = Coords { uv: hit.uv, pos: hit.pos, nrm: hit.nrm };
    let cover = match projection {
        Projection::Planar { placed: Some(frame), projector, .. } => {
            let Some((uv, cover)) = placed_cover(frame, projector, hit.pos, hit.nrm) else { return (shades[2], 0.0) };
            coords.uv = uv;
            cover
        }
        _ => projector_cover(projection, hit),
    }
    .clamp(0.0, 1.0);

    let mut light = 0.0;
    for (uv, weight) in image_samples(projection, &coords) {
        let cell = (uv * CHECKER_CELLS).floor();
        let even = (cell.x + cell.y).rem_euclid(2.0) < 0.5;
        light += weight * f32::from(u8::from(even));
    }
    let [dark, bright, uncovered] = shades;
    (uncovered.lerp(dark.lerp(bright, light.clamp(0.0, 1.0)), cover), cover)
}

/// The nearest hit of the ray from `origin` along unit `dir` on the preview shape
fn intersect(shape: PreviewShape, origin: Vec3, dir: Vec3) -> Option<Coords> {
    match shape {
        PreviewShape::Sphere => intersect_sphere(origin, dir),
        PreviewShape::Cube => intersect_cube(origin, dir),
        PreviewShape::Cylinder => intersect_cylinder(origin, dir),
    }
}

/// The nearer hit of the ray from `ray_origin` along unit `ray_dir` on the sphere round the middle
fn intersect_sphere(ray_origin: Vec3, ray_dir: Vec3) -> Option<Coords> {
    // |origin + t·dir|² = r², with |dir| = 1: t² + 2·half_b·t + c = 0
    let half_b = ray_origin.dot(ray_dir);
    let c = ray_origin.length_squared() - SPHERE_RADIUS * SPHERE_RADIUS;
    let discriminant = half_b * half_b - c;
    if discriminant < 0.0 {
        return None;
    }

    let point = ray_origin + ray_dir * (-half_b - discriminant.sqrt());
    Some(hit(point, point.normalize_or_zero()))
}

/// Where the ray from `ray_origin` along `ray_dir` enters the cube round the middle: the latest of the three slabs' entries, before the earliest exit
fn intersect_cube(ray_origin: Vec3, ray_dir: Vec3) -> Option<Coords> {
    let (mut near, mut far, mut nrm) = (f32::NEG_INFINITY, f32::INFINITY, Vec3::ZERO);
    for axis in 0..3 {
        let (start, along) = (ray_origin[axis], ray_dir[axis]);
        if along.abs() < PARALLEL_TOLERANCE {
            if start.abs() > CUBE_HALF {
                return None;
            }

            continue;
        }

        let (low_face, high_face) = ((-CUBE_HALF - start) / along, (CUBE_HALF - start) / along);
        let (enter, exit) = (low_face.min(high_face), low_face.max(high_face));
        if enter > near {
            near = enter;
            nrm = Vec3::ZERO;
            nrm[axis] = -along.signum();
        }
        far = far.min(exit);
    }
    (near <= far && near.is_finite()).then(|| hit(ray_origin + ray_dir * near, nrm))
}

/// The nearest hit in front of `ray_origin` along `ray_dir` on the upright cylinder round the middle: its side or one of its caps
fn intersect_cylinder(ray_origin: Vec3, ray_dir: Vec3) -> Option<Coords> {
    let mut best: Option<(f32, Coords)> = None;
    let mut keep = |distance: f32, hit: Coords| {
        if distance > 0.0 && best.as_ref().is_none_or(|(best_distance, _)| distance < *best_distance) {
            best = Some((distance, hit));
        }
    };

    // The side: x² + z² = r² within the height, a·t² + 2·half_b·t + c = 0
    let a = ray_dir.x * ray_dir.x + ray_dir.z * ray_dir.z;
    let half_b = ray_origin.x * ray_dir.x + ray_origin.z * ray_dir.z;
    let c = ray_origin.x * ray_origin.x + ray_origin.z * ray_origin.z - CYLINDER_RADIUS * CYLINDER_RADIUS;
    let discriminant = half_b * half_b - a * c;
    if a > PARALLEL_TOLERANCE && discriminant >= 0.0 {
        let distance = (-half_b - discriminant.sqrt()) / a;
        let point = ray_origin + ray_dir * distance;
        if point.y.abs() <= CYLINDER_HALF_HEIGHT {
            keep(distance, hit(point, Vec3::new(point.x, 0.0, point.z).normalize_or_zero()));
        }
    }

    // The caps
    if ray_dir.y.abs() > PARALLEL_TOLERANCE {
        for sign in [1.0, -1.0] {
            let distance = (sign * CYLINDER_HALF_HEIGHT - ray_origin.y) / ray_dir.y;
            let point = ray_origin + ray_dir * distance;
            if point.x * point.x + point.z * point.z <= CYLINDER_RADIUS * CYLINDER_RADIUS {
                keep(distance, hit(point, Vec3::Y * sign));
            }
        }
    }
    best.map(|(_, hit)| hit)
}

/// A shape point the camera sees, in the compositor's coordinates (no texture coordinates)
fn hit(pos: Vec3, nrm: Vec3) -> Coords {
    Coords { uv: Vec2::ZERO, pos, nrm }
}

/// `color` in linear light
fn linear(color: Color32) -> Vec3 {
    let rgba = egui::Rgba::from(color);
    Vec3::new(rgba.r(), rgba.g(), rgba.b())
}

/// A linear-light `color` back as a display colour
fn from_linear(color: Vec3) -> Color32 {
    egui::Rgba::from_rgb(color.x, color.y, color.z).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_engine::core::{Projector, ShapeCrop, UvTransform};

    fn tokens() -> Tokens {
        Tokens::for_brightness(crate::theme::Brightness::MediumDark)
    }

    fn spherical(tiling: f32) -> Projection {
        Projection::Spherical {
            scale: 1.0,
            uv: UvTransform { tiling: [tiling; 2], ..UvTransform::default() },
            offset: [0.0; 3],
            projector: Projector::default(),
        }
    }

    fn cylindrical(projector: Projector) -> Projection {
        Projection::Cylindrical { scale: 1.0, axis: 1, uv: UvTransform::default(), offset: [0.0; 3], projector, angle: 360.0 }
    }

    #[test]
    fn the_shape_fills_the_middle_and_leaves_the_corners_to_the_background() {
        let tokens = tokens();
        for projection in [spherical(1.0), cylindrical(Projector::default()), Projection::triplanar()] {
            let image = render(&projection, 32, VIEW_HALF, SUPERSAMPLE, &tokens);

            assert_eq!(image.pixels[0], tokens.field, "{projection:?}: the corner is background");
            assert_ne!(image.pixels[16 * 32 + 16], tokens.field, "{projection:?}: the middle shows the shape");
        }
    }

    #[test]
    fn the_preview_follows_the_fill_settings() {
        let tokens = tokens();

        assert_ne!(
            render(&spherical(1.0), 32, VIEW_HALF, SUPERSAMPLE, &tokens).pixels,
            render(&spherical(3.0), 32, VIEW_HALF, SUPERSAMPLE, &tokens).pixels,
            "tiling changes the checker"
        );

        let culled =
            Projector { backface: texturecraft_engine::core::Backface { on: true, ..Default::default() }, crop: ShapeCrop::Cropped, ..Projector::default() };
        assert_ne!(
            render(&cylindrical(Projector::default()), 32, VIEW_HALF, SUPERSAMPLE, &tokens).pixels,
            render(&cylindrical(culled), 32, VIEW_HALF, SUPERSAMPLE, &tokens).pixels,
            "culling shows on the cylinder"
        );
    }

    #[test]
    fn projections_without_a_shape_have_no_preview() {
        assert!(shown_projection(&Projection::UvTile, BOUNDS).is_none());
        assert!(shown_projection(&Projection::Uv { scale: [1.0; 2], rotation: 0.0, offset: [0.0; 2] }, BOUNDS).is_none());
        assert!(shown_projection(&spherical(1.0), BOUNDS).is_some());
    }

    const BOUNDS: (Vec3, Vec3) = (Vec3::NEG_ONE, Vec3::ONE);

    #[test]
    fn outline_occlusion_uses_the_same_camera_as_the_preview() {
        let view = View::new();
        let screen = Screen { view, rect: Rect::from_min_size(Pos2::ZERO, vec2(96.0, 96.0)), half: VIEW_HALF };
        let far = CAMERA_DISTANCE + CYLINDER_HALF_HEIGHT * 2.0;

        assert!(screen.hidden(PreviewShape::Cylinder, -view.eye * far));
        assert!(!screen.hidden(PreviewShape::Cylinder, view.eye * far));
        assert!(!screen.hidden(PreviewShape::Cylinder, -view.eye * far + view.right * (CYLINDER_RADIUS * 2.0)));
    }

    #[test]
    fn placed_preview_preserves_crop_and_the_frames_image_coordinates() {
        let frame = texturecraft_engine::core::Decal {
            center: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            size: [1.0; 2],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 1.0,
            symmetry: None,
            flip: [false; 2],
        };
        let uv = UvTransform { tiling: [1.7, 2.4], rotation: 33.0, offset: [0.1, -0.2] };
        let projector = Projector::default();
        let outside = Coords { pos: Vec3::new(SPHERE_RADIUS * 0.75, 0.0, 0.0), uv: Vec2::ZERO, nrm: Vec3::Z };

        for crop in [ShapeCrop::ExtendsOutside, ShapeCrop::Cropped] {
            for flip in [[false, false], [true, false], [false, true], [true, true]] {
                let frame = texturecraft_engine::core::Decal { flip, ..frame };
                let projector = Projector { crop, ..projector };
                let source = Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv, projector, placed: Some(frame) };
                let shown = shown_projection(&source, BOUNDS).unwrap();

                assert_eq!(projector_cover(&shown, &outside), f32::from(u8::from(crop == ShapeCrop::ExtendsOutside)));
                for point in [Vec3::ZERO, Vec3::new(0.2, 0.1, 0.0), Vec3::new(-0.15, 0.21, 0.0)] {
                    let (frame_uv, _) = texturecraft_engine::compose::projector_cover::placed_cover(&frame, &projector, point, Vec3::Z).unwrap();
                    let expected = image_samples(&source, &Coords { uv: frame_uv, pos: point, nrm: Vec3::Z })[0].0;
                    let actual = image_samples(&shown, &Coords { uv: Vec2::ZERO, pos: point * SPHERE_RADIUS, nrm: Vec3::Z })[0].0;

                    assert!(actual.distance(expected) < 1e-5, "{crop:?} {flip:?} at {point}: expected {expected}, got {actual}");
                }
            }
        }
    }

    #[test]
    fn an_uneven_placed_projector_keeps_its_backface_culling_in_the_preview() {
        let center = Vec3::new(2.0, 3.0, 4.0);
        let frame = texturecraft_engine::core::Decal {
            center: center.to_array(),
            normal: Vec3::Z.to_array(),
            up: Vec3::Y.to_array(),
            size: [1.0; 2],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 1.0,
            symmetry: None,
            flip: [false; 2],
        };
        let projector = Projector {
            scale: [2.0, 1.0, 1.0],
            backface: texturecraft_engine::core::Backface { on: true, angle: 90.0, hardness: 1.0 },
            crop: ShapeCrop::Cropped,
            ..Default::default()
        };
        let source = Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector, placed: Some(frame) };
        let bounds = (center - Vec3::splat(2.0), center + Vec3::splat(2.0));

        assert!(folded(&source).is_none(), "folding this projector would change how it culls");

        let shown = shown_projection(&source, bounds).expect("a valid placed projector has a 3D preview");
        let coverage = |pos, nrm| surface(&shown, &Coords { pos, nrm, uv: Vec2::ZERO }, [Vec3::ZERO, Vec3::ONE, Vec3::splat(0.5)]).1;

        assert!(coverage(Vec3::ZERO, Vec3::Z) > 0.0, "the image-facing side is covered");
        assert_eq!(coverage(Vec3::ZERO, Vec3::NEG_Z), 0.0, "the back is culled");
        assert_eq!(coverage(Vec3::Y * SPHERE_RADIUS, Vec3::Z), 0.0, "the original frame still crops the image");
    }

    fn shown(projection: &Projection) -> Vec<Color32> {
        let shown = shown_projection(projection, BOUNDS).unwrap();
        let (_, half) = resting_fit(&shown, shape_for(&shown));
        render(&shown, 32, half, SUPERSAMPLE, &tokens()).pixels
    }

    /// What the 3D view's gizmo changes shows here too: a sphere's or cylinder's size, turn and centre, a placed frame's size, stretch and turn
    #[test]
    fn the_preview_shows_the_projectors_size_turn_and_place() {
        let base = shown(&spherical(1.0));
        let bigger = Projector { scale: [1.15; 3], ..Projector::default() };
        let Projection::Spherical { uv, .. } = spherical(1.0) else { panic!("spherical() makes a spherical projection") };
        let sphere = |projector: Projector, offset: [f32; 3]| Projection::Spherical { scale: 1.0, uv, offset, projector };

        assert_ne!(shown(&sphere(bigger, [0.0; 3])), base, "a bigger sphere zooms the view out");
        assert_ne!(shown(&sphere(Projector { rotation: [0.0, 0.0, 40.0], ..Projector::default() }, [0.0; 3])), base, "a turned sphere turns its checker");
        assert_ne!(shown(&sphere(Projector::default(), [0.3, 0.0, 0.0])), base, "an offset moves it");
        assert_eq!(shown(&sphere(Projector::default(), [0.0; 3])), base, "unmoved: as before");
        assert_ne!(
            shown(&cylindrical(Projector { scale: [1.3, 1.0, 1.3], ..Projector::default() })),
            shown(&cylindrical(Projector::default())),
            "a wider cylinder"
        );

        let frame = texturecraft_engine::core::Decal {
            center: [0.0, 0.0, 1.0],
            normal: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            size: [1.0, 1.0],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 1.0,
            symmetry: None,
            flip: [false; 2],
        };
        let placed = |frame| Projection::decal(frame);
        let square = shown(&placed(frame));
        assert_ne!(shown(&placed(texturecraft_engine::core::Decal { size: [0.5, 0.5], ..frame })), square, "a smaller frame");
        assert_ne!(shown(&placed(texturecraft_engine::core::Decal { size: [1.5, 1.0], ..frame })), square, "a stretched frame");
        assert_ne!(shown(&placed(texturecraft_engine::core::Decal { rotation: 30.0, ..frame })), square, "a turned frame");
        assert_ne!(shown(&placed(texturecraft_engine::core::Decal { offset: [0.3, 0.0], ..frame })), square, "a slid frame");
    }

    fn fitted_half(projection: &Projection) -> f32 {
        let shown = shown_projection(projection, BOUNDS).unwrap();
        resting_fit(&shown, shape_for(&shown)).1
    }

    fn sized(scale: f32) -> Projection {
        let Projection::Spherical { uv, .. } = spherical(1.0) else { panic!("spherical() makes a spherical projection") };
        Projection::Spherical { scale: 1.0, uv, offset: [0.0; 3], projector: Projector { scale: [scale; 3], ..Projector::default() } }
    }

    /// The view zooms out until the projector fits, never nearer than the plain framing and never past half the shape's size; past that the projector runs off the square and each side it passes is marked
    #[test]
    fn the_view_fits_the_projector_down_to_half_the_shape() {
        assert_eq!(fitted_half(&sized(0.4)), VIEW_HALF, "a small projector keeps the plain framing");

        let default = fitted_half(&sized(1.0));
        assert!(default > VIEW_HALF && default < MAX_VIEW_HALF, "{default}");
        assert_eq!(fitted_half(&sized(3.0)), MAX_VIEW_HALF, "x3 stops at half the shape's size");

        let rect = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0));
        let sides = |scale: f32| {
            let shown = sized(scale);
            let outline = ProjectorOutline::of(&shown, PreviewShape::Sphere.half_size()).unwrap();
            let screen = Screen { view: View::new(), rect, half: fitted_half(&shown) };
            passed_sides(outline.points().map(|point| screen.at(point)), rect)
        };

        assert_eq!(sides(1.0), [false; 4], "a fitted projector stays inside the square");
        assert_eq!(sides(3.0), [true; 4], "an x3 sphere runs past every side");

        let triplanar = Projection::Triplanar {
            scale: 1.0,
            sharpness: Projection::TRIPLANAR_SHARPNESS,
            offset: [0.0; 3],
            uv: UvTransform::default(),
            projector: Projector { scale: [10.0; 3], ..Projector::default() },
        };
        assert_eq!(fitted_half(&triplanar), MAX_VIEW_HALF, "every kind fits the same way");
    }

    /// Sliding a projector never zooms the view: it fits the projector at Offset 0, the ring stays in the middle and the projector's centre slides away from it
    #[test]
    fn the_offset_slides_the_projector_without_zooming_the_view() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0));
        let Projection::Spherical { scale, uv, projector, .. } = sized(1.0) else { panic!("sized() makes a spherical projection") };
        for offset in [[0.0, 0.0, 0.0], [0.3, 0.0, 0.0], [1.5, 0.8, 0.0]] {
            let slid = Projection::Spherical { scale, uv, offset, projector };

            assert_eq!(fitted_half(&slid), fitted_half(&sized(1.0)), "Offset {offset:?} keeps the view");

            let (resting, half) = resting_fit(&slid, PreviewShape::Sphere);
            let screen = Screen { view: View::new(), rect, half };
            let ring = screen.at(resting.unwrap().center);
            let dot = screen.at(ProjectorOutline::of(&slid, PreviewShape::Sphere.half_size()).unwrap().center);

            assert!(ring.distance(rect.center()) < 1e-3, "the ring marks the middle: {ring:?}");
            assert_eq!(dot.distance(ring) > 1.0, offset != [0.0; 3], "the filled dot leaves the ring only when slid: {dot:?}");
        }
    }

    /// A huge spherical projector no longer fills the square with its own ball: the shape keeps half its size in the middle with background round it
    #[test]
    fn a_huge_sphere_keeps_the_shape_in_the_square() {
        let tokens = tokens();
        let image = render(&sized(3.0), 32, fitted_half(&sized(3.0)), SUPERSAMPLE, &tokens);

        assert_eq!(image.pixels[0], tokens.field, "the corner is background");
        assert_eq!(image.pixels[16 * 32 + 4], tokens.field, "the shape is half its size: its side is background");
        assert_ne!(image.pixels[16 * 32 + 16], tokens.field, "the shape is in the middle");
    }

    /// The outline is dashed where the shape hides it: the equator's back half runs behind the ball
    #[test]
    fn the_shape_hides_the_back_of_the_outline() {
        let screen = Screen { view: View::new(), rect: Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0)), half: 1.2 };
        let outline = ProjectorOutline::of(&sized(1.0), PreviewShape::Sphere.half_size()).unwrap();
        let hidden = |line: &Vec<Vec3>| line.iter().filter(|point| screen.hidden(PreviewShape::Sphere, **point)).count();

        assert_eq!(hidden(&outline.lines[0]), 0, "the rim is all seen");

        let equator = hidden(&outline.lines[1]);
        assert!(equator > 0 && equator < outline.lines[1].len(), "the equator is half hidden: {equator}");
    }

    /// Every kind at x0.4, x1, x2, x3 and x10 (projector scale), as the panel's square draws it
    #[test]
    #[ignore = "requires an offscreen GPU adapter and TC_UI_SNAPSHOTS"]
    fn preview_fit_snapshots() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        use crate::theme::Brightness;
        let Some(directory) = std::env::var_os("TC_UI_SNAPSHOTS").map(std::path::PathBuf::from) else { return };
        std::fs::create_dir_all(&directory).unwrap();
        const SIZES: [f32; 5] = [0.4, 1.0, 2.0, 3.0, 10.0];
        const SQUARE: f32 = 104.0;
        const MARGIN: f32 = 4.0;
        type Kind = (&'static str, fn(f32) -> Projection);
        let kinds: [Kind; 4] = [
            ("planar", |s| Projection::Planar {
                scale: 1.0,
                axis: 2,
                offset: [0.0; 3],
                uv: UvTransform::default(),
                projector: Projector { scale: [s, s, 1.0], ..Projector::default() },
                placed: None,
            }),
            ("spherical", sized),
            ("cylindrical", |s| cylindrical(Projector { scale: [s; 3], ..Projector::default() })),
            ("triplanar", |s| Projection::Triplanar {
                scale: 1.0,
                sharpness: Projection::TRIPLANAR_SHARPNESS,
                offset: [0.0; 3],
                uv: UvTransform::default(),
                projector: Projector { scale: [s; 3], ..Projector::default() },
            }),
        ];

        // Cover every size in the medium theme and two sizes in the other themes
        let themes = [(Brightness::MediumDark, "", &SIZES[..]), (Brightness::Dark, "-dark", &[1.0, 3.0][..]), (Brightness::Light, "-light", &[1.0, 3.0][..])];
        for ((brightness, theme, sizes), (name, make)) in themes.into_iter().flat_map(|theme| kinds.map(|kind| (theme, kind))) {
            for &scale in sizes {
                let projection = make(scale);
                let rect = Rect::from_min_size(pos2(MARGIN, MARGIN), vec2(SQUARE, SQUARE));
                let mut harness =
                    egui_kittest::Harness::builder().with_size(vec2(SQUARE + 2.0 * MARGIN, SQUARE + 2.0 * MARGIN)).with_pixels_per_point(2.0).wgpu().build_ui(
                        move |ui| {
                            let tokens = Tokens::for_brightness(brightness);
                            ui.painter().rect_filled(ui.ctx().content_rect(), 0, tokens.panel);
                            projection_preview(ui.painter(), rect, &projection, BOUNDS, &tokens, egui::Id::unique("square"));
                        },
                    );
                harness.run_steps(2);
                harness.render().unwrap().save(directory.join(format!("preview-fit-{name}-x{scale}{theme}.png"))).unwrap();
            }
        }

        // In the panel: a x3 sphere zoomed out to its floor, marked on every side
        let (mut harness, _) = crate::frame_gizmo::tests::fill_app_rendered(serde_json::json!({"projection": "spherical", "projectorScale": [3, 3, 3]}), true);
        harness.run_steps(4);
        harness.render().unwrap().save(directory.join("preview-fit-in-panel-spherical-x3.png")).unwrap();
    }

    /// One frame of `projection`'s preview square, with `events` as the frame's input
    fn preview_frame(ctx: &egui::Context, projection: &Projection, events: Vec<egui::Event>) {
        let tokens = tokens();
        let input = egui::RawInput { events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            projection_preview(ui.painter(), Rect::from_min_size(pos2(0.0, 0.0), vec2(64.0, 64.0)), projection, BOUNDS, &tokens, egui::Id::unique("square"));
        });

        // No renderer takes the uploads here
        output.textures_delta.clear();
    }

    #[test]
    fn a_drag_renders_the_square_lighter_and_the_release_renders_it_in_full() {
        let ctx = egui::Context::default();
        let at = pos2(10.0, 10.0);
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        preview_frame(&ctx, &spherical(1.0), Vec::new());

        assert_eq!(shown_supersample(&ctx), Some(SUPERSAMPLE), "at rest: in full");

        preview_frame(&ctx, &spherical(1.0), vec![egui::Event::PointerMoved(at), button(true)]);

        assert_eq!(shown_supersample(&ctx), Some(SUPERSAMPLE), "a press alone keeps the full picture: nothing changed");

        preview_frame(&ctx, &spherical(2.0), Vec::new());

        assert_eq!(shown_supersample(&ctx), Some(DRAG_SUPERSAMPLE), "a change while held renders lighter");

        let dragged = shown_key(&ctx);

        preview_frame(&ctx, &spherical(2.0), vec![button(false)]);

        assert_eq!(shown_supersample(&ctx), Some(SUPERSAMPLE), "let go: the same picture in full");
        assert_eq!(shown_key(&ctx), dragged);
    }

    /// Two squares of one projection at different sizes in one frame (the docked panel's and the popup's) keep their own pictures, so neither re-renders over the other
    #[test]
    fn two_squares_drawn_together_keep_their_own_pictures() {
        let ctx = egui::Context::default();
        let tokens = tokens();
        let (docked, popup) = (egui::Id::unique("docked"), egui::Id::unique("popup"));
        let rendered = |owner| cached_preview(&ctx, owner).expect("a picture");
        let mut shown = Vec::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                projection_preview(ui.painter(), Rect::from_min_size(pos2(0.0, 0.0), vec2(64.0, 64.0)), &spherical(1.0), BOUNDS, &tokens, docked);
                projection_preview(ui.painter(), Rect::from_min_size(pos2(100.0, 0.0), vec2(96.0, 96.0)), &spherical(1.0), BOUNDS, &tokens, popup);
            });
            output.textures_delta.clear();
            shown.push([docked, popup].map(|owner| rendered(owner).texture.id()));
        }

        let [docked_picture, popup_picture] = [rendered(docked), rendered(popup)];

        assert_ne!(docked_picture.texture.id(), popup_picture.texture.id(), "each square draws its own texture");
        assert!(docked_picture.key.starts_with("64") && popup_picture.key.starts_with("96"), "each picture is rendered at its own side");
        assert_eq!(shown[0], shown[1], "the next frame keeps both pictures");
    }

    #[test]
    fn hidden_and_replaced_projection_previews_release_their_textures() {
        let ctx = egui::Context::default();
        let tokens = tokens();
        let bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(64.0, 64.0));
        let popup = egui::Id::unique("popup");
        let active_previews = || ctx.tex_manager().read().allocated().filter(|(_, texture)| texture.name == "projection-preview").count();
        let mut counts = Vec::new();

        for index in 0..12 {
            let owner = egui::Id::unique(("selection", index));
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                projection_preview(ui.painter(), bounds, &spherical(1.0), BOUNDS, &tokens, owner);
                projection_preview(ui.painter(), bounds, &spherical(2.0), BOUNDS, &tokens, popup);
            });
            output.textures_delta.clear();
            counts.push(active_previews());
        }
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();

        assert!(counts.iter().all(|&count| count == 2), "views retained after each frame: {counts:?}");
        assert_eq!(active_previews(), 0);
    }

    /// Render times of the preview square (prints ms per render at its largest side; run with --ignored --nocapture)
    #[test]
    #[ignore]
    fn preview_render_timings() {
        let tokens = tokens();
        let kinds = [("sphere", spherical(2.0)), ("cylinder", cylindrical(Projector::default())), ("cube", Projection::triplanar())];
        for (name, projection) in kinds {
            let start = std::time::Instant::now();
            const RUNS: u32 = 10;
            for _ in 0..RUNS {
                std::hint::black_box(render(&projection, MAX_SIDE_PX, VIEW_HALF, SUPERSAMPLE, &tokens));
            }
            let full = start.elapsed().as_secs_f64() * 1000.0 / f64::from(RUNS);
            let start = std::time::Instant::now();
            for _ in 0..RUNS {
                std::hint::black_box(render(&projection, MAX_SIDE_PX, VIEW_HALF, DRAG_SUPERSAMPLE, &tokens));
            }
            let dragged = start.elapsed().as_secs_f64() * 1000.0 / f64::from(RUNS);
            println!("preview {name}: {full:.1} ms at rest, {dragged:.1} ms while dragged");
        }
    }
}
