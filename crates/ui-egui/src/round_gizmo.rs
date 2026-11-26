//! The selected tri-planar, spherical or cylindrical fill shows its projector round the whole model: a dashed box, sphere or cylinder with the centre square, X/Y/Z arrows and turn rings, and size handles: a dot scales it evenly (or a cylinder across its axis), a double arrow stretches it along one direction. Only a press on one of those handles moves it: any other drag paints or turns the camera

use egui::{Color32, Pos2, Rect, Stroke, Ui};
use serde_json::{Value, json};
use texturecraft_engine::compose::pattern::{cylinder_axis_index, mesh_frame, nonzero_stretch, projector_stretch};
use texturecraft_engine::core::Projection;
use texturecraft_engine::geom::glam::{EulerRot, Mat3, Quat, Vec2, Vec3};
use texturecraft_engine::layer_cmds::signed_scale;

use crate::TextureCraftApp;
use crate::frame_gizmo::{
    ARROW, EDGE_ON_FACING, HANDLE_EDGE, MIN_HANDLE_REACH, MIN_SCREEN_SPAN, TURN_RING, arrow_distance, center_distance, dashed_outline, distance_to_segment,
    handle_dot, move_handles, nearest, ring_open,
};
use crate::move_on_model::MoveOnModel;
use crate::projection_gizmo::{Gizmo, dragging};
use crate::theme::{AXIS_COLORS, SHAFT_HALO, SHAFT_HALO_EXTRA_WIDTH, SHAFT_WIDTH, Tokens};
use crate::viewport::{project, view_ray};

mod round_grab;
mod shape;
mod size_handle;

pub(crate) use round_grab::RoundGrab;
use shape::Shape;
pub(crate) use size_handle::SizeHandle;

/// The projector of the fill Move on model moves, as the 3D view shows it
#[derive(Clone, Debug)]
pub(crate) struct RoundGizmo {
    pub(crate) target: MoveOnModel,
    command: &'static str,
    shape: Shape,
    /// Mesh centre and radius: the projector reads positions around it in radii
    mesh_center: Vec3,
    radius: f32,
    /// The projector's turn and per-axis scale (a mirrored axis keeps its sign, as the projection reads it)
    turn: Mat3,
    stretch: Vec3,
    /// The projection's centre in the projector's units, and what offset puts it there (offset = -local × this)
    local: Vec3,
    offset_per_local: f32,
    /// On screen: the centre, the arrow tips, the turn rings, the dashed outline and the size handles
    pub center: Pos2,
    pub arrows: [Option<Pos2>; 3],
    pub rings: [Vec<Pos2>; 3],
    outline: Vec<Vec<Pos2>>,
    pub sizes: Vec<SizeHandle>,
    /// Screen movement per world unit along each projector direction at the centre
    steps: [Option<egui::Vec2>; 3],
}

/// How far round the model each shape reaches, in the mesh's half sizes: the box, the sphere, the cylinder's radius and half height
pub(crate) const BOX_REACH: f32 = 1.08;
pub(crate) const SPHERE_REACH: f32 = 1.2;
pub(crate) const CYLINDER_REACH: f32 = 1.1;
pub(crate) const CYLINDER_HEIGHT: f32 = 1.15;
/// The size dots' radius (points)
const SIZE_DOT: f32 = 7.0;
/// A size double arrow's half length, tip to middle, and its heads' length and half width (points)
const SIZE_ARROW: f32 = 12.0;
const SIZE_ARROW_HEAD: f32 = 7.0;
const SIZE_ARROW_HEAD_HALF_WIDTH: f32 = 5.0;
/// How far from the centre a box face's double arrow must land on screen to show: nearer, the face is seen head on and its arrow would sit on the centre square
const FACE_CLEARANCE: f32 = 40.0;
/// Points on a drawn circle
pub(crate) const CIRCLE_POINTS: usize = 48;
/// A box's corners, and the bit of a corner's index that flips it along each axis
const BOX_CORNERS: usize = 8;
const BOX_AXIS_BITS: [usize; 3] = [1, 2, 4];
/// How far along each direction its screen step is measured, in mesh radii: half way out, inside the model, where perspective bends it little
const STEP_PROBE_RADII: f32 = 0.5;

impl RoundGizmo {
    /// The projector of the selected tri-planar, spherical or cylindrical fill
    pub(crate) fn of(app: &TextureCraftApp, rect: Rect) -> Option<RoundGizmo> {
        if app.ui.mode != crate::state::Mode::Painting {
            return None;
        }

        let target = MoveOnModel::selected(app)?;
        let (projection, command) = target.projection_editor(app)?;
        let (shape, offset, projector, offset_per_local) = match projection {
            Projection::Triplanar { scale, offset, projector, .. } => (Shape::Box, offset, projector, *scale),
            Projection::Spherical { offset, projector, .. } => (Shape::Sphere, offset, projector, 1.0),
            Projection::Cylindrical { axis, offset, projector, .. } => (Shape::Cylinder(cylinder_axis_index(*axis)), offset, projector, 1.0),
            _ => return None,
        };

        let (lower, upper) = app.viewport_mesh_bounds.get(&app.session.mesh);
        let (mesh_center, radius) = mesh_frame((lower, upper));
        let [x, y, z] = projector.rotation.map(f32::to_radians);
        let turn = Mat3::from_euler(EulerRot::ZYX, z, y, x);
        let stretch = projector_stretch(projector);
        let offset_per_local = nonzero_stretch(offset_per_local);
        let local = -Vec3::from(*offset) / offset_per_local;
        let mut gizmo = RoundGizmo {
            target,
            command,
            shape,
            mesh_center,
            radius,
            turn,
            stretch,
            local,
            offset_per_local,
            center: Pos2::ZERO,
            arrows: [None; 3],
            rings: Default::default(),
            outline: Vec::new(),
            sizes: Vec::new(),
            steps: [None; 3],
        };
        let origin = gizmo.world(local);
        if !origin.is_finite() {
            return None;
        }

        gizmo.center = project(app, rect, origin)?;
        gizmo.move_and_turn_handles(app, rect, origin)?;

        // Keep offscreen handles reachable even if the pane is narrower than the handle
        let at = |point: Vec3| project(app, rect, mesh_center + turn * (stretch * point) * radius);
        let handle_area = (rect.size() - egui::Vec2::splat(2.0 * SIZE_ARROW)).max(egui::Vec2::ZERO);
        let view = Rect::from_center_size(rect.center(), handle_area);
        let half = (upper - lower) * 0.5 / radius;
        let forward = app.session.camera.forward();

        // The view direction in the projector's units: what faces the camera there
        let facing = (turn.transpose() * forward / stretch).try_normalize().unwrap_or(Vec3::Z);
        match shape {
            Shape::Box => gizmo.box_handles(at, half * BOX_REACH, forward, view),
            Shape::Sphere => gizmo.sphere_handles(at, half.max_element() * SPHERE_REACH, facing, view),
            Shape::Cylinder(axis) => gizmo.cylinder_handles(at, axis, half, facing, view),
        }
        Some(gizmo)
    }

    /// The arrows along the projector's turned directions and the rings that turn round them, each ring seen too near edge on left out (empty: neither drawn nor grabbed)
    fn move_and_turn_handles(&mut self, app: &TextureCraftApp, rect: Rect, origin: Vec3) -> Option<()> {
        let center = self.center;
        let directions = self.directions();
        let reach = self.radius * STEP_PROBE_RADII;
        self.steps = directions.map(|direction| Some((project(app, rect, origin + direction * reach)? - center) / reach));
        self.arrows = self.steps.map(|step| step.filter(|step| step.length() * reach > MIN_SCREEN_SPAN).map(|step| center + step.normalized() * ARROW));

        let right = app.session.camera.forward().cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
        let pixels = (project(app, rect, origin + right * reach)? - center).length() / reach;
        let ring = TURN_RING / pixels.max(f32::MIN_POSITIVE);
        self.rings = directions.map(|direction| {
            if !ring_open(app, rect, center, direction) {
                return Vec::new();
            }

            let (tangent, bitangent) = direction.any_orthonormal_pair();
            circle(|unit| project(app, rect, origin + (tangent * unit.x + bitangent * unit.y) * ring))
        });
        Some(())
    }

    /// A box of half size `size`: its edges, a dot on the corner up and right on screen that scales it evenly, and a double arrow on each face turned towards the camera that stretches it along that direction
    fn box_handles(&mut self, at: impl Fn(Vec3) -> Option<Pos2>, size: Vec3, forward: Vec3, view: Rect) {
        let local = self.local;
        let corner = |index: usize| local + size * box_corner(index);
        for (first_corner, second_corner) in box_edges() {
            if let (Some(from), Some(to)) = (at(corner(first_corner)), at(corner(second_corner))) {
                self.outline.push(vec![from, to]);
            }
        }

        let corners = (0..BOX_CORNERS).filter_map(|index| at(corner(index)));
        if let Some(top_right) = corners.max_by(|first, second| (first.x - first.y).total_cmp(&(second.x - second.y))) {
            self.sizes.push(SizeHandle { at: kept_in(view, top_right), axes: [true; 3], along: None });
        }
        for (axis, direction) in self.directions().iter().enumerate() {
            // The face toward the camera, on the other side in the projector's units when a mirrored axis flips it
            let facing_side = if direction.dot(forward) > 0.0 { -1.0 } else { 1.0 };
            let toward = facing_side * self.stretch[axis].signum();
            let Some(face) = at(local + Vec3::AXES[axis] * size * toward) else { continue };
            let out = face - self.center;
            if out.length() >= FACE_CLEARANCE {
                self.sizes.push(SizeHandle { at: kept_in(view, face), axes: only_axis(axis), along: Some(out.normalized()) });
            }
        }
    }

    /// A sphere of `radius`: its silhouette across `facing` and its equator, and one dot on its right side (a sphere only scales evenly)
    fn sphere_handles(&mut self, at: impl Fn(Vec3) -> Option<Pos2>, radius: f32, facing: Vec3, view: Rect) {
        let local = self.local;
        let (tangent, bitangent) = facing.any_orthonormal_pair();
        let silhouette = circle(|unit| at(local + (tangent * unit.x + bitangent * unit.y) * radius));
        if let Some(side) = silhouette.iter().copied().max_by(|first, second| first.x.total_cmp(&second.x)) {
            self.sizes.push(SizeHandle { at: kept_in(view, side), axes: [true; 3], along: None });
        }
        self.outline.push(silhouette);
        self.outline.push(circle(|unit| at(local + Vec3::new(unit.x, 0.0, unit.y) * radius)));
    }

    /// A cylinder round `axis` over a mesh of half sizes `half`: its caps and side lines, a double arrow on the top rim that stretches its height (seen end on, a dot) and a dot on its right side that scales it across (seen end on, on the rim's right)
    fn cylinder_handles(&mut self, at: impl Fn(Vec3) -> Option<Pos2>, axis: usize, half: Vec3, facing: Vec3, view: Rect) {
        let local = self.local;
        let along = Vec3::AXES[axis];
        let (tangent, bitangent) = (Vec3::AXES[(axis + 1) % 3], Vec3::AXES[(axis + 2) % 3]);
        let radius = (half * (Vec3::ONE - along)).max_element() * CYLINDER_REACH;
        let half_height = half[axis] * CYLINDER_HEIGHT;
        let cap = |height: f32| circle(|unit| at(local + along * height + (tangent * unit.x + bitangent * unit.y) * radius));
        let top = cap(half_height);
        let along_axis = match (at(local + along * half_height), at(local - along * half_height)) {
            (Some(top), Some(bottom)) => Some(top - bottom).filter(|span| span.length() > MIN_SCREEN_SPAN).map(egui::Vec2::normalized),
            _ => None,
        };
        if let Some(rim) = top.iter().copied().min_by(|first, second| first.y.total_cmp(&second.y)) {
            self.sizes.push(SizeHandle { at: kept_in(view, rim), axes: only_axis(axis), along: along_axis });
        }
        let right_of_rim = top.iter().copied().max_by(|first, second| first.x.total_cmp(&second.x));
        self.outline.extend([top, cap(-half_height)]);

        // End-on views show a rim handle because the two side lines coincide
        let side = along.cross(facing - along * facing.dot(along)).try_normalize();
        let mut sides = Vec::new();
        for edge in side.into_iter().flat_map(|side| [side, -side]) {
            if let (Some(top), Some(bottom)) = (at(local + edge * radius + along * half_height), at(local + edge * radius - along * half_height)) {
                self.outline.push(vec![top, bottom]);
                sides.push(top.lerp(bottom, 0.5));
            }
        }
        let across_at = match side {
            Some(_) => sides.into_iter().max_by(|first, second| first.x.total_cmp(&second.x)),
            None => right_of_rim,
        };
        if let Some(side) = across_at {
            let mut across = [true; 3];
            across[axis] = false;
            self.sizes.push(SizeHandle { at: kept_in(view, side), axes: across, along: None });
        }
    }

    /// A point in the projector's units, in the world
    fn world(&self, local: Vec3) -> Vec3 {
        self.mesh_center + self.turn * (self.stretch * local) * self.radius
    }

    /// The projector's turned X, Y and Z directions in the world
    fn directions(&self) -> [Vec3; 3] {
        Vec3::AXES.map(|axis| self.turn * axis)
    }

    /// The command that slides the projector as its centre square does
    pub(crate) fn slid(&self, app: &TextureCraftApp, rect: Rect, from: Pos2, to: Pos2) -> Option<(&'static str, Value)> {
        self.edit(app, rect, RoundGrab::Slide, from, to)
    }

    /// The `layer.fill.projection` params a drag of `grab` from `from` to `to` sets; only a size handle sets the scale, so a mirrored axis keeps its sign through a slide or turn
    fn params(&self, app: &TextureCraftApp, rect: Rect, grab: RoundGrab, from: Pos2, to: Pos2) -> Option<Value> {
        let origin = self.world(self.local);
        let (turn, stretch, center) = match grab {
            RoundGrab::Slide => {
                let forward = app.session.camera.forward();
                let moved = on_view_plane(app, rect, origin, forward, to)? - on_view_plane(app, rect, origin, forward, from)?;
                (self.turn, self.stretch, origin + moved)
            }
            RoundGrab::Axis(axis) => {
                let step = (*self.steps.get(axis)?)?;
                let distance = (to - from).dot(step) / step.length_sq().max(f32::MIN_POSITIVE);
                (self.turn, self.stretch, origin + self.directions()[axis] * distance)
            }
            RoundGrab::Ring(axis) => {
                let angle = (to - self.center).angle() - (from - self.center).angle();

                // Seen from the side its direction points to, a turn runs the other way round on screen
                let facing = self.directions()[axis].dot(app.session.camera.forward());
                let angle = if facing < 0.0 { -angle } else { angle };
                let turn = self.turn * Mat3::from_axis_angle(Vec3::AXES[axis], angle);
                (turn, self.stretch, origin)
            }
            RoundGrab::Size(axes) => {
                // Clamp the drag to the command's size limits so the offset keeps the same center
                let reach = from - self.center;
                let factor = ((to - self.center).dot(reach) / reach.length().max(MIN_HANDLE_REACH).powi(2)).max(0.0);
                let stretch = Vec3::from(axes.map(|on| if on { factor } else { 1.0 })) * self.stretch;
                (self.turn, stretch.map(signed_scale), origin)
            }
        };

        // The centre stays where it was put: the offset follows the turn and scale
        let local = turn.transpose() * ((center - self.mesh_center) / self.radius) / stretch;
        let offset = -local * self.offset_per_local;
        let (z, y, x) = Quat::from_mat3(&turn).to_euler(EulerRot::ZYX);
        let rotation = [x, y, z].map(f32::to_degrees);
        let values = [offset.to_array(), rotation, stretch.to_array()];
        if !values.iter().flatten().all(|value| value.is_finite()) {
            return None;
        }

        let mut params = self.target.target();
        params["offset"] = json!(offset.to_array());
        params["projectorRotation"] = json!(rotation);
        if matches!(grab, RoundGrab::Size(_)) {
            params["projectorScale"] = json!(stretch.to_array());
        }
        Some(params)
    }
}

impl Gizmo for RoundGizmo {
    type Grab = RoundGrab;

    fn drag_id() -> egui::Id {
        egui::Id::unique("tc_round_drag")
    }

    /// The handle under screen point `p`: the nearest one within reach (where rings cross, the nearest ring)
    fn grab(&self, at: Pos2) -> Option<RoundGrab> {
        let mut handles = vec![(RoundGrab::Slide, center_distance(self.center, at))];
        handles.extend((0..3).filter_map(|axis| Some((RoundGrab::Axis(axis), arrow_distance(at, self.center, self.arrows[axis]?)))));
        handles.extend(self.sizes.iter().map(|size| (RoundGrab::Size(size.axes), size.distance(at))));
        let distance = |ring: &Vec<Pos2>| ring.windows(2).map(|pair| distance_to_segment(at, pair[0], pair[1])).fold(f32::INFINITY, f32::min);
        handles.extend((0..3).map(|axis| (RoundGrab::Ring(axis), distance(&self.rings[axis]))));
        nearest(handles)
    }

    fn same_projection(&self, other: &RoundGizmo) -> bool {
        self.target == other.target && self.command == other.command && self.shape == other.shape
    }

    fn edit(&self, app: &TextureCraftApp, rect: Rect, grab: RoundGrab, from: Pos2, to: Pos2) -> Option<(&'static str, Value)> {
        Some((self.command, self.params(app, rect, grab, from, to)?))
    }

    /// A size handle's resize cursor runs along its arrow
    fn cursor(&self, grab: RoundGrab, dragging: bool) -> egui::CursorIcon {
        match grab {
            RoundGrab::Slide if dragging => egui::CursorIcon::Grabbing,
            RoundGrab::Slide => egui::CursorIcon::Grab,
            RoundGrab::Axis(_) | RoundGrab::Ring(_) => egui::CursorIcon::Move,
            RoundGrab::Size(axes) => self.sizes.iter().find(|size| size.axes == axes).map_or(egui::CursorIcon::ResizeNeSw, SizeHandle::cursor),
        }
    }
}

/// A size handle that stretches one way: a double arrow through `at` along `along`, `fill` over a dark halo like the axis arrows
fn size_arrow(painter: &egui::Painter, at: Pos2, along: egui::Vec2, fill: Color32) {
    let side = along.rot90() * SIZE_ARROW_HEAD_HALF_WIDTH;
    let shaft = [at - along * (SIZE_ARROW - SIZE_ARROW_HEAD), at + along * (SIZE_ARROW - SIZE_ARROW_HEAD)];
    painter.line_segment(shaft, Stroke::new(SHAFT_WIDTH + SHAFT_HALO_EXTRA_WIDTH, SHAFT_HALO));
    painter.line_segment(shaft, Stroke::new(SHAFT_WIDTH, fill));
    for direction in [along, -along] {
        let base = at + direction * (SIZE_ARROW - SIZE_ARROW_HEAD);
        painter.add(egui::Shape::convex_polygon(vec![at + direction * SIZE_ARROW, base + side, base - side], fill, HANDLE_EDGE));
    }
}

/// Corner `index` of a box from -1 to 1: the index's three low bits pick its low (0) or high (1) side along X, Y and Z
pub(crate) fn box_corner(index: usize) -> Vec3 {
    let side = |bit: usize| if index & bit == 0 { -1.0 } else { 1.0 };
    Vec3::new(side(BOX_AXIS_BITS[0]), side(BOX_AXIS_BITS[1]), side(BOX_AXIS_BITS[2]))
}

/// A box's twelve edges as pairs of corner indices, each once: from a corner on an axis's low side to the corner across that axis
pub(crate) fn box_edges() -> impl Iterator<Item = (usize, usize)> {
    (0..BOX_CORNERS).flat_map(|corner| BOX_AXIS_BITS.into_iter().filter(move |bit| corner & bit == 0).map(move |bit| (corner, corner | bit)))
}

/// Which projector directions a handle along `axis` alone works on
fn only_axis(axis: usize) -> [bool; 3] {
    let mut axes = [false; 3];
    axes[axis] = true;
    axes
}

/// `point` kept inside `view`, where a handle past its edge can still be grabbed
fn kept_in(view: Rect, point: Pos2) -> Pos2 {
    point.clamp(view.min, view.max)
}

/// A closed circle of screen points from `at` (given the unit circle's points); empty when a point is off screen
fn circle(at: impl Fn(Vec2) -> Option<Pos2>) -> Vec<Pos2> {
    (0..=CIRCLE_POINTS)
        .map(|index| {
            let angle = index as f32 / CIRCLE_POINTS as f32 * std::f32::consts::TAU;
            at(Vec2::new(angle.cos(), angle.sin()))
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default()
}

/// Where screen point `p` meets the plane facing the camera through `origin`
fn on_view_plane(app: &TextureCraftApp, rect: Rect, origin: Vec3, forward: Vec3, p: Pos2) -> Option<Vec3> {
    let ray = view_ray(app, rect, p);
    let facing = ray.dir.dot(forward);
    if facing.abs() < EDGE_ON_FACING {
        return None;
    }

    let hit = ray.origin + ray.dir * ((origin - ray.origin).dot(forward) / facing);
    hit.is_finite().then_some(hit)
}

/// The projector over the 3D view: dashed shape, rings, arrows, centre square and dots, the handle under the pointer lit
pub(crate) fn overlay(app: &TextureCraftApp, ui: &Ui, rect: Rect, gizmo: &RoundGizmo) {
    let tokens = Tokens::get(ui.ctx());
    let painter = ui.painter_at(rect);
    let active = dragging::<RoundGizmo>(ui.ctx()).map(|drag| drag.grab);

    // A held Move on model button lights none
    let hovered = ui.ctx().pointer_hover_pos().filter(|hover| rect.contains(*hover) && app.moving_on_model.is_none()).and_then(|hover| gizmo.grab(hover));
    let lit = |grab: RoundGrab| active == Some(grab) || (active.is_none() && hovered == Some(grab));
    let fill = |grab: RoundGrab| if lit(grab) { tokens.viewport_accent } else { Color32::WHITE };
    let axis_color = |axis: usize, grab: RoundGrab| if lit(grab) { tokens.viewport_accent } else { AXIS_COLORS[axis] };
    for line in &gizmo.outline {
        dashed_outline(&painter, line);
    }
    for (axis, ring) in gizmo.rings.iter().enumerate() {
        painter.add(egui::Shape::line(ring.clone(), Stroke::new(SHAFT_WIDTH + SHAFT_HALO_EXTRA_WIDTH, SHAFT_HALO)));
        painter.add(egui::Shape::line(ring.clone(), Stroke::new(SHAFT_WIDTH, axis_color(axis, RoundGrab::Ring(axis)))));
    }
    for size in &gizmo.sizes {
        let color = fill(RoundGrab::Size(size.axes));
        match size.along {
            Some(along) => size_arrow(&painter, size.at, along, color),
            None => handle_dot(&painter, size.at, SIZE_DOT, color),
        }
    }
    move_handles(&painter, gizmo.center, gizmo.arrows, |axis| axis_color(axis, RoundGrab::Axis(axis)), fill(RoundGrab::Slide));
}

#[cfg(test)]
pub(crate) mod tests {
    use egui::{Modifiers, PointerButton, vec2};
    use egui_kittest::kittest::Queryable;
    use texturecraft_engine::core::{Fill, LayerId, LayerKind, Projection, Projector};

    use super::*;
    use crate::frame_gizmo::tests::{ball_app, drag, drag_frames, fill_app, fill_app_with, panel_xyz, press_and_move, projection};

    fn gizmo(harness: &egui_kittest::Harness<'_, TextureCraftApp>) -> RoundGizmo {
        RoundGizmo::of(harness.state(), harness.state().viewport_rect).unwrap()
    }

    fn projector_and_offset(projection: &Projection) -> (Projector, [f32; 3]) {
        match projection {
            Projection::Triplanar { projector, offset, .. }
            | Projection::Spherical { projector, offset, .. }
            | Projection::Cylindrical { projector, offset, .. } => (*projector, *offset),
            other => panic!("a round projection: {other:?}"),
        }
    }

    #[test]
    fn spherical_size_handle_stays_inside_a_narrow_view() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (harness, _) = fill_app("spherical");
        let app = harness.state();
        let narrow_view = Rect::from_center_size(app.viewport_rect.center(), vec2(SIZE_ARROW, app.viewport_rect.height()));

        let projector = RoundGizmo::of(app, narrow_view).expect("spherical projector in the narrow view");

        assert!(projector.sizes.iter().any(|handle| narrow_view.contains(handle.at)));
    }

    #[test]
    fn dragging_a_round_mask_fill_moves_the_effect() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("uv");
        let effect = harness.state_mut().run("effect.add.fill", json!({"layer": layer.0, "target": "mask", "projection": "spherical"})).unwrap();
        let effect_id = LayerId(effect["effect"].as_u64().unwrap());
        harness.run_steps(2);
        let center = gizmo(&harness).center;

        drag(&mut harness, center, center + vec2(40.0, 0.0), Modifiers::NONE);

        let effect = &harness.state().session.project().effect(effect_id).unwrap().1.kind;
        let texturecraft_engine::core::EffectKind::Fill { projection, .. } = effect else { panic!("mask fill effect") };

        assert_ne!(projector_and_offset(projection).1, [0.0; 3]);
    }

    #[test]
    fn a_stale_effect_target_cannot_show_a_projector_on_another_layer() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, other_layer) = fill_app("uv");
        let app = harness.state_mut();
        let layer = app.run("layer.add.fill", json!({})).unwrap()["layer"].as_u64().unwrap();
        let effect = app.run("effect.add.fill", json!({"layer": layer, "target": "content", "projection": "spherical"})).unwrap();
        let effect_id = effect["effect"].as_u64().unwrap();
        let view = app.viewport_rect;

        assert!(RoundGizmo::of(app, view).is_some());

        let selection = texturecraft_engine::execute(&mut app.session, "layer.select", &json!({"layer": other_layer.0, "target": effect_id}));

        assert!(selection.is_err());
        assert!(RoundGizmo::of(app, view).is_none());
    }

    #[test]
    fn round_projections_show_their_projector_while_their_fill_is_selected() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for kind in ["triplanar", "spherical", "cylindrical"] {
            let (mut harness, _) = fill_app(kind);
            harness.run_steps(2);
            let shown = gizmo(&harness);

            assert!(shown.arrows.iter().all(Option::is_some) && shown.rings.iter().all(|ring| ring.len() > 8), "{kind}: arrows and rings {shown:?}");
            assert!(!shown.outline.is_empty(), "{kind}: its shape is drawn");

            // The oblique view exposes the box's face arrows and the cylinder's top and side handles
            let size_handles = match kind {
                "triplanar" => 4,
                "spherical" => 1,
                _ => 2,
            };
            assert_eq!(shown.sizes.len(), size_handles, "{kind}: size handles {:?}", shown.sizes);
        }
        let (harness, _) = fill_app("planar");
        assert!(RoundGizmo::of(harness.state(), harness.state().viewport_rect).is_none(), "planar projections show their frame instead");
    }

    #[test]
    fn a_cylinder_axis_past_z_shows_round_y_as_the_compositor_wraps_it() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app_with(json!({"projection": "cylindrical", "axis": 1}));
        let round_y = gizmo(&harness).outline;

        // A hand-edited project can store an axis past Z: no command writes one
        let stored = &mut harness.state_mut().session.doc.project.layer_mut(layer).unwrap().kind;
        let LayerKind::Fill(Fill { projection: Projection::Cylindrical { axis, .. }, .. }) = stored else { panic!("a cylindrical fill") };
        *axis = 3;

        assert_eq!(gizmo(&harness).outline, round_y);
    }

    #[test]
    fn the_arrows_move_the_rings_turn_and_the_dots_scale_the_projector_one_step_per_drag() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("spherical");
        let steps = harness.state().session.history_labels().len();

        let start = gizmo(&harness);
        let tip = start.arrows[0].unwrap();
        drag(&mut harness, tip, tip + (tip - start.center).normalized() * 30.0, Modifiers::NONE);
        let (projector, offset) = projector_and_offset(&projection(&harness, layer));

        assert!(offset[0] < -0.05 && offset[1].abs() < 1e-3 && offset[2].abs() < 1e-3, "the X arrow moves the centre along X: {offset:?}");
        assert_eq!(projector.rotation, [0.0; 3]);

        let moved = gizmo(&harness);
        assert!(moved.center.x > start.center.x + 20.0, "the centre follows: {moved:?}");

        // The Z ring (seen face on) turns it round Z; the centre stays where it is
        let clear = |p: &&Pos2| moved.rings[..2].iter().flatten().map(|other| other.distance(**p)).fold(f32::INFINITY, f32::min);
        let on_ring = *moved.rings[2].iter().max_by(|a, b| clear(a).total_cmp(&clear(b))).unwrap();

        assert_eq!(moved.grab(on_ring), Some(RoundGrab::Ring(2)));

        drag(&mut harness, on_ring, moved.center + (on_ring - moved.center).rot90(), Modifiers::NONE);
        let (turned, _) = projector_and_offset(&projection(&harness, layer));
        assert!(turned.rotation[2].abs() > 30.0 && turned.rotation[0].abs() < 1e-2, "{:?}", turned.rotation);
        assert!(gizmo(&harness).center.distance(moved.center) < 1.0, "turning keeps the centre");

        let scaling = gizmo(&harness);
        let dot = scaling.sizes.iter().find(|size| size.axes == [true; 3]).expect("the even-scale dot").at;
        let center = scaling.center;
        assert_eq!(gizmo(&harness).grab(dot), Some(RoundGrab::Size([true; 3])));

        drag(&mut harness, dot, center + (dot - center) * 1.5, Modifiers::NONE);
        let (scaled, _) = projector_and_offset(&projection(&harness, layer));
        assert!(scaled.scale.iter().all(|scale| (scale - 1.5).abs() < 0.05), "the side dot scales it evenly: {:?}", scaled.scale);
        assert!(gizmo(&harness).center.distance(center) < 1.0, "scaling keeps the centre");
        assert_eq!(harness.state().session.history_labels().len(), steps + 3, "one undo step per drag");
    }

    #[test]
    fn a_cylinders_top_dot_stretches_its_height_and_a_tri_planar_box_moves_by_its_texture_offset() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("cylindrical");
        let cylinder = gizmo(&harness);
        let height = cylinder.sizes.iter().find(|size| size.axes == [false, true, false]).expect("the height handle");
        let SizeHandle { at: top, axes, along } = *height;

        assert_eq!(axes, [false, true, false], "the top handle works along the axis");

        let along = along.expect("the height handle is a double arrow");
        assert!(along.y.abs() > 0.99, "drawn upright, along the upright axis: {along:?}");
        assert_eq!(cylinder.cursor(RoundGrab::Size(axes), false), egui::CursorIcon::ResizeVertical);

        drag(&mut harness, top, cylinder.center + (top - cylinder.center) * 0.5, Modifiers::NONE);
        let (projector, _) = projector_and_offset(&projection(&harness, layer));
        assert!(projector.scale[1] < 0.7 && projector.scale[0] == 1.0 && projector.scale[2] == 1.0, "{:?}", projector.scale);

        let (mut harness, layer) = fill_app("triplanar");
        harness.state_mut().run("layer.fill.projection", json!({"layer": layer.0, "scale": 4})).unwrap();
        harness.run_steps(2);
        let box_gizmo = gizmo(&harness);
        drag(&mut harness, box_gizmo.center, box_gizmo.center + vec2(0.0, -30.0), Modifiers::NONE);
        let (_, offset) = projector_and_offset(&projection(&harness, layer));
        assert!(offset[1] < -0.1 && offset[0].abs() < 0.05, "sliding up shifts the texture by the tiling: {offset:?}");
        assert!(gizmo(&harness).center.distance(box_gizmo.center + vec2(0.0, -30.0)) < 1.5, "the centre sits under the pointer");
    }

    /// Every drag on the projector shows in the panel the same frame: its Offset, Rotation and Scale fields hold the projector's values and the preview square re-renders, every frame of the drag; one undo step per drag
    #[test]
    fn the_panel_and_its_preview_follow_every_frame_of_a_projector_drag() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for kind in ["spherical", "cylindrical"] {
            let (mut harness, layer) = fill_app(kind);
            let steps = harness.state().session.history_labels().len();
            let read = |harness: &egui_kittest::Harness<'_, TextureCraftApp>| {
                let (projector, offset) = projector_and_offset(&projection(harness, layer));
                let (row_offset, row_rotation, row_scale) = (panel_xyz(harness, "Offset"), panel_xyz(harness, "Rotation"), panel_xyz(harness, "Scale"));
                for (row, field, value) in
                    [("Offset", row_offset, offset), ("Rotation", row_rotation, projector.rotation), ("Scale", row_scale, projector.scale)]
                {
                    for axis in 0..3 {
                        assert!((field[axis] - value[axis]).abs() < 1e-3, "{kind}: the panel's {row} shows the projector's {value:?}, not {field:?}");
                    }
                }
            };

            // A sphere's one dot scales it evenly, a cylinder's height arrow stretches it along its axis
            let start = gizmo(&harness);
            let axes = if kind == "spherical" { [true; 3] } else { [false, true, false] };
            let dot = start.sizes.iter().find(|size| size.axes == axes).unwrap_or_else(|| panic!("{kind}: its size handle {axes:?}")).at;
            drag_frames(&mut harness, layer, dot, start.center + (dot - start.center) * 1.5, Modifiers::NONE, true, read);
            let tip = gizmo(&harness).arrows[0].unwrap();
            let center = gizmo(&harness).center;
            drag_frames(&mut harness, layer, tip, tip + (tip - center).normalized() * 30.0, Modifiers::NONE, true, read);
            let turned = gizmo(&harness);
            let clear = |p: &&Pos2| turned.rings[..2].iter().flatten().map(|other| other.distance(**p)).fold(f32::INFINITY, f32::min);
            let on_ring = *turned.rings[2].iter().max_by(|a, b| clear(a).total_cmp(&clear(b))).unwrap();
            drag_frames(&mut harness, layer, on_ring, turned.center + (on_ring - turned.center).rot90(), Modifiers::NONE, true, read);
            let (projector, _) = projector_and_offset(&projection(&harness, layer));
            assert!(
                projector.scale.iter().copied().fold(0.0, f32::max) > 1.3 && projector.rotation[2].abs() > 30.0,
                "{kind}: resized and turned: {projector:?}"
            );
            assert_eq!(harness.state().session.history_labels().len(), steps + 3, "{kind}: one undo step per drag");
        }
    }

    /// Whether a projector drag is under way
    fn round_dragging(harness: &egui_kittest::Harness<'_, TextureCraftApp>) -> bool {
        dragging::<RoundGizmo>(&harness.ctx).is_some()
    }

    /// Losing the focus mid-drag (Alt+Tab) ends a projector drag as a release does: one undo step, and moves after it (egui still has the button down) leave the projector alone
    #[test]
    fn losing_the_window_focus_ends_a_projector_drag() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("spherical");
        let steps = harness.state().session.history_labels().len();
        let center = gizmo(&harness).center;
        let to = center + vec2(40.0, 0.0);
        press_and_move(&mut harness, center, to, Modifiers::NONE);

        assert!(round_dragging(&harness), "the centre square is held");

        harness.event(egui::Event::WindowFocused(false));
        harness.run_steps(1);
        let left = projection(&harness, layer);
        assert!(!round_dragging(&harness), "the focus loss ends the drag");
        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "as one undo step");

        for step in 1..=3 {
            harness.event(egui::Event::PointerMoved(to + vec2(20.0, 10.0) * step as f32));
            harness.run_steps(1);
        }

        assert_eq!(projection(&harness, layer), left, "moves after it leave the projector where it was");
    }

    /// Alt pressed mid-drag hands the pointer to the camera and ends the projector drag where it is: let go of Alt with the button still down, the drag does not resume; one undo step
    #[test]
    fn alt_mid_drag_ends_a_projector_drag() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("spherical");
        let steps = harness.state().session.history_labels().len();
        let center = gizmo(&harness).center;
        let to = center + vec2(40.0, 0.0);
        press_and_move(&mut harness, center, to, Modifiers::NONE);
        let slid = projection(&harness, layer);
        let camera = harness.state().session.camera;

        harness.event_modifiers(egui::Event::PointerMoved(to + vec2(20.0, 0.0)), Modifiers::ALT);
        harness.run_steps(1);

        assert_ne!(harness.state().session.camera, camera, "Alt turns the camera");
        assert!(!round_dragging(&harness), "and ends the projector drag");

        for step in 2..=4 {
            harness.event(egui::Event::PointerMoved(to + vec2(20.0, 5.0) * step as f32));
            harness.run_steps(1);
        }

        assert_eq!(projection(&harness, layer), slid, "Alt let go, the projector stays where the drag left it");

        harness.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        harness.run_steps(2);

        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "one undo step");
    }

    /// Seen edge on a ring is a line, whose screen-angle turn would jump half a turn across the centre: from straight in front (no yaw) the X ring is neither drawn nor grabbed while the Y and Z rings stay; turned aside, it opens and shows again
    #[test]
    fn a_ring_seen_edge_on_is_neither_drawn_nor_grabbed() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, _) = fill_app("spherical");
        harness.state_mut().session.camera.yaw = 0.0;
        harness.run_steps(2);

        let front = gizmo(&harness);

        assert!(front.rings[0].is_empty(), "the X ring seen edge on is left out: {:?}", front.rings[0]);
        assert!(front.rings[1..].iter().all(|ring| !ring.is_empty()), "the Y and Z rings show");

        let on_the_line = front.center + vec2(0.0, 40.0);
        assert_ne!(front.grab(on_the_line), Some(RoundGrab::Ring(0)), "a press where its line would be does not grab it");

        harness.state_mut().session.camera.yaw = 0.5;
        harness.run_steps(2);

        assert!(gizmo(&harness).rings.iter().all(|ring| !ring.is_empty()), "turned aside, every ring shows");
    }

    /// A paint layer on the ball whose mask holds a fill effect (selected) projecting `kind`: painting there paints the mask
    fn mask_fill_app(kind: &str) -> (egui_kittest::Harness<'static, TextureCraftApp>, LayerId) {
        let mut harness = ball_app(false);
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let image = harness.state_mut().session.import_resource_bytes("decal.png", png).unwrap();
        harness.state_mut().run("layer.add.decal", json!({"image": image, "channel": "mask", "size": 0.5})).unwrap();
        let texturecraft_engine::StackTarget::Effect(effect) = harness.state().session.target else { panic!("the mask fill is selected") };
        harness.state_mut().run("layer.fill.projection", json!({"effect": effect.0, "projection": kind})).unwrap();
        harness.run_steps(2);
        (harness, effect)
    }

    fn effect_projection(harness: &egui_kittest::Harness<'_, TextureCraftApp>, effect: LayerId) -> Projection {
        match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
            texturecraft_engine::core::EffectKind::Fill { projection, .. } => projection.clone(),
            other => panic!("a fill effect: {other:?}"),
        }
    }

    /// A point on the model clear of every handle of the shown gizmo
    pub(crate) fn on_the_model_off_the_handles(harness: &egui_kittest::Harness<'_, TextureCraftApp>) -> Pos2 {
        let app = harness.state();
        let rect = app.viewport_rect;
        let round = RoundGizmo::of(app, rect);
        let frame = crate::frame_gizmo::FrameGizmo::of(app, rect);
        let clear = |p: Pos2| {
            round.as_ref().is_none_or(|gizmo| gizmo.grab(p).is_none())
                && frame.as_ref().is_none_or(|gizmo| gizmo.grab(p).is_none())
                && app.session.pick(texturecraft_engine::geom::glam::Vec2::new(p.x - rect.left(), p.y - rect.top()), app.session.viewport).is_some()
        };
        (1..12)
            .flat_map(|ring| (0..16).map(move |step| rect.center() + egui::Vec2::angled(step as f32 / 16.0 * std::f32::consts::TAU) * (ring as f32 * 14.0)))
            .find(|p| clear(*p) && clear(*p + vec2(30.0, 0.0)))
            .expect("the model shows past the handles")
    }

    /// With every projection, a drag on the model away from the gizmo leaves the projection put (and paints nothing: no tool acts on a selected fill effect); only a drag that starts on a handle moves the projection
    #[test]
    fn only_a_handle_moves_the_projection_and_any_other_drag_on_the_model_leaves_it() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for kind in ["planar", "warp", "triplanar", "spherical", "cylindrical"] {
            let (mut harness, effect) = mask_fill_app(kind);
            let rect = harness.state().viewport_rect;
            let before = effect_projection(&harness, effect);
            let steps = harness.state().session.history_labels().len();

            let from = on_the_model_off_the_handles(&harness);
            drag(&mut harness, from, from + vec2(30.0, 0.0), Modifiers::NONE);

            assert_eq!(effect_projection(&harness, effect), before, "{kind}: a drag off the handles leaves the projection alone");
            assert_eq!(harness.state().session.history_labels().len(), steps, "{kind}: and adds no undo step");

            let center = match RoundGizmo::of(harness.state(), rect) {
                Some(round) => round.center,
                None => crate::frame_gizmo::FrameGizmo::of(harness.state(), rect).unwrap_or_else(|| panic!("{kind}: a gizmo shows")).center,
            };
            drag(&mut harness, center, center + vec2(0.0, -30.0), Modifiers::NONE);

            assert_ne!(effect_projection(&harness, effect), before, "{kind}: the centre square moves it");
            assert_eq!(harness.state().session.history_labels().len(), steps + 1, "{kind}: one step, no stroke");
        }
    }

    /// A sphere has one size handle (it only scales evenly); a tri-planar box scales evenly from its corner and along one direction from a face's double arrow
    #[test]
    fn a_sphere_scales_from_one_dot_and_a_tri_planar_box_from_its_corner_and_faces() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("spherical");
        let start = gizmo(&harness);

        assert_eq!(start.sizes.len(), 1, "one dot: {:?}", start.sizes);

        let dot = start.sizes[0].at;
        assert_eq!(start.grab(dot), Some(RoundGrab::Size([true; 3])));

        drag(&mut harness, dot, start.center + (dot - start.center) * 1.4, Modifiers::NONE);
        let (scaled, _) = projector_and_offset(&projection(&harness, layer));
        assert!(scaled.scale.iter().all(|scale| (scale - 1.4).abs() < 0.08), "it scales evenly: {:?}", scaled.scale);

        let (mut harness, layer) = fill_app("triplanar");
        let start = gizmo(&harness);
        let corner = start.sizes.iter().find(|size| size.axes == [true; 3]).expect("a corner dot").at;
        drag(&mut harness, corner, start.center + (corner - start.center) * 1.25, Modifiers::NONE);
        let (scaled, _) = projector_and_offset(&projection(&harness, layer));
        assert!(scaled.scale.iter().all(|scale| (scale - 1.25).abs() < 0.05), "the corner scales it evenly: {:?}", scaled.scale);
        assert!(gizmo(&harness).center.distance(start.center) < 1.0, "scaling keeps the centre");

        let box_gizmo = gizmo(&harness);
        let across = *box_gizmo.sizes.iter().find(|size| size.axes == [true, false, false]).expect("an X face arrow");
        assert!(across.along.is_some_and(|along| along.x.abs() > 0.95), "it points across: {across:?}");
        assert_eq!(
            box_gizmo.grab(across.at + across.along.unwrap_or_default() * (SIZE_ARROW - 1.0)),
            Some(RoundGrab::Size([true, false, false])),
            "grabbed from its tip"
        );

        drag(&mut harness, across.at, box_gizmo.center + (across.at - box_gizmo.center) * 0.5, Modifiers::NONE);
        let (stretched, _) = projector_and_offset(&projection(&harness, layer));
        assert!((stretched.scale[0] - scaled.scale[0] * 0.5).abs() < 0.05, "the X face halves it across: {:?}", stretched.scale);
        assert!(
            (stretched.scale[1] - scaled.scale[1]).abs() < 1e-4 && (stretched.scale[2] - scaled.scale[2]).abs() < 1e-4,
            "and only across: {:?}",
            stretched.scale
        );
    }

    /// A projector a project file mirrors along X (a negative scale, as the projection reads it) reads mirrored in the gizmo and stays mirrored through a slide, a move along an arrow, a turn and a resize
    #[test]
    fn a_mirrored_projector_keeps_its_mirror_through_every_handle() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("spherical");
        let mirrored = [-1.0, 1.0, 1.0];
        match &mut harness.state_mut().session.doc.project.layer_mut(layer).unwrap().kind {
            texturecraft_engine::core::LayerKind::Fill(fill) => match &mut fill.projection {
                Projection::Spherical { projector, .. } => projector.scale = mirrored,
                other => panic!("a spherical projection: {other:?}"),
            },
            other => panic!("a fill layer: {other:?}"),
        }
        harness.run_steps(2);

        let start = gizmo(&harness);

        assert_eq!(start.stretch, Vec3::from(mirrored), "the gizmo reads the mirror as the projection does");

        drag(&mut harness, start.center, start.center + vec2(30.0, -20.0), Modifiers::NONE);
        let (slid, offset) = projector_and_offset(&projection(&harness, layer));
        assert_eq!(slid.scale, mirrored, "a slide keeps the mirror");
        assert!(offset.iter().any(|axis| axis.abs() > 0.05), "and slides it: {offset:?}");

        let tip = gizmo(&harness).arrows[1].unwrap();
        let center = gizmo(&harness).center;
        drag(&mut harness, tip, tip + (tip - center).normalized() * 20.0, Modifiers::NONE);

        assert_eq!(projector_and_offset(&projection(&harness, layer)).0.scale, mirrored, "so does a move along an arrow");

        let before_turn = projector_and_offset(&projection(&harness, layer)).0;
        let ring = gizmo(&harness).rings[2].clone();
        drag(&mut harness, ring[CIRCLE_POINTS / 8], ring[CIRCLE_POINTS / 8 + 3], Modifiers::NONE);
        let turned = projector_and_offset(&projection(&harness, layer)).0;
        assert_ne!(turned.rotation, before_turn.rotation, "the ring turns it");
        assert_eq!(turned.scale, mirrored, "and keeps the mirror");

        let resized = gizmo(&harness);
        let dot = resized.sizes.iter().find(|size| size.axes == [true; 3]).expect("the even-scale dot").at;
        drag(&mut harness, dot, resized.center + (dot - resized.center) * 1.5, Modifiers::NONE);
        let scale = projector_and_offset(&projection(&harness, layer)).0.scale;
        let expected = [-1.5, 1.5, 1.5];
        assert!(scale.iter().zip(expected).all(|(axis, wanted)| (axis - wanted).abs() < 1e-3), "a resize scales it evenly, still mirrored: {scale:?}");
    }

    /// A box mirrored along X (a negative scale from a project file) keeps its X double arrow on the face toward the camera, where the unmirrored box shows it
    #[test]
    fn a_mirrored_box_keeps_its_face_arrow_toward_the_camera() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("triplanar");
        let x_face_arrow = |harness: &egui_kittest::Harness<'_, TextureCraftApp>| {
            gizmo(harness).sizes.iter().find(|size| size.axes == [true, false, false]).map(|size| size.at)
        };
        let unmirrored = x_face_arrow(&harness).expect("an X face arrow");
        match &mut harness.state_mut().session.doc.project.layer_mut(layer).unwrap().kind {
            LayerKind::Fill(fill) => match &mut fill.projection {
                Projection::Triplanar { projector, .. } => projector.scale = [-1.0, 1.0, 1.0],
                other => panic!("a tri-planar projection: {other:?}"),
            },
            other => panic!("a fill layer: {other:?}"),
        }
        harness.run_steps(2);

        let mirrored = x_face_arrow(&harness).expect("the mirrored box's X face arrow");

        assert!(mirrored.distance(unmirrored) < 0.5, "the arrow stays on the near face: {mirrored:?}, unmirrored {unmirrored:?}");
    }

    /// The pointer over a handle shows what it does only while the gizmo takes input: with Alt held the camera orbits, and the handle's cursor is not shown
    #[test]
    fn alt_over_a_handle_shows_no_handle_cursor() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, _) = fill_app("triplanar");
        let center = gizmo(&harness).center;
        harness.hover_at(center);
        harness.run_steps(2);

        assert_eq!(harness.output().platform_output.cursor_icon, egui::CursorIcon::Grab, "the centre square shows it slides");

        harness.event(egui::Event::ModifiersChanged(Modifiers::ALT));
        harness.event(egui::Event::PointerMoved(center + vec2(1.0, 0.0)));
        harness.run_steps(2);

        assert_ne!(harness.output().platform_output.cursor_icon, egui::CursorIcon::Grab, "with Alt held it does not");

        harness.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        harness.run_steps(2);
    }

    /// A plain left press on the projector's handle while a colour picker is open only closes the picker: no stroke, no slide, and the press after it works again
    #[test]
    fn a_press_that_closes_the_colour_picker_neither_paints_nor_slides() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("triplanar");
        let before = projection(&harness, layer);
        let from = gizmo(&harness).center;
        harness.get_by_role_and_label(egui::Role::Tab, "Material").click();
        harness.run_steps(25);
        harness.get_by_role_and_label(egui::Role::Button, "Base color").click();
        harness.run_steps(2);

        assert!(crate::color_picker::open_window(&harness.ctx).is_some(), "the picker is open");

        let steps = harness.state().session.history_labels().len();

        drag(&mut harness, from, from + vec2(40.0, 0.0), Modifiers::NONE);

        assert!(crate::color_picker::open_window(&harness.ctx).is_none(), "the press closed the picker");
        assert_eq!(projection(&harness, layer), before, "and slid nothing");
        assert_eq!(harness.state().session.history_labels().len(), steps, "nor made an undo step");

        drag(&mut harness, from, from + vec2(40.0, 0.0), Modifiers::NONE);

        assert_ne!(projection(&harness, layer), before, "the next press slides again");
    }

    /// A tri-planar fill selected while the quick mask is edited with the Paint tool: brushes act and the projector shows
    fn quick_mask_paint_app() -> (egui_kittest::Harness<'static, TextureCraftApp>, LayerId) {
        let (mut harness, layer) = fill_app("triplanar");
        harness.state_mut().run("viewport.quickMask", json!({"enabled": true})).unwrap();
        harness.state_mut().run("viewport.quickMask.edit", json!({"editing": true})).unwrap();
        harness.state_mut().session.tool.tool = texturecraft_engine::Tool::Paint;
        harness.run_steps(2);
        assert!(gizmo(&harness).sizes.len() > 1, "the projector shows");
        (harness, layer)
    }

    /// A Ctrl+drag that starts on a handle changes the brush only: the projector stays where it is
    #[test]
    fn a_ctrl_drag_on_a_handle_changes_the_brush_and_not_the_projection() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = quick_mask_paint_app();
        let before = projection(&harness, layer);
        let flow = harness.state().session.tool.brush.flow;
        let center = gizmo(&harness).center;

        harness.event(egui::Event::ModifiersChanged(Modifiers::COMMAND));
        drag(&mut harness, center, center + vec2(-40.0, 0.0), Modifiers::COMMAND);
        harness.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        harness.run_steps(2);

        assert_ne!(harness.state().session.tool.brush.flow, flow, "the drag changes the brush");
        assert_eq!(projection(&harness, layer), before, "and leaves the projection");
    }

    /// A press on a handle's edge that drifts just off it before release (still a click) is the handle's: it paints no dab
    #[test]
    fn a_click_that_drifts_off_a_handle_paints_nothing() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, _) = quick_mask_paint_app();
        let shown = gizmo(&harness);
        let rect = harness.state().viewport_rect;
        let on_model = |p: Pos2| {
            harness
                .state()
                .session
                .pick(texturecraft_engine::geom::glam::Vec2::new(p.x - rect.left(), p.y - rect.top()), harness.state().session.viewport)
                .is_some()
        };
        let drift = 2.0;

        // Find the last center-handle point before a small move lands on the model
        let (edge, off) = (0..16)
            .filter_map(|step| {
                let toward = egui::Vec2::angled(step as f32 / 16.0 * std::f32::consts::TAU);
                let edge = (0..200).map(|half| shown.center + toward * (half as f32 * 0.5)).take_while(|p| shown.grab(*p).is_some()).last()?;
                let off = edge + toward * drift;
                (shown.grab(off).is_none() && on_model(off)).then_some((edge, off))
            })
            .next()
            .expect("a handle edge on the model");
        let click = |harness: &mut egui_kittest::Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2| {
            harness.hover_at(from);
            harness.run_steps(1);
            harness.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            harness.run_steps(1);
            harness.event(egui::Event::PointerMoved(to));
            harness.run_steps(1);
            harness.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            harness.run_steps(2);
        };

        let painted = harness.state().session.quick_mask.revision;
        click(&mut harness, edge, off);

        assert_eq!(harness.state().session.quick_mask.revision, painted, "a press that began on the handle paints no dab");

        // The same click a little past the handle paints
        click(&mut harness, off, off + (off - edge));

        assert_ne!(harness.state().session.quick_mask.revision, painted, "a press off the handle paints");
    }

    /// A cylinder seen straight down its axis keeps the dot that scales it across, on its rim's right
    #[test]
    fn a_cylinder_seen_end_on_keeps_its_across_dot() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app_with(json!({"projection": "cylindrical", "axis": 2}));
        harness.state_mut().session.camera.yaw = 0.0;
        harness.state_mut().session.camera.pitch = 0.0;
        harness.run_steps(2);

        let end_on = gizmo(&harness);
        let across = [true, true, false];
        let dot = end_on.sizes.iter().find(|size| size.axes == across).unwrap_or_else(|| panic!("the across dot: {:?}", end_on.sizes)).at;

        assert_eq!(end_on.grab(dot), Some(RoundGrab::Size(across)));

        drag(&mut harness, dot, end_on.center + (dot - end_on.center) * 1.5, Modifiers::NONE);
        let (projector, _) = projector_and_offset(&projection(&harness, layer));
        assert!(projector.scale[0] > 1.2 && projector.scale[1] > 1.2 && projector.scale[2] == 1.0, "it scales across only: {:?}", projector.scale);
    }
}
