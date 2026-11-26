//! The frame of the selected planar or warp projection in the 3D view: the centre square slides it, the axis arrows move it along its across, up and out directions, the ring turns it round its normal, a planar's corners scale it, the edge dots stretch one side, a warp's corners pull freely

use egui::{Color32, Pos2, Rect, Stroke, Ui, vec2};
use serde_json::{Value, json};
use texturecraft_engine::compose::axis_frame::{placed_from_axis, placing_moves_pattern};
use texturecraft_engine::compose::frame_placement::folded;
use texturecraft_engine::compose::pattern::{decal_axes, decal_corners};
use texturecraft_engine::core::{Decal, EffectKind, Fill, LayerId, LayerKind, Projection, Source};
use texturecraft_engine::geom::glam::Vec3;

use crate::TextureCraftApp;
use crate::projection_gizmo::{Gizmo, dragging};
use crate::theme::{AXIS_COLORS, SHAFT_HALO, SHAFT_HALO_EXTRA_WIDTH, SHAFT_WIDTH, Tokens};
use crate::viewport::{project, view_ray};

mod frame_grab;

pub(crate) use frame_grab::FrameGrab;

/// The selected fill's frame as the 3D view shows it
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameGizmo {
    pub layer: LayerId,
    /// The selected mask fill effect projecting from the frame, when it is in a mask
    pub effect: Option<LayerId>,
    /// The frame; an axis planar shows the frame it is placed as on its first drag
    pub decal: Decal,
    /// A warp's corners, across and up in frame sizes from its centre
    pub warp: Option<[[f32; 2]; 4]>,
    /// On screen: the corners (a warp's pulled ones), the edge dots and the centre
    pub corners: [Pos2; 4],
    pub sides: [Pos2; 4],
    pub center: Pos2,
    /// On screen, the tips of the across, up and out arrows (None when one points at the camera)
    pub arrows: [Option<Pos2>; 3],
    /// On screen, the turn ring round the centre in the frame's plane (None when it is edge on)
    pub ring: Option<[Pos2; TURN_RING_POINTS]>,
    /// In the world: the frame's centre, and its across, up and out directions
    origin: Vec3,
    axes: [Vec3; 3],
    /// Screen movement per world unit along each direction at the centre
    steps: [Option<egui::Vec2>; 3],
}

/// Screen length of the axis arrows, and their heads (points): long enough to grab each one apart from the others
pub(crate) const ARROW: f32 = 56.0;
const ARROW_HEAD: f32 = 14.0;
/// The centre square's side, the corner and edge dot radii, a warp corner's radius (points)
pub(crate) const CENTER_SQUARE: f32 = 16.0;
const CORNER_DOT: f32 = 6.5;
const SIDE_DOT: f32 = 5.5;
const WARP_DOT: f32 = 8.0;
/// How near a handle or arrow a press grabs it (points)
pub(crate) const GRAB: f32 = 11.0;
/// Screen radius of the turn rings (frames and round projectors alike): past the arrow heads, so a ring is grabbed apart from the arrows
pub(crate) const TURN_RING: f32 = 84.0;
/// Points on a frame's turn ring, the first repeated at the end to close it
pub(crate) const TURN_RING_POINTS: usize = 49;
/// The smallest share of its size a dragged side keeps
const MIN_SIDE_STRETCH: f32 = 0.02;
/// The least distance from the centre a resize drag measures from, so a press on the centre never divides by zero (points)
pub(crate) const MIN_HANDLE_REACH: f32 = 1.0;

/// An unplaced axis planar can acquire a frame only if placing it preserves its pattern.
fn placeable_projection<'a>(projection: &'a Projection, sources: impl Iterator<Item = &'a Source>) -> Option<&'a Projection> {
    let unplaced = matches!(projection, Projection::Planar { placed: None, .. });
    if unplaced && placing_moves_pattern(sources) {
        return None;
    }

    Some(projection)
}

impl FrameGizmo {
    /// The frame of the selected fill layer, or of the selected mask fill effect, with a planar or warp projection
    pub(crate) fn of(app: &TextureCraftApp, rect: Rect) -> Option<FrameGizmo> {
        use texturecraft_engine::StackTarget;

        if app.ui.mode != crate::state::Mode::Painting {
            return None;
        }

        let layers = &app.session.doc.project;
        let selected = app.session.selected_layer()?;
        let (layer, effect, projection) = match app.session.target {
            StackTarget::Content => {
                let layer = layers.content_layer(selected)?;
                let LayerKind::Fill(Fill { projection, sources, .. }) = &layer.kind else { return None };

                (layer.id, None, placeable_projection(projection, sources.values())?)
            }
            StackTarget::Effect(effect) => {
                // An instance can show a frame from its inherited mask
                let owner = layers.mask_owner(selected);
                let mask = layers.layer(owner).ok()?.mask.as_ref()?;
                let mask_effect = mask.effects.iter().find(|candidate| candidate.id == effect)?;
                let EffectKind::Fill { projection, source, .. } = &mask_effect.kind else { return None };

                (owner, Some(effect), placeable_projection(projection, std::iter::once(source))?)
            }
            StackTarget::Mask => return None,
        };

        let placed = placed_from_axis(projection, app.viewport_mesh_bounds.get(&app.session.mesh));

        // Match the panel by folding the projector transform into the frame first
        let landed = folded(placed.as_ref().unwrap_or(projection))?;
        let decal = *landed.frame()?;
        let warp = match projection {
            Projection::Warp { corners, .. } => Some(*corners),
            _ => None,
        };
        let (origin, across, up, out) = decal_axes(&decal)?;

        // Warp corners can sit away from the frame corners
        let at = |[x, y]: [f32; 2]| project(app, rect, origin + across * x * decal.size[0] + up * y * decal.size[1]);
        let corners = match warp {
            Some([top_left, top_right, bottom_right, bottom_left]) => [at(top_left)?, at(top_right)?, at(bottom_right)?, at(bottom_left)?],
            None => {
                let [top_left, top_right, bottom_right, bottom_left] = decal_corners(&decal)?.map(|corner| project(app, rect, corner));
                [top_left?, top_right?, bottom_right?, bottom_left?]
            }
        };

        let sides = [0, 1, 2, 3].map(|side| corners[side].lerp(corners[(side + 1) % 4], 0.5));
        let center = project(app, rect, origin)?;

        // Arrows use the same screen steps as the drag
        let reach = decal.size[0].max(decal.size[1]);
        let axes = [across, up, out];
        let steps = axes.map(|axis| Some((project(app, rect, origin + axis * reach)? - center) / reach.max(f32::MIN_POSITIVE)));
        let arrows = steps.map(|step| step.filter(|step| step.length() * reach > MIN_SCREEN_SPAN).map(|step| center + step.normalized() * ARROW));

        // The ring can still project when an arrow's endpoint is behind the camera
        let ring = if ring_open(app, rect, center, out) { turn_ring(app, rect, center, origin, across, up) } else { None };

        Some(FrameGizmo { layer, effect, decal, warp, corners, sides, center, arrows, ring, origin, axes, steps })
    }

    /// The frame's offset after sliding it across its plane by screen movement `delta` (None when the frame is edge-on)
    pub(crate) fn slid_on_screen(&self, delta: egui::Vec2) -> Option<[f32; 2]> {
        let [across, up] = solve(self.steps[0]?, self.steps[1]?, delta)?;
        Some(self.slid(across, up))
    }

    /// Where screen point `p` meets the frame's plane, across and up in frame sizes from its centre
    fn on_plane(&self, app: &TextureCraftApp, rect: Rect, p: Pos2) -> Option<[f32; 2]> {
        let ray = view_ray(app, rect, p);
        let [across, up, out] = self.axes;
        let facing = ray.dir.dot(out);
        if facing.abs() < EDGE_ON_FACING {
            return None;
        }

        let hit = ray.origin + ray.dir * ((self.origin - ray.origin).dot(out) / facing);
        let local = [(hit - self.origin).dot(across) / self.decal.size[0], (hit - self.origin).dot(up) / self.decal.size[1]];
        local.iter().all(|value| value.is_finite()).then_some(local)
    }

    /// The frame's offset after moving `across` and `up` world units along its turned directions, unturned as `layer.decal.set` stores it
    fn slid(&self, across: f32, up: f32) -> [f32; 2] {
        let (sin, cos) = self.decal.rotation.to_radians().sin_cos();
        [self.decal.offset[0] + across * cos - up * sin, self.decal.offset[1] + across * sin + up * cos]
    }
}

/// Half a turn in degrees: a ring drag's sweep is kept within one either way
const HALF_TURN: f32 = 180.0;

/// The smallest determinant of two screen steps (points² per world unit²) that still tells them apart: below it they run parallel on screen
const PARALLEL_DETERMINANT: f32 = 1e-6;

/// The smallest cosine between a view ray and a plane's normal for the ray to meet the plane: nearer edge on, the hit runs off far away
pub(crate) const EDGE_ON_FACING: f32 = 1e-4;

/// The smallest length a direction spans on screen to point somewhere (points): shorter, it points at the camera
pub(crate) const MIN_SCREEN_SPAN: f32 = 1.0;

/// The smallest cosine between the view ray and a turn ring's axis for the ring to show: flatter, its half height on screen is under its drawn line's width, it reads as a line and a drag along it turns half a turn at once across the centre
const MIN_RING_FACING: f32 = (SHAFT_WIDTH + SHAFT_HALO_EXTRA_WIDTH) / TURN_RING;

/// Whether a turn ring round `axis`, centred on screen at `center`, is seen open enough to show and grab
pub(crate) fn ring_open(app: &TextureCraftApp, rect: Rect, center: Pos2, axis: Vec3) -> bool {
    view_ray(app, rect, center).dir.dot(axis).abs() >= MIN_RING_FACING
}

/// Project a circle in the frame's plane whose longest screen reach is `TURN_RING` points.
fn turn_ring(app: &TextureCraftApp, rect: Rect, center: Pos2, origin: Vec3, across: Vec3, up: Vec3) -> Option<[Pos2; TURN_RING_POINTS]> {
    const RING_SEGMENTS: usize = TURN_RING_POINTS - 1;

    let view_projection = app.session.camera.view_proj(rect.width() / rect.height().max(1.0));
    let center_clip = view_projection * origin.extend(1.0);
    let center_ndc = center_clip / center_clip.w;
    let half_size = rect.size() * 0.5;

    // The z component tracks how the perspective divisor changes per world unit
    let screen_step = |axis: Vec3| {
        let clip_step = (view_projection * axis.extend(0.0)) / center_clip.w;
        let ndc_numerator = clip_step - center_ndc * clip_step.w;
        Vec3::new(ndc_numerator.x * half_size.x, -ndc_numerator.y * half_size.y, clip_step.w)
    };
    let across_step = screen_step(across);
    let up_step = screen_step(up);
    let ring_steps: [Vec3; RING_SEGMENTS] = std::array::from_fn(|index| {
        let (sin, cos) = (index as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU).sin_cos();
        across_step * cos + up_step * sin
    });

    // Solve radius * length(xy) / (1 + radius * z) <= TURN_RING for every sample
    let limiting_growth = ring_steps.iter().map(|step| step.truncate().length() - TURN_RING * step.z).fold(0.0, f32::max);
    if !limiting_growth.is_finite() || limiting_growth <= f32::EPSILON {
        return None;
    }

    let radius = TURN_RING / limiting_growth;
    let mut ring = [center; TURN_RING_POINTS];
    for (point, step) in ring.iter_mut().zip(ring_steps) {
        let depth = 1.0 + radius * step.z;
        if !depth.is_finite() || depth <= f32::EPSILON {
            return None;
        }

        *point += vec2(step.x, step.y) * (radius / depth);
    }
    ring[RING_SEGMENTS] = ring[0];

    Some(ring)
}

/// `delta` as a mix of screen steps `across_step` and `up_step` (None when they are parallel on screen)
fn solve(across_step: egui::Vec2, up_step: egui::Vec2, delta: egui::Vec2) -> Option<[f32; 2]> {
    let determinant = across_step.x * up_step.y - across_step.y * up_step.x;
    if determinant.abs() < PARALLEL_DETERMINANT {
        return None;
    }

    Some([(delta.x * up_step.y - delta.y * up_step.x) / determinant, (across_step.x * delta.y - across_step.y * delta.x) / determinant])
}

/// How far `p` is from the centre square (0 inside it)
pub(crate) fn center_distance(center: Pos2, p: Pos2) -> f32 {
    Rect::from_center_size(center, egui::Vec2::splat(CENTER_SQUARE)).distance_to_pos(p)
}

/// How far `p` is from the arrow from `center` to `tip`, its head included
pub(crate) fn arrow_distance(p: Pos2, center: Pos2, tip: Pos2) -> f32 {
    distance_to_segment(p, center, tip + (tip - center).normalized() * ARROW_HEAD)
}

/// The handle nearest its distance, if one is within `GRAB` (the first listed wins a tie)
pub(crate) fn nearest<T: Copy>(handles: impl IntoIterator<Item = (T, f32)>) -> Option<T> {
    let mut best: Option<(T, f32)> = None;
    for (handle, distance) in handles {
        if distance <= GRAB && best.is_none_or(|(_, nearest)| distance < nearest) {
            best = Some((handle, distance));
        }
    }
    best.map(|(handle, _)| handle)
}

pub(crate) fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

impl Gizmo for FrameGizmo {
    type Grab = FrameGrab;

    fn drag_id() -> egui::Id {
        egui::Id::unique("tc_frame_drag")
    }

    /// The handle under screen point `p`: the nearest one within reach, so a small frame's corners stay grabbable beside its arrows
    fn grab(&self, at: Pos2) -> Option<FrameGrab> {
        let mut handles = vec![(FrameGrab::Slide, center_distance(self.center, at))];
        handles.extend((0..3).filter_map(|axis| Some((FrameGrab::Axis(axis), arrow_distance(at, self.center, self.arrows[axis]?)))));
        let near = |points: [Pos2; 4], grab: fn(usize) -> FrameGrab| (0..4).map(move |index| (grab(index), points[index].distance(at)));
        match self.warp {
            Some(_) => handles.extend(near(self.corners, FrameGrab::Corner)),
            None => handles.extend(near(self.corners, |_| FrameGrab::Scale)),
        }
        handles.extend(near(self.sides, FrameGrab::Side));
        if let Some(ring) = &self.ring {
            handles.push((FrameGrab::Turn, ring.windows(2).map(|pair| distance_to_segment(at, pair[0], pair[1])).fold(f32::INFINITY, f32::min)));
        }
        nearest(handles)
    }

    fn same_projection(&self, other: &FrameGizmo) -> bool {
        self.layer == other.layer && self.effect == other.effect
    }

    /// The command and params a drag of `grab` from `from` to `to` runs
    fn edit(&self, app: &TextureCraftApp, rect: Rect, grab: FrameGrab, from: Pos2, to: Pos2) -> Option<(&'static str, Value)> {
        let delta = to - from;
        let center = self.center;
        let mut params = match grab {
            FrameGrab::Slide => json!({"offset": self.slid_on_screen(delta)?}),
            FrameGrab::Axis(axis) => {
                let step = (*self.steps.get(axis)?)?;
                let distance = delta.dot(step) / step.length_sq().max(f32::MIN_POSITIVE);
                match axis {
                    0 => json!({"offset": self.slid(distance, 0.0)}),
                    1 => json!({"offset": self.slid(0.0, distance)}),
                    _ => json!({"position": (self.origin + self.axes[2] * distance).to_array(), "normal": self.axes[2].to_array()}),
                }
            }
            FrameGrab::Scale => {
                let scale = (to - center).length() / (from - center).length().max(MIN_HANDLE_REACH);
                json!({"size": [self.decal.size[0] * scale, self.decal.size[1] * scale]})
            }
            FrameGrab::Side(side) => {
                let direction = (*self.sides.get(side)? - center).normalized();
                let stretch = ((to - center).dot(direction) / (from - center).dot(direction).max(MIN_HANDLE_REACH)).max(MIN_SIDE_STRETCH);
                let mut size = self.decal.size;
                size[1 - side % 2] *= stretch;
                json!({"size": size})
            }
            FrameGrab::Turn => {
                // Measure the turn in the frame's own plane so a tilted frame follows the pointer
                let angle = |p: Pos2| self.on_plane(app, rect, p).map(|[across, up]| (up * self.decal.size[1]).atan2(across * self.decal.size[0]).to_degrees());
                let swept = angle(to)? - angle(from)?;
                let swept = (swept + HALF_TURN).rem_euclid(2.0 * HALF_TURN) - HALF_TURN;
                json!({"rotation": self.decal.rotation + swept})
            }
            FrameGrab::Corner(corner) => {
                let mut corners = self.warp?;
                let dragged_corner = *self.corners.get(corner)? + delta;
                *corners.get_mut(corner)? = self.on_plane(app, rect, dragged_corner)?;

                json!({"warpCorners": corners})
            }
        };
        params["layer"] = json!(self.layer.0);
        if let Some(effect) = self.effect {
            params["effect"] = json!(effect.0);
        }
        let command = if matches!(grab, FrameGrab::Corner(_)) { "layer.fill.projection" } else { "layer.decal.set" };
        Some((command, params))
    }

    fn cursor(&self, grab: FrameGrab, dragging: bool) -> egui::CursorIcon {
        grab.cursor(dragging)
    }
}

/// The selected frame over the 3D view: dashed outline, corner and edge dots, and the centre square with its axis arrows
pub(crate) fn overlay(ui: &Ui, rect: Rect, gizmo: &FrameGizmo) {
    let tokens = Tokens::get(ui.ctx());
    let painter = ui.painter_at(rect);
    let active = dragging::<FrameGizmo>(ui.ctx()).map(|drag| drag.grab);
    let hovered = ui.ctx().pointer_hover_pos().filter(|hover| rect.contains(*hover)).and_then(|hover| gizmo.grab(hover));
    let lit = |grab: FrameGrab| active == Some(grab) || (active.is_none() && hovered == Some(grab));
    let dot_fill = |grab: FrameGrab| if lit(grab) { tokens.viewport_accent } else { Color32::WHITE };

    let mut outline = gizmo.corners.to_vec();
    outline.push(gizmo.corners[0]);
    dashed_outline(&painter, &outline);
    if let Some(ring) = &gizmo.ring {
        let color = if lit(FrameGrab::Turn) { tokens.viewport_accent } else { AXIS_COLORS[2] };
        painter.add(egui::Shape::line(ring.to_vec(), Stroke::new(SHAFT_WIDTH + SHAFT_HALO_EXTRA_WIDTH, SHAFT_HALO)));
        painter.add(egui::Shape::line(ring.to_vec(), Stroke::new(SHAFT_WIDTH, color)));
    }
    match gizmo.warp {
        Some(_) => {
            for (index, corner) in gizmo.corners.into_iter().enumerate() {
                handle_dot(&painter, corner, WARP_DOT, dot_fill(FrameGrab::Corner(index)));
            }
        }
        None => {
            for corner in gizmo.corners {
                handle_dot(&painter, corner, CORNER_DOT, dot_fill(FrameGrab::Scale));
            }
        }
    }
    for (index, side) in gizmo.sides.into_iter().enumerate() {
        handle_dot(&painter, side, SIDE_DOT, dot_fill(FrameGrab::Side(index)));
    }

    let arrow_color = |axis: usize| if lit(FrameGrab::Axis(axis)) { tokens.viewport_accent } else { AXIS_COLORS[axis] };
    move_handles(&painter, gizmo.center, gizmo.arrows, arrow_color, dot_fill(FrameGrab::Slide));
}

/// The centre square (filled `square`) with its X/Y/Z arrows to `arrows`, coloured by `arrow_color`; shafts and heads over a dark halo like the symmetry manipulator's
pub(crate) fn move_handles(painter: &egui::Painter, center: Pos2, arrows: [Option<Pos2>; 3], arrow_color: impl Fn(usize) -> Color32, square: Color32) {
    for (axis, tip) in arrows.into_iter().enumerate() {
        let Some(tip) = tip else { continue };

        let color = arrow_color(axis);
        let direction = (tip - center).normalized();
        let side = direction.rot90() * ARROW_HEAD * ARROW_HEAD_HALF_WIDTH;
        let start = center + direction * CENTER_SQUARE * 0.5;
        painter.line_segment([start, tip], Stroke::new(SHAFT_WIDTH + SHAFT_HALO_EXTRA_WIDTH, SHAFT_HALO));
        painter.line_segment([start, tip], Stroke::new(SHAFT_WIDTH, color));
        painter.add(egui::Shape::convex_polygon(vec![tip + direction * ARROW_HEAD, tip + side, tip - side], color, HANDLE_EDGE));
    }
    let square_rect = Rect::from_center_size(center, vec2(CENTER_SQUARE, CENTER_SQUARE));
    painter.rect(square_rect, CENTER_SQUARE_CORNER_RADIUS, square, HANDLE_EDGE, egui::StrokeKind::Middle);
}

/// The dashed outline: the dark shadow's width and colour, the light dashes' width and colour over it (white at 230 alpha, premultiplied), the dash and gap lengths
const OUTLINE_SHADOW_WIDTH: f32 = 3.0;
const OUTLINE_SHADOW: Color32 = Color32::from_black_alpha(140);
const OUTLINE_WIDTH: f32 = 1.2;
const OUTLINE_COLOR: Color32 = Color32::from_rgba_premultiplied(230, 230, 230, 230);
const OUTLINE_DASH: f32 = 6.0;
const OUTLINE_GAP: f32 = 4.0;

/// A gizmo's dashed outline through `points`, light over a dark shadow so it reads on any model
pub(crate) fn dashed_outline(painter: &egui::Painter, points: &[Pos2]) {
    let strokes = [Stroke::new(OUTLINE_SHADOW_WIDTH, OUTLINE_SHADOW), Stroke::new(OUTLINE_WIDTH, OUTLINE_COLOR)];
    crate::viewport::paint_dashed_outline(painter, points, OUTLINE_DASH, OUTLINE_GAP, strokes);
}

/// An arrow head's half width, as a share of its length
const ARROW_HEAD_HALF_WIDTH: f32 = 0.45;

/// The dark rim around a handle dot
const DOT_RIM: f32 = 1.5;
/// The thin dark edge round handle dots, arrow heads and the centre square
pub(crate) const HANDLE_EDGE: Stroke = Stroke { width: 1.0, color: Color32::from_black_alpha(200) };
/// The centre square's corner radius (points)
const CENTER_SQUARE_CORNER_RADIUS: f32 = 2.0;

/// A handle dot of `radius`: `fill` over a dark rim, so it reads on a light or dark model
pub(crate) fn handle_dot(painter: &egui::Painter, at: Pos2, radius: f32, fill: Color32) {
    painter.circle_filled(at, radius + DOT_RIM, SHAFT_HALO);
    painter.circle(at, radius, fill, HANDLE_EDGE);
}

#[cfg(test)]
pub(crate) mod tests {
    use egui::{Modifiers, PointerButton, Pos2, vec2};
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use texturecraft_engine::Session;
    use texturecraft_engine::core::{LayerId, LayerKind, Projection};

    use super::*;

    /// The test window: big enough that the docked panels leave the 3D view room for the ball with every gizmo handle round it
    const TEST_WINDOW: egui::Vec2 = vec2(1600.0, 960.0);

    /// The ball in a new project, its first frames run; with `render` the views render through wgpu
    pub(crate) fn ball_app(render: bool) -> egui_kittest::Harness<'static, TextureCraftApp> {
        let session = Session::new_project("Gizmo", texturecraft_engine::geom::primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 128).unwrap();
        let builder = egui_kittest::Harness::builder().with_size(TEST_WINDOW);
        let builder = if render { builder.wgpu() } else { builder };
        let mut harness = builder.build_eframe(|context| TextureCraftApp::new(context, session));
        harness.run_steps(4);
        harness
    }

    /// The ball with a selected fill layer projecting `projection`
    pub(crate) fn fill_app(projection: &str) -> (egui_kittest::Harness<'static, TextureCraftApp>, LayerId) {
        fill_app_with(json!({"projection": projection}))
    }

    /// The ball with a selected fill layer set up by `fields` of layer.fill.projection
    pub(crate) fn fill_app_with(fields: serde_json::Value) -> (egui_kittest::Harness<'static, TextureCraftApp>, LayerId) {
        fill_app_rendered(fields, false)
    }

    /// As [`fill_app_with`]; with `render` the views render through wgpu
    pub(crate) fn fill_app_rendered(mut fields: serde_json::Value, render: bool) -> (egui_kittest::Harness<'static, TextureCraftApp>, LayerId) {
        let mut harness = ball_app(render);
        let layer = harness.state_mut().run("layer.add.fill", json!({})).unwrap()["layer"].as_u64().unwrap();
        fields["layer"] = json!(layer);
        harness.state_mut().run("layer.fill.projection", fields).unwrap();
        harness.run_steps(2);
        (harness, LayerId(layer))
    }

    pub(crate) fn projection(harness: &egui_kittest::Harness<'_, TextureCraftApp>, layer: LayerId) -> Projection {
        match &harness.state().session.doc.project.layer(layer).unwrap().kind {
            LayerKind::Fill(fill) => fill.projection.clone(),
            _ => panic!("a fill layer"),
        }
    }

    /// A primary-button drag from `from` to `to` over a few frames, with `modifiers` held
    pub(crate) fn drag(harness: &mut egui_kittest::Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2, modifiers: Modifiers) {
        press_and_move(harness, from, to, modifiers);
        harness.event_modifiers(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers }, modifiers);
        harness.run_steps(2);
    }

    /// The press and moves of [`drag`], the button still down at `to`
    pub(crate) fn press_and_move(harness: &mut egui_kittest::Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2, modifiers: Modifiers) {
        harness.event_modifiers(egui::Event::PointerMoved(from), modifiers);
        harness.event_modifiers(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers }, modifiers);
        harness.run_steps(2);
        for step in 1..=4 {
            harness.event_modifiers(egui::Event::PointerMoved(from.lerp(to, step as f32 / 4.0)), modifiers);
            harness.run_steps(1);
        }
    }

    /// Whether a frame drag is under way
    fn frame_dragging(harness: &egui_kittest::Harness<'_, TextureCraftApp>) -> bool {
        crate::projection_gizmo::dragging::<FrameGizmo>(&harness.ctx).is_some()
    }

    /// A drag as a mouse sends it, one small move per frame; after each frame `check` looks at the app and (with `preview`) the 3D preview square must have re-rendered
    #[track_caller]
    pub(crate) fn drag_frames(
        harness: &mut egui_kittest::Harness<'_, TextureCraftApp>,
        layer: LayerId,
        from: Pos2,
        to: Pos2,
        modifiers: Modifiers,
        preview: bool,
        mut check: impl FnMut(&egui_kittest::Harness<'_, TextureCraftApp>),
    ) {
        harness.event_modifiers(egui::Event::PointerMoved(from), modifiers);
        harness.event_modifiers(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers }, modifiers);
        harness.run_steps(2);
        let mut shown = crate::projection_preview::shown_key(&harness.ctx);
        let mut was = projection(harness, layer);
        let mut moved = 0;
        for step in 1..=6 {
            harness.event_modifiers(egui::Event::PointerMoved(from.lerp(to, step as f32 / 6.0)), modifiers);
            harness.step();
            let now = projection(harness, layer);

            // Until the pointer passes the drag threshold nothing moves
            if now == was {
                assert_eq!(moved, 0, "frame {step}: once started, every frame of the drag moves it");
                continue;
            }

            moved += 1;
            check(harness);
            let key = crate::projection_preview::shown_key(&harness.ctx);
            assert!(!preview || (key.is_some() && key != shown), "frame {step}: the preview square re-rendered for the new values");
            (shown, was) = (key, now);
        }
        assert!(moved >= 4, "the drag moved it on {moved} frames");
        harness.event_modifiers(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers }, modifiers);
        harness.run_steps(2);
        check(harness);
    }

    /// The panel's X, Y and Z fields of a 3D projection settings row
    pub(crate) fn panel_xyz(harness: &egui_kittest::Harness<'_, TextureCraftApp>, row: &str) -> [f32; 3] {
        use egui_kittest::kittest::NodeT;
        ["X", "Y", "Z"].map(|tag| harness.get_by_label(&format!("{row} {tag}")).accesskit_node().numeric_value().unwrap() as f32)
    }

    fn gizmo(harness: &egui_kittest::Harness<'_, TextureCraftApp>) -> FrameGizmo {
        FrameGizmo::of(harness.state(), harness.state().viewport_rect).unwrap()
    }

    #[test]
    fn an_axis_planar_is_placed_on_its_first_drag_and_its_handles_move_size_and_stretch_it() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        // Repeating three times across the mesh, the frame is smaller than the ball
        let (mut harness, layer) = fill_app_with(json!({"projection": "planar", "scale": 3}));
        let axis = gizmo(&harness);

        assert!(matches!(projection(&harness, layer), Projection::Planar { placed: None, .. }), "a new planar projects along its axis");
        assert!(axis.warp.is_none() && axis.arrows.iter().all(Option::is_some), "a planar frame with its three arrows: {axis:?}");

        drag(&mut harness, axis.center, axis.center + vec2(30.0, 0.0), Modifiers::NONE);
        let Projection::Planar { placed: Some(slid), .. } = projection(&harness, layer) else { panic!("placed on the first drag") };
        assert!(slid.offset[0] > 0.0 && slid.offset[1].abs() < slid.offset[0] * 0.2, "the centre square slides it right: {slid:?}");
        assert_eq!(slid.size, axis.decal.size, "sliding keeps its size");
        assert_eq!(harness.state().session.history_labels().last().map(String::as_str), Some("Edit decal"));

        let steps = harness.state().session.history_labels().len();

        let placed = gizmo(&harness);
        drag(&mut harness, placed.sides[1], placed.center + (placed.sides[1] - placed.center) * 0.5, Modifiers::NONE);
        let Projection::Planar { placed: Some(stretched), .. } = projection(&harness, layer) else { panic!("still placed") };
        assert!(stretched.size[0] < slid.size[0] * 0.75 && stretched.size[1] == slid.size[1], "the right edge dot narrows it only: {stretched:?}");

        let narrowed = gizmo(&harness);
        drag(&mut harness, narrowed.corners[2], narrowed.center + (narrowed.corners[2] - narrowed.center) * 1.5, Modifiers::NONE);
        let Projection::Planar { placed: Some(scaled), .. } = projection(&harness, layer) else { panic!("still placed") };
        assert!(scaled.size[1] > stretched.size[1] * 1.2, "a corner scales it: {scaled:?}");
        assert_eq!(harness.state().session.history_labels().len(), steps + 2, "one undo step per drag");
    }

    #[test]
    fn a_warp_corner_follows_the_pointer_on_the_frame_plane() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("warp");
        let warp = gizmo(&harness);

        assert_eq!(warp.warp, Some(Projection::FRAME_CORNERS));

        let to = warp.corners[1] + vec2(-40.0, 30.0);
        drag(&mut harness, warp.corners[1], to, Modifiers::NONE);
        let Projection::Warp { corners, frame, .. } = projection(&harness, layer) else { panic!("still a warp") };
        assert_eq!((frame, corners[0], corners[2], corners[3]), (warp.decal, [-0.5, 0.5], [0.5, -0.5], [-0.5, -0.5]), "only that corner moves");
        assert!(corners[1][0] < 0.45 && corners[1][1] < 0.45, "top right pulled in: {corners:?}");
        assert!(gizmo(&harness).corners[1].distance(to) < 1.5, "the corner sits under the pointer");
    }

    #[test]
    fn grabbing_the_side_of_a_warp_corner_does_not_snap_it_to_the_pointer() {
        let (mut harness, _) = fill_app("warp");
        let warp = gizmo(&harness);
        let corner = warp.corners[1];
        let press = corner + vec2(6.0, 2.0);
        let movement = vec2(-40.0, 30.0);

        assert_eq!(warp.grab(press), Some(FrameGrab::Corner(1)));

        drag(&mut harness, press, press + movement, Modifiers::NONE);

        let moved = gizmo(&harness).corners[1];
        assert!(moved.distance(corner + movement) < 1.5, "the original grab offset is kept: {moved:?}");
    }

    /// A move that arrives in the same frame as the release still moves the corner: it ends under the pointer, not one step short
    #[test]
    fn a_move_in_the_release_frame_still_moves_the_corner() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, _) = fill_app("warp");
        let warp = gizmo(&harness);
        let to = warp.corners[1] + vec2(-40.0, 30.0);
        press_and_move(&mut harness, warp.corners[1], warp.corners[1].lerp(to, 0.5), Modifiers::NONE);

        // Straight into the next frame's input: the harness's queued events would each get a frame of their own
        let release = egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
        harness.input_mut().events.extend([egui::Event::PointerMoved(to), release]);
        harness.run_steps(2);

        assert!(crate::projection_gizmo::dragging::<FrameGizmo>(&harness.ctx).is_none(), "the release ends the drag");
        assert!(gizmo(&harness).corners[1].distance(to) < 1.5, "the corner sits under the release point: {:?} {to:?}", gizmo(&harness).corners[1]);
    }

    /// The across and up arrows slide a frame on its own plane and the out arrow moves it along its normal, each following the pointer; one undo step per drag
    #[test]
    fn the_arrows_move_a_frame_along_its_directions() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("warp");
        let steps = harness.state().session.history_labels().len();
        let frame = |harness: &egui_kittest::Harness<'_, TextureCraftApp>| match projection(harness, layer) {
            Projection::Warp { frame, .. } => frame,
            other => panic!("still a warp: {other:?}"),
        };

        for axis in 0..3 {
            let start = gizmo(&harness);
            let tip = start.arrows[axis].unwrap_or_else(|| panic!("arrow {axis} shows"));
            let along = (tip - start.center).normalized();
            let before = frame(&harness);
            drag(&mut harness, tip, tip + along * 25.0, Modifiers::NONE);

            let moved = gizmo(&harness);

            assert!(
                (moved.center - start.center).dot(along) > 15.0,
                "arrow {axis}: the centre follows the pointer along it: {:?} -> {:?}",
                start.center,
                moved.center
            );
            assert_eq!(frame(&harness).normal, before.normal, "arrow {axis}: moving never turns the frame");

            // Across and up slide it on its own plane, out moves it along its normal
            let shift = moved.origin - start.origin;
            assert!(shift.normalize().dot(start.axes[axis]) > 0.99, "arrow {axis} moves it along its own direction: {shift:?}");
            assert_eq!(harness.state().session.history_labels().len(), steps + axis + 1, "arrow {axis}: one undo step per drag");
        }
    }

    /// The panel's Rotation turns a frame and its Scale sizes it, so the gizmo does both: the ring turns a planar or warp frame round its normal, following the pointer round the centre; a warp's edge dots size its frame as a planar's do
    #[test]
    fn the_ring_turns_a_frame_and_a_warps_edge_dots_size_it() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for kind in ["warp", "planar"] {
            let (mut harness, _) = fill_app(kind);
            let start = gizmo(&harness);
            let ring = start.ring.unwrap_or_else(|| panic!("{kind}: a frame facing the camera shows its ring"));
            let on_ring = *ring
                .iter()
                .filter(|at| start.grab(**at) == Some(FrameGrab::Turn))
                .max_by(|a, b| a.y.total_cmp(&b.y))
                .expect("a ring point clear of the other handles");
            let steps = harness.state().session.history_labels().len();
            let quarter = start.center + (on_ring - start.center).rot90();
            drag(&mut harness, on_ring, quarter, Modifiers::NONE);
            let turned = gizmo(&harness);
            let sweep = |from: egui::Vec2, to: egui::Vec2| (to.angle() - from.angle()).to_degrees();
            let pointer = sweep(on_ring - start.center, quarter - start.center);
            let corner = sweep(start.corners[1] - start.center, turned.corners[1] - turned.center);
            let off = (corner - pointer + 180.0).rem_euclid(360.0) - 180.0;

            assert!(off.abs() < 6.0, "{kind}: the frame turns a quarter with the pointer: corner {corner}°, pointer {pointer}°");
            assert!(turned.center.distance(start.center) < 1.0, "{kind}: round its centre");
            assert_eq!(harness.state().session.history_labels().len(), steps + 1, "{kind}: one undo step");
        }

        let (mut harness, layer) = fill_app("warp");
        let warp = gizmo(&harness);
        assert_eq!(warp.grab(warp.sides[1]), Some(FrameGrab::Side(1)));

        drag(&mut harness, warp.sides[1], warp.center + (warp.sides[1] - warp.center) * 0.5, Modifiers::NONE);
        let Projection::Warp { frame, corners, .. } = projection(&harness, layer) else { panic!("still a warp") };
        assert!(frame.size[0] < warp.decal.size[0] * 0.75 && (frame.size[1] - warp.decal.size[1]).abs() < 1e-4, "the right edge dot narrows it: {frame:?}");
        assert_eq!(corners, Projection::FRAME_CORNERS, "its corners stay where they are on the frame");
    }

    /// The longest reach stays near the intended screen radius as the camera orbits.
    #[test]
    fn the_ring_keeps_its_screen_size_at_oblique_angles() {
        const MAX_RING_SHARE: f32 = 1.25;
        const MIN_RING_SHARE: f32 = 0.6;
        const ORBIT_DEGREES: [f32; 6] = [0.0, 40.0, 60.0, 75.0, 80.0, 84.0];
        const ZOOM_OUT_EXPONENT: f32 = 1.5;
        const MAX_FORESHORTENED_ACROSS_TO_UP_RATIO: f32 = 0.4;

        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for kind in ["planar", "warp"] {
            let (mut harness, _) = fill_app(kind);
            let mut foreshortened_shown = false;

            // A distant view isolates the foreshortening
            harness.state_mut().session.camera.zoom(-ZOOM_OUT_EXPONENT);
            for degrees in ORBIT_DEGREES {
                harness.state_mut().session.camera.set_angles(degrees.to_radians(), 0.0);
                harness.run_steps(2);

                let shown = gizmo(&harness);
                let Some(ring) = shown.ring else { continue };
                let reach = ring.iter().map(|at| at.distance(shown.center)).fold(0.0, f32::max);
                let across_step = shown.steps[0].unwrap().length();
                let up_step = shown.steps[1].unwrap().length();

                assert!(
                    (TURN_RING * MIN_RING_SHARE..=TURN_RING * MAX_RING_SHARE).contains(&reach),
                    "{kind} orbited {degrees} degrees: the ring reaches {reach} points"
                );

                foreshortened_shown |= across_step < up_step * MAX_FORESHORTENED_ACROSS_TO_UP_RATIO;
            }

            assert!(foreshortened_shown, "{kind}: the ring was checked with the across direction foreshortened");
        }
    }

    #[test]
    fn the_ring_keeps_its_screen_size_when_the_oblique_frame_is_turning_near_the_camera() {
        const FRAME_SIDE: f32 = 4.0;
        const LARGE_FRAME_SIDE: f32 = 5.0;
        const CAMERA_DISTANCE: f32 = 4.0;
        const ORBIT_DEGREES: f32 = 65.0;
        const EDGE_ON_ANGLE: f32 = std::f32::consts::FRAC_PI_2;
        const MIN_RING_SHARE: f32 = 0.75;
        const MAX_RING_SHARE: f32 = 1.25;

        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for kind in ["planar", "warp"] {
            let (mut harness, layer) = fill_app(kind);
            let camera = &mut harness.state_mut().session.camera;
            camera.distance = CAMERA_DISTANCE;
            camera.set_angles(ORBIT_DEGREES.to_radians(), 0.0);

            // Reverse the frame axes, then put a large frame's across step behind the camera
            for (rotation_degrees, side) in [(-45.0, FRAME_SIDE), (135.0, FRAME_SIDE), (0.0, LARGE_FRAME_SIDE)] {
                let size = [side; 2];
                harness.state_mut().run("layer.decal.set", json!({"layer": layer.0, "size": size, "rotation": rotation_degrees})).unwrap();
                harness.run_steps(2);

                let shown = gizmo(&harness);
                let ring = shown.ring.unwrap_or_else(|| panic!("{kind} turned {rotation_degrees} degrees: the oblique ring should show"));
                let reach = ring.iter().map(|point| point.distance(shown.center)).fold(0.0, f32::max);

                assert!(
                    (TURN_RING * MIN_RING_SHARE..=TURN_RING * MAX_RING_SHARE).contains(&reach),
                    "{kind} turned {rotation_degrees} degrees: the ring reaches {reach} points"
                );
            }

            harness.state_mut().session.camera.set_angles(EDGE_ON_ANGLE, 0.0);
            harness.run_steps(2);

            assert!(gizmo(&harness).ring.is_none(), "{kind}: hide the ring when the frame is edge-on");
        }
    }

    /// Losing the focus mid-drag (Alt+Tab) ends a frame drag as a release does: one undo step, and moves after it (egui still has the button down) leave the frame alone
    #[test]
    fn losing_the_window_focus_ends_a_frame_drag() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("planar");
        let steps = harness.state().session.history_labels().len();
        let center = gizmo(&harness).center;
        let to = center + vec2(40.0, 0.0);
        press_and_move(&mut harness, center, to, Modifiers::NONE);

        assert!(frame_dragging(&harness), "the centre square is held");

        harness.event(egui::Event::WindowFocused(false));
        harness.run_steps(1);
        let left = projection(&harness, layer);
        assert!(!frame_dragging(&harness), "the focus loss ends the drag");
        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "as one undo step");

        for step in 1..=3 {
            harness.event(egui::Event::PointerMoved(to + vec2(20.0, 10.0) * step as f32));
            harness.run_steps(1);
        }

        assert_eq!(projection(&harness, layer), left, "moves after it leave the frame where it was");

        harness.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        harness.run_steps(2);

        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "and add no step");
    }

    /// Alt pressed mid-drag hands the pointer to the camera and ends the frame drag where it is: let go of Alt with the button still down, the drag does not resume from where it began; one undo step
    #[test]
    fn alt_mid_drag_ends_a_frame_drag() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("planar");
        let steps = harness.state().session.history_labels().len();
        let center = gizmo(&harness).center;
        let to = center + vec2(40.0, 0.0);
        press_and_move(&mut harness, center, to, Modifiers::NONE);
        let slid = projection(&harness, layer);
        let camera = harness.state().session.camera;

        harness.event_modifiers(egui::Event::PointerMoved(to + vec2(20.0, 0.0)), Modifiers::ALT);
        harness.run_steps(1);

        assert_ne!(harness.state().session.camera, camera, "Alt turns the camera");
        assert!(!frame_dragging(&harness), "and ends the frame drag");

        for step in 2..=4 {
            harness.event(egui::Event::PointerMoved(to + vec2(20.0, 5.0) * step as f32));
            harness.run_steps(1);
        }

        assert_eq!(projection(&harness, layer), slid, "Alt let go, the frame stays where the drag left it");

        harness.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        harness.run_steps(2);

        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "one undo step");
    }

    /// The rest of a drag held at `from`: moves to `to` over a few frames and lets go there
    fn move_and_let_go(harness: &mut egui_kittest::Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2) {
        for step in 1..=4 {
            harness.event(egui::Event::PointerMoved(from.lerp(to, step as f32 / 4.0)));
            harness.run_steps(1);
        }
        harness.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        harness.run_steps(2);
    }

    /// A drag whose first edit changes nothing (its arrow pulled further past the offset limit) makes no undo step, so its later edits make the drag's own step instead of joining the edit before it
    #[test]
    fn a_drag_that_first_changes_nothing_keeps_its_own_undo_step() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("warp");

        // Offset the frame from the panels and back the camera up to expose its arrows
        harness.state_mut().run("layer.decal.set", json!({"layer": layer.0, "offset": [-100.0, 0.0]})).unwrap();
        harness.state_mut().session.camera.distance *= 3.0;
        harness.run_steps(2);
        let at_limit = projection(&harness, layer);
        let steps = harness.state().session.history_labels().len();
        let start = gizmo(&harness);
        let tip = start.arrows[0].expect("the across arrow shows");
        let inward = (tip - start.center).normalized();

        press_and_move(&mut harness, tip, tip - inward * 25.0, Modifiers::NONE);

        assert_eq!(projection(&harness, layer), at_limit, "past the limit the frame stays where it is");

        move_and_let_go(&mut harness, tip - inward * 25.0, tip + inward * 25.0);

        assert_ne!(projection(&harness, layer), at_limit, "back inside the frame moves");
        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "the drag made its own undo step");

        harness.state_mut().session.undo().unwrap();

        assert_eq!(projection(&harness, layer), at_limit, "undo takes back the drag only");
    }

    /// An axis planar reading a pattern keeps its axis, so it shows no frame to drag; reading an image it shows one
    #[test]
    fn an_axis_planar_reading_a_pattern_has_no_frame() {
        let (mut harness, layer) = fill_app("planar");

        assert!(FrameGizmo::of(harness.state(), harness.state().viewport_rect).is_some(), "a uniform fill places as its frame");

        harness.state_mut().run("layer.fill.set", json!({"layer": layer.0, "channel": "roughness", "procedural": "Checker"})).unwrap();
        harness.run_steps(2);

        assert!(FrameGizmo::of(harness.state(), harness.state().viewport_rect).is_none(), "placing would move the pattern");
    }

    /// An Undo mid-drag takes the drag's step away: the drag's next edits make a step of their own instead of joining the drag before it
    #[test]
    fn undo_mid_drag_leaves_the_drag_before_its_own_step() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("warp");
        let center = gizmo(&harness).center;
        drag(&mut harness, center, center + vec2(30.0, 0.0), Modifiers::NONE);
        let first = projection(&harness, layer);
        let steps = harness.state().session.history_labels().len();

        let center = gizmo(&harness).center;
        press_and_move(&mut harness, center, center + vec2(0.0, 30.0), Modifiers::NONE);
        harness.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
        harness.run_steps(1);

        assert_eq!(projection(&harness, layer), first, "Ctrl+Z undid the drag so far");

        move_and_let_go(&mut harness, center + vec2(0.0, 30.0), center + vec2(0.0, 40.0));

        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "the drag's later edits made a step of their own");

        harness.state_mut().session.undo().unwrap();

        assert_eq!(projection(&harness, layer), first, "undo goes back to where the first drag left the frame");
    }

    /// A click as a mouse sends it: move there, press, release
    fn click(harness: &mut egui_kittest::Harness<'_, TextureCraftApp>, at: Pos2) {
        harness.event(egui::Event::PointerMoved(at));
        harness.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        harness.run_steps(1);
        harness.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        harness.run_steps(2);
    }

    /// The Projection row's eye hides the gizmo: not drawn, a press on where its handle was moves nothing, and the eye brings it back
    #[test]
    fn the_projection_rows_eye_hides_and_shows_the_gizmo() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app_with(json!({"projection": "planar", "scale": 3}));
        let shown = gizmo(&harness);
        let eye = harness.get_by_label("Hide projection gizmos").rect();
        let projection_row = harness.get_by_label("Planar projection").rect();

        assert!(eye.left() > projection_row.right() && (eye.center().y - projection_row.center().y).abs() < 0.5, "the eye ends the Projection row");

        click(&mut harness, eye.center());

        assert!(!harness.state().session.doc.project.display.projection_gizmos, "the eye hides the gizmos");
        assert!(crate::projection_gizmo::ShownGizmo::of(harness.state(), harness.state().viewport_rect).is_none(), "the 3D view has no frame to draw or grab");

        let before = projection(&harness, layer);
        drag(&mut harness, shown.center, shown.center + vec2(30.0, 0.0), Modifiers::NONE);

        assert_eq!(projection(&harness, layer), before, "a drag where the centre square was moves nothing");

        let eye = harness.get_by_label("Show projection gizmos").rect();
        click(&mut harness, eye.center());

        assert!(harness.state().session.doc.project.display.projection_gizmos, "the eye shows them again");

        drag(&mut harness, shown.center, shown.center + vec2(30.0, 0.0), Modifiers::NONE);

        assert_ne!(projection(&harness, layer), before, "shown, the centre square slides it again");

        // A UV projection has no gizmo: its row keeps the whole width for the dropdown
        harness.state_mut().run("layer.fill.projection", json!({"layer": layer.0, "projection": "uv"})).unwrap();
        harness.run_steps(2);

        assert!(harness.query_by_label("Hide projection gizmos").is_none(), "no eye without a gizmo");
    }

    #[test]
    fn round_projections_have_no_frame() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for projection in ["uv", "triplanar", "spherical", "cylindrical"] {
            let (harness, _) = fill_app(projection);

            assert!(FrameGizmo::of(harness.state(), harness.state().viewport_rect).is_none(), "{projection}");
        }
    }

    /// The planar and warp frames over the ball, a handle hovered, Move on model lit, and the round projectors
    #[test]
    #[ignore = "requires an offscreen GPU adapter and TC_UI_SNAPSHOTS"]
    fn frame_gizmo_snapshots() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(directory) = std::env::var_os("TC_UI_SNAPSHOTS").map(std::path::PathBuf::from) else { return };

        std::fs::create_dir_all(&directory).unwrap();
        let checker = |harness: &mut egui_kittest::Harness<'_, TextureCraftApp>, layer: LayerId| {
            let fields =
                json!({"layer": layer.0, "channel": "baseColor", "procedural": "Checker", "scale": 4, "low": [0.2, 0.2, 0.2], "high": [0.6, 0.6, 0.6]});
            harness.state_mut().run("layer.fill.set", fields).unwrap();
            harness.state_mut().run("layer.fill.projection", json!({"layer": layer.0, "shapeCrop": "cropped"})).unwrap();
        };
        let (mut harness, layer) = fill_app_rendered(json!({"projection": "planar", "scale": 3}), true);
        checker(&mut harness, layer);
        harness.run_steps(4);
        let planar = gizmo(&harness);
        harness.hover_at(planar.sides[1]);
        harness.run_steps(4);
        harness.render().unwrap().save(directory.join("move-planar-side-hover.png")).unwrap();

        // Move on model held down and dragged into the 3D view: the button stays lit and the frame follows
        let button = harness.get_by_label("Move on model").rect().center();
        harness.event(egui::Event::PointerMoved(button));
        harness.event(egui::Event::PointerButton { pos: button, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        harness.run_steps(2);
        for step in 1..=6 {
            harness.event(egui::Event::PointerMoved(button + vec2(-40.0, 10.0) * step as f32));
            harness.run_steps(1);
        }
        harness.run_steps(3);
        harness.render().unwrap().save(directory.join("move-planar-held.png")).unwrap();
        harness.event(egui::Event::PointerButton {
            pos: button + vec2(-240.0, 60.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        harness.run_steps(2);

        let (mut harness, layer) = fill_app_rendered(json!({"projection": "warp"}), true);
        checker(&mut harness, layer);
        let pulled = json!([[-0.35, 0.3], [0.4, 0.42], [0.42, -0.4], [-0.3, -0.32]]);
        harness.state_mut().run("layer.fill.projection", json!({"layer": layer.0, "warpCorners": pulled, "projectionDepth": 1})).unwrap();
        harness.run_steps(4);
        harness.hover_at(gizmo(&harness).corners[1]);
        harness.run_steps(4);
        harness.render().unwrap().save(directory.join("move-warp-corner-hover.png")).unwrap();

        // Mid drag: the panel's 3D projection settings and the preview square hold the frame the gizmo is dragging
        let (mut harness, layer) = fill_app_rendered(json!({"projection": "planar", "scale": 3}), true);
        checker(&mut harness, layer);
        harness.run_steps(4);
        let held = gizmo(&harness);
        let (from, to) = (held.sides[0], held.center + (held.sides[0] - held.center) * 0.6 + vec2(30.0, 0.0));
        harness.event(egui::Event::PointerMoved(from));
        harness.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        harness.run_steps(2);
        for step in 1..=6 {
            harness.event(egui::Event::PointerMoved(from.lerp(to, step as f32 / 6.0)));
            harness.run_steps(1);
        }
        harness.run_steps(3);
        harness.render().unwrap().save(directory.join("sync-planar-mid-drag.png")).unwrap();
        harness.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        harness.run_steps(2);

        let (mut harness, layer) = fill_app_rendered(json!({"projection": "spherical"}), true);
        checker(&mut harness, layer);
        harness.run_steps(4);
        let round = crate::round_gizmo::RoundGizmo::of(harness.state(), harness.state().viewport_rect).unwrap();
        let dot = round.sizes.iter().find(|size| size.axes == [true; 3]).expect("the even-scale dot").at;
        let to = round.center + (dot - round.center) * 0.6;
        harness.event(egui::Event::PointerMoved(dot));
        harness.event(egui::Event::PointerButton { pos: dot, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        harness.run_steps(2);
        for step in 1..=6 {
            harness.event(egui::Event::PointerMoved(dot.lerp(to, step as f32 / 6.0)));
            harness.run_steps(1);
        }
        harness.run_steps(3);
        harness.render().unwrap().save(directory.join("sync-sphere-mid-drag.png")).unwrap();

        // Show the sphere size dot and cylinder height arrow on selected round projectors
        for kind in ["triplanar", "spherical", "cylindrical"] {
            let (mut harness, layer) = fill_app_rendered(json!({"projection": kind}), true);
            checker(&mut harness, layer);
            harness.run_steps(4);
            let round = crate::round_gizmo::RoundGizmo::of(harness.state(), harness.state().viewport_rect).unwrap();
            let size_at = |axes: [bool; 3]| round.sizes.iter().find(|size| size.axes == axes).expect("the size handle").at;
            let hover = match kind {
                "triplanar" => round.rings[2][crate::round_gizmo::CIRCLE_POINTS / 8],
                "spherical" => size_at([true; 3]),
                _ => size_at([false, true, false]),
            };
            harness.hover_at(hover);
            harness.run_steps(4);
            harness.render().unwrap().save(directory.join(format!("move-{kind}.png"))).unwrap();
        }
    }

    /// The panel's 3D projection settings show the frame the gizmo drags (its centre, turn and size), and the preview square follows, every frame of every drag: resize, stretch, slide, turn, a warp's slide and corners; one undo step per drag
    #[test]
    fn the_panel_and_its_preview_follow_every_frame_of_a_frame_drag() {
        // Dolly exponent stepping the camera back before the corner drag
        const CORNER_ZOOM_OUT: f32 = 0.4;

        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        use texturecraft_engine::compose::frame_placement::placement;

        let check = |layer: LayerId| {
            move |harness: &egui_kittest::Harness<'_, TextureCraftApp>| {
                let Some(shown) = placement(&projection(harness, layer), harness.state().session.mesh.bounds()) else { panic!("placed on its first drag") };
                for (row, value) in [("Offset", shown.offset), ("Rotation", shown.rotation), ("Scale", shown.scale)] {
                    let field = panel_xyz(harness, row);
                    for axis in 0..3 {
                        assert!((field[axis] - value[axis]).abs() < 1e-3, "the panel's {row} shows the frame's {value:?}, not {field:?}");
                    }
                }
            }
        };

        let (mut harness, layer) = fill_app("planar");
        let steps = harness.state().session.history_labels().len();
        let center = gizmo(&harness).center;
        drag_frames(&mut harness, layer, center, center + vec2(30.0, -10.0), Modifiers::NONE, true, check(layer));
        let slid = placement(&projection(&harness, layer), harness.state().session.mesh.bounds()).unwrap();

        // The top edge dot and a corner inside the view (the frame starts bigger than the model)
        let side = gizmo(&harness);
        drag_frames(&mut harness, layer, side.sides[0], side.center + (side.sides[0] - side.center) * 0.5, Modifiers::NONE, true, check(layer));
        let narrowed = placement(&projection(&harness, layer), harness.state().session.mesh.bounds()).unwrap();

        assert!(narrowed.scale[1] < slid.scale[1] * 0.75 && (narrowed.scale[0] - slid.scale[0]).abs() < 1e-4, "{slid:?} -> {narrowed:?}");

        // The view steps back so a corner shows beside the panels
        harness.state_mut().session.camera.zoom(-CORNER_ZOOM_OUT);
        harness.run_steps(2);
        let corner = gizmo(&harness);
        let view = harness.state().viewport_rect.shrink(10.0);
        let inside = *corner.corners.iter().find(|at| view.contains(**at)).expect("a corner in the view");
        drag_frames(&mut harness, layer, inside, corner.center + (inside - corner.center) * 0.6, Modifiers::NONE, true, check(layer));
        let scaled = placement(&projection(&harness, layer), harness.state().session.mesh.bounds()).unwrap();

        assert!(scaled.scale[1] < narrowed.scale[1] * 0.8 && scaled.scale[0] < narrowed.scale[0] * 0.8, "a corner scales it: {scaled:?}");

        // Move on model held with Shift turns the frame round its normal
        let before = placement(&projection(&harness, layer), harness.state().session.mesh.bounds()).unwrap();
        let button = harness.get_by_label("Move on model").rect().center();
        drag_frames(&mut harness, layer, button, button + vec2(-60.0, 0.0), Modifiers::SHIFT, true, check(layer));
        let turned = placement(&projection(&harness, layer), harness.state().session.mesh.bounds()).unwrap();

        assert!((turned.rotation[2] - before.rotation[2]).abs() > 10.0, "turned: {before:?} -> {turned:?}");
        assert_eq!(harness.state().session.history_labels().len(), steps + 4, "one undo step per drag");

        let (mut harness, layer) = fill_app("warp");
        let warp = gizmo(&harness);
        drag_frames(&mut harness, layer, warp.center, warp.center + vec2(30.0, 25.0), Modifiers::NONE, false, check(layer));
        let warp = gizmo(&harness);
        drag_frames(&mut harness, layer, warp.corners[0], warp.corners[0] + vec2(-25.0, -15.0), Modifiers::NONE, false, check(layer));
        let Projection::Warp { corners, .. } = projection(&harness, layer) else { panic!("a warp") };
        assert_ne!(corners[0], Projection::FRAME_CORNERS[0], "the corner was pulled");
    }
}
