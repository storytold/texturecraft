//! A fill's Properties: its projection, size, placement and edges, then its material

use std::ops::RangeInclusive;

use egui::{Painter, Rect};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::compose::frame_placement;
use texturecraft_engine::compose::tile_length::{MeshSize, tile_length, tiling_for_length};
use texturecraft_engine::core::{Backface, Channel, Fill, Filtering, LayerId, Projection, Projector, ScaleMode, ShapeCrop, UvWrap};
use texturecraft_engine::layer_cmds::{MAX_PLACEMENT, MIN_SCALE};
use texturecraft_engine::paint::FULL_TURN_DEGREES;

use crate::TextureCraftApp;
use crate::dock::with;
use crate::fill_pictures::{
    angle_picture, backface_picture, crop_picture, depth_picture, filtering_picture, projection_picture, warp_preview, window_preview, wrap_picture,
};
use crate::i18n::{filtering_label, n_, tr, wrap_label};
use crate::move_on_model::MoveOnModel;
use crate::placement::Placement;
use crate::projection_kind::ProjectionKind;
use crate::projection_preview::projection_preview;
use crate::property_sheet::{NumberField, PreviewLower, PreviewSize, PropertySheet, RowLead};
use crate::theme::{AXIS_COLORS, AXIS_NAMES, Tokens};

/// Centimetres in one mesh unit: meshes are read in metres
const CM_PER_UNIT: f32 = 100.0;

/// What the Tiling track spans
const TILING_TRACK: RangeInclusive<f32> = 0.01..=5.0;

/// What a typed tiling may be: the command's own limits
const TILING_TYPED: RangeInclusive<f32> = MIN_SCALE..=MAX_PLACEMENT;

/// What the Size (cm) track spans
const SIZE_CM_TRACK: RangeInclusive<f32> = 1.0..=500.0;

/// What a typed size (cm) may be
const SIZE_CM_TYPED: RangeInclusive<f32> = 0.1..=100_000.0;

/// What an Offset track spans (tiles, centred on zero)
const OFFSET_TRACK: RangeInclusive<f32> = -2.0..=2.0;

/// What a typed offset may be (tiles): the command's own limits
const OFFSET_TYPED: RangeInclusive<f32> = -MAX_PLACEMENT..=MAX_PLACEMENT;

/// Range of a fill's rotation, a full turn
const ROTATION_RANGE: RangeInclusive<f32> = 0.0..=FULL_TURN_DEGREES;

/// Tri-planar blend's largest exponent: Hardness 1 (0 = exponent 1, 0.5 = 8)
const TRIPLANAR_SHARPNESS_MAX: f32 = *Projection::TRIPLANAR_SHARPNESS_RANGE.end();

/// Range of every Hardness the panel shows
const HARDNESS_RANGE: RangeInclusive<f32> = 0.0..=1.0;

/// What a cylindrical projection's Angle may be: the arc one repeat spans, in degrees
const CYLINDER_ANGLE_RANGE: RangeInclusive<f32> = Projection::CYLINDER_ANGLE_RANGE;

/// What Backface culling's Angle may be, in degrees from facing the projector
const BACKFACE_ANGLE_RANGE: RangeInclusive<f32> = Backface::ANGLE_RANGE;

/// What the Projection Depth track spans, in lengths of the frame's longer side
const PROJECTION_DEPTH_TRACK: RangeInclusive<f32> = 0.0..=1.0;

/// A degenerate warp frame's preview size: its side spans the preview's middle
const WARP_SPAN_FALLBACK: f32 = 1.7;

/// Change per pixel dragged on a 3D offset field
pub(crate) const OFFSET_FIELD_SPEED: f32 = 0.01;

/// Change per pixel dragged on a 3D rotation field, in degrees
const ROTATION_FIELD_SPEED: f32 = 0.5;

/// Change per pixel dragged on a 3D scale field
const SCALE_FIELD_SPEED: f32 = 0.01;

/// The tabs over a fill's Properties, each jumping to its section: a mask fill's last section is its Grayscale, a fill layer's its Material
pub(crate) fn fill_tabs(grayscale: bool) -> [(&'static str, &'static str); 3] {
    let last = if grayscale { (ic::SQUARE_HALF, tr("Grayscale")) } else { (ic::CIRCLE_HALF, tr("Material")) };
    [(ic::SLIDERS_HORIZONTAL, tr("Fill")), (ic::FLIP_HORIZONTAL, tr("Symmetry")), last]
}

/// Which fill a `base` addresses, (layer, effect), as the salt of its rows' widget state
pub(crate) type FillKey = (Option<u64>, Option<u64>);

/// The [`FillKey`] of the fill at `base`
pub(crate) fn fill_key(base: &Value) -> FillKey {
    (base.get("layer").and_then(Value::as_u64), base.get("effect").and_then(Value::as_u64))
}

/// The mesh's size for the texture set being painted, as the projections see it, measured once per mesh and material
fn mesh_size(app: &TextureCraftApp, ctx: &egui::Context) -> MeshSize {
    let mesh = &app.session.mesh;
    let material = app.session.doc.project.texture_set(app.session.ts()).map_or(0, |texture_set| texture_set.material);
    let key = (app.session.mesh_id, material);

    // Both measures walk the whole mesh, too slow for every frame the sheet draws on a dense mesh
    let cache_id = egui::Id::unique("tc_fill_mesh_size");
    if let Some((cached_key, size)) = ctx.data(|data| data.get_temp::<((u64, usize), MeshSize)>(cache_id))
        && cached_key == key
    {
        return size;
    }

    let size = MeshSize { radius: mesh.radius(), uv_density: mesh.uv_density(material).unwrap_or(1.0) };
    ctx.data_mut(|data| data.insert_temp(cache_id, (key, size)));
    size
}

/// A fill's whole Properties: the Fill section, the Symmetry section, then the Material section with one slot per channel that is on
pub(crate) fn fill_sheet(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, fill: &Fill, base: &Value, on: &[Channel], reader: LayerId) {
    placement_sections(app, sheet, fill, base, reader);
    crate::fill_material_panel::material_section(app, sheet, fill, base, on, reader);
}

/// The Fill section (with its reset) and the Symmetry section of the fill at `base`: a fill layer's, a content fill effect's or a mask fill's
pub(crate) fn placement_sections(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, fill: &Fill, base: &Value, reader: LayerId) {
    // The section reset: projection, placement, sampling and culling back to a new fill's in one undo step
    if sheet.section_reset(tr("Fill"), !texturecraft_engine::fill_reset_cmds::at_defaults(&fill.projection, fill.sampling)) {
        app.run("layer.fill.resetProjection", base.clone());
    }
    projection_rows(app, sheet, fill, base, fill_key(base), reader);
    crate::fill_symmetry_panel::symmetry_section(app, sheet, &fill.projection, fill.sampling, base);
}

/// Runs `layer.fill.projection` on the fill `base` names with `fields`
pub(crate) fn run_projection(app: &mut TextureCraftApp, base: &Value, fields: Value) {
    app.run("layer.fill.projection", with(base, fields));
}

/// Projection, Scale, the preview with size and offset (held, it moves the projection on the model), Rotation, the culling rows, then the Filtering and edges and 3D projection settings bands
fn projection_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, fill: &Fill, base: &Value, salt: FillKey, reader: LayerId) {
    let tokens = Tokens::get(sheet.ui.ctx());
    let kind = ProjectionKind::of(&fill.projection);
    let run = |app: &mut TextureCraftApp, fields: Value| run_projection(app, base, fields);

    let picture = |painter: &Painter, rect: Rect, kind: ProjectionKind| projection_picture(painter, rect, kind, &tokens);

    // The eye controls the project's viewport gizmo visibility
    let gizmos = app.session.doc.project.display.projection_gizmos;
    let tip = if gizmos { tr("Hide projection gizmos") } else { tr("Show projection gizmos") };
    let name = |kind: ProjectionKind| tr(kind.label()).to_owned();
    let (picked, eye) = if matches!(kind, ProjectionKind::Uv | ProjectionKind::UvTile) {
        (sheet.dropdown(tr("Projection"), kind, &ProjectionKind::ALL, name, Some(&picture)).0, false)
    } else {
        sheet.dropdown_with_eye(tr("Projection"), kind, &ProjectionKind::ALL, name, Some(&picture), (gizmos, tip))
    };
    if let Some(kind) = picked {
        run(app, json!({"projection": kind.id()}));
    }
    if eye {
        app.run("display.set", json!({"projectionGizmos": !gizmos}));
    }

    // Per-tile fills still sample images without a placement to edit
    if kind != ProjectionKind::UvTile {
        let size = mesh_size(app, sheet.ui.ctx());
        let length = tile_length(&fill.projection, size);
        let physical = length.is_some() && fill.sampling.scale == ScaleMode::PhysicalSize;
        if length.is_some() {
            let modes = [tr("Tiling"), tr("Physical size")];
            if let Some(index) = sheet.switch(tr("Scale"), &modes, usize::from(physical))
                && let Some(mode) = ScaleMode::ALL.get(index)
            {
                run(app, json!({"scaleMode": mode.id()}));
            }
        }

        let target = match base.get("effect").and_then(Value::as_u64) {
            Some(effect) => MoveOnModel { layer: reader, effect: Some(LayerId(effect)) },
            None => MoveOnModel::layer(reader),
        };
        placement_rows(app, sheet, fill, base, length.filter(|_| physical), size, target);
        frame_rows(app, sheet, fill, base);
        culling_rows(app, sheet, fill, base);
    }

    let crop = projector_of(&fill.projection).map(|projector| projector.crop);
    let shows_wrap = kind != ProjectionKind::Triplanar;
    let edges_changed = fill.sampling.filtering != Filtering::default()
        || (shows_wrap && fill.sampling.wrap != UvWrap::default())
        || crop.is_some_and(|crop| crop != ShapeCrop::default());
    let band = sheet.band(tr("Filtering and edges"), true, Some(edges_changed), None);
    if band.reset {
        let mut fields = json!({"filtering": Filtering::default().id(), "wrap": UvWrap::default().id()});
        if crop.is_some() {
            fields["shapeCrop"] = json!(ShapeCrop::default().id());
        }
        run(app, fields);
    }
    if band.open {
        let filtering = fill.sampling.filtering;
        let picture = |painter: &Painter, rect: Rect, filtering: Filtering| filtering_picture(painter, rect, filtering, &tokens);
        let (picked, _) = sheet.dropdown(tr("Filtering"), filtering, &Filtering::ALL, |filtering| tr(filtering_label(filtering)).to_owned(), Some(&picture));
        if let Some(filtering) = picked {
            run(app, json!({"filtering": filtering.id()}));
        }

        // Tri-planar has no tile to repeat
        if shows_wrap {
            let picture = |painter: &Painter, rect: Rect, wrap: UvWrap| wrap_picture(painter, rect, wrap, &tokens);
            let (picked, _) = sheet.dropdown(tr("UV Wrap"), fill.sampling.wrap, &UvWrap::ALL, |wrap| tr(wrap_label(wrap)).to_owned(), Some(&picture));
            if let Some(wrap) = picked {
                run(app, json!({"wrap": wrap.id()}));
            }
        }
        if let Some(projector) = projector_of(&fill.projection) {
            let picture = |painter: &Painter, rect: Rect, crop: ShapeCrop| crop_picture(painter, rect, crop, &tokens);
            let (picked, _) = sheet.dropdown(tr("Shape crop"), projector.crop, &ShapeCrop::ALL, |crop| tr(crop_label(crop)).to_owned(), Some(&picture));
            if let Some(crop) = picked {
                run(app, json!({"shapeCrop": crop.id()}));
            }
        }
    }

    world_rows(app, sheet, fill, base, salt);
}

/// The settings of the projector itself, shared by every 3D projection
fn projector_of(projection: &Projection) -> Option<&Projector> {
    match projection {
        Projection::Triplanar { projector, .. }
        | Projection::Planar { projector, .. }
        | Projection::Spherical { projector, .. }
        | Projection::Cylindrical { projector, .. }
        | Projection::Warp { projector, .. } => Some(projector),
        Projection::Uv { .. } | Projection::UvTile => None,
    }
}

/// The name of a Shape crop choice; both fit beside the label at the default panel width
fn crop_label(crop: ShapeCrop) -> &'static str {
    match crop {
        ShapeCrop::Cropped => n_("Cropped to Shape"),
        ShapeCrop::ExtendsOutside => n_("Extends Outside Shape"),
    }
}

/// Tri-planar Hardness as the panel shows it (0..1) from the blend's exponent (1..64): exponent = 64^hardness
fn triplanar_hardness(sharpness: f32) -> f32 {
    (sharpness.max(1.0).ln() / TRIPLANAR_SHARPNESS_MAX.ln()).clamp(0.0, 1.0)
}

/// The blend's exponent for a tri-planar Hardness the panel shows
fn triplanar_sharpness(hardness: f32) -> f32 {
    TRIPLANAR_SHARPNESS_MAX.powf(hardness.clamp(0.0, 1.0))
}

/// A warp's Projection Depth and Reset frame
fn frame_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, fill: &Fill, base: &Value) {
    let Projection::Warp { depth, .. } = &fill.projection else { return };
    let mut depth = *depth;

    // Its i tells it apart from Depth culling; it keeps the column every projection shares
    sheet.info(tr("How far the warp projection reaches into the mesh, in front of and behind its frame, in lengths of the frame's longer side. It bounds the fill only when cropped to its shape."));
    if let Some(merge) = sheet.slider_led(RowLead::Loose(tr("Depth")), &mut depth, PROJECTION_DEPTH_TRACK, 2, "") {
        run_projection(app, base, json!({"projectionDepth": depth, "merge": merge}));
    }
    if sheet.button(ic::ARROW_CLOCKWISE, tr("Reset frame")).clicked() {
        app.run("layer.fill.resetFrame", base.clone());
    }
}

/// Cylindrical Angle, then Depth culling (planar, warp) and Backface culling (planar, cylindrical), each with its settings under it
fn culling_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, fill: &Fill, base: &Value) {
    let tokens = Tokens::get(sheet.ui.ctx());
    let run = |app: &mut TextureCraftApp, fields: Value| run_projection(app, base, fields);
    let (projector, angle) = match &fill.projection {
        Projection::Cylindrical { projector, angle, .. } => (projector, Some(*angle)),
        Projection::Planar { projector, .. } | Projection::Warp { projector, .. } => (projector, None),
        _ => return,
    };

    if let Some(mut angle) = angle {
        let arc = angle;
        let picture = |painter: &Painter, rect: Rect| angle_picture(painter, rect, arc, &tokens);
        let lead = RowLead::Picture { picture: &picture, text: tr("Angle") };
        if let Some(merge) = sheet.slider_led(lead, &mut angle, CYLINDER_ANGLE_RANGE, 0, "°") {
            run(app, json!({"angle": angle, "merge": merge}));
        }
    }

    if !matches!(fill.projection, Projection::Cylindrical { .. }) {
        let mut culling = projector.depth;
        let picture = |painter: &Painter, rect: Rect| depth_picture(painter, rect, &tokens);
        sheet.info(tr("Paints only the surface the projector sees first; parts hidden behind it stay unpainted."));
        if sheet.picture_toggle(&picture, tr("Depth culling"), &mut culling.on) {
            run(app, json!({"depthCulling": culling.on}));
        }
        if let Some(merge) = sheet.slider_led(RowLead::Child(tr("Hardness")), &mut culling.hardness, HARDNESS_RANGE, 2, "") {
            run(app, json!({"depthCullingHardness": culling.hardness, "merge": merge}));
        }
    }

    if !matches!(fill.projection, Projection::Warp { .. }) {
        let mut backface = projector.backface;
        let culled = backface.on;
        let picture = |painter: &Painter, rect: Rect| backface_picture(painter, rect, culled, &tokens);
        sheet.info(tr("Leaves out faces turned away from the projector, so the fill doesn't run onto their back."));
        if sheet.picture_toggle(&picture, tr("Backface culling"), &mut backface.on) {
            run(app, json!({"backfaceCulling": backface.on}));
        }

        // Cylindrical culls by the angle to its axis's outward direction alone
        if angle.is_none()
            && let Some(merge) = sheet.angle_led(RowLead::Child(tr("Angle")), &mut backface.angle, BACKFACE_ANGLE_RANGE)
        {
            run(app, json!({"backfaceAngle": backface.angle, "merge": merge}));
        }
        if let Some(merge) = sheet.slider_led(RowLead::Child(tr("Hardness")), &mut backface.hardness, HARDNESS_RANGE, 2, "") {
            run(app, json!({"backfaceHardness": backface.hardness, "merge": merge}));
        }
    }
}

/// The preview square beside the size (tiling, or the tile's length while Physical size) and offset, held to move `target` on the model; Rotation under it
fn placement_rows(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    fill: &Fill,
    base: &Value,
    length: Option<[f32; 2]>,
    size: MeshSize,
    target: MoveOnModel,
) {
    let tokens = Tokens::get(sheet.ui.ctx());
    let (tiling, rotation, offset, offset_key) = match &fill.projection {
        Projection::Uv { scale, rotation, offset } => (*scale, *rotation, *offset, "offset"),
        Projection::Triplanar { uv, .. }
        | Projection::Planar { uv, .. }
        | Projection::Spherical { uv, .. }
        | Projection::Cylindrical { uv, .. }
        | Projection::Warp { uv, .. } => (uv.tiling, uv.rotation, uv.offset, "uvOffset"),
        Projection::UvTile => return,
    };

    let link_id = sheet.ui.make_persistent_id(("fill_tiling_link", fill_key(base)));
    let mut linked = sheet.ui.data(|data| data.get_temp::<bool>(link_id)).unwrap_or(tiling[0] == tiling[1]);
    let was_linked = linked;
    let mut values = match length {
        Some(length) => length.map(|units| units * CM_PER_UNIT),
        None => tiling,
    };
    let shown = values;
    let (label, range, typed, decimals) = match length {
        Some(_) => (tr("Size (cm)"), SIZE_CM_TRACK, SIZE_CM_TYPED, 0),
        None => (tr("Tiling"), TILING_TRACK, TILING_TYPED, 2),
    };
    let size_edit = PreviewSize { label, values: &mut values, range, typed, decimals, linked: &mut linked };

    let mut lower_offset = offset;
    let mut hardness = match &fill.projection {
        Projection::Triplanar { sharpness, .. } => Some(triplanar_hardness(*sharpness)),
        _ => None,
    };
    let lower = match hardness.as_mut() {
        Some(hardness) => PreviewLower::Slider { label: tr("Hardness"), value: hardness, range: HARDNESS_RANGE, decimals: 2 },
        None => PreviewLower::Offset { values: &mut lower_offset, range: OFFSET_TRACK, typed: OFFSET_TYPED },
    };
    let placement = Placement { tiling, rotation, offset };
    let bounds = app.viewport_mesh_bounds.get(&app.session.mesh);

    // A warp shows its frame's size and turn as its gizmo leaves them
    let warp = match &fill.projection {
        Projection::Warp { corners, .. } => {
            let landed = frame_placement::folded(&fill.projection);
            let frame = landed.as_ref().and_then(Projection::frame);
            let diagonal = (bounds.1 - bounds.0).length().max(f32::EPSILON);
            let size = frame.map_or([1.0 / WARP_SPAN_FALLBACK; 2], |frame| frame.size.map(|side| side / diagonal));
            let turn = frame.and_then(frame_placement::upright_turn).unwrap_or(0.0);
            Some((*corners, size, turn))
        }
        _ => None,
    };
    let projection = &fill.projection;
    let owner = sheet.ui.scope_id();
    let picture = |painter: &Painter, rect: Rect| match warp {
        Some((corners, size, turn)) => warp_preview(painter, rect, corners, size, turn, placement, &tokens),
        None => {
            if projection_preview(painter, rect, projection, bounds, &tokens, owner).is_none() {
                window_preview(painter, rect, placement, &tokens);
            }
        }
    };
    let edit = sheet.preview_block(&picture, size_edit, lower);
    crate::move_on_model::pad(app, sheet, edit.square, target);

    if linked != was_linked {
        sheet.ui.data_mut(|data| data.insert_temp(link_id, linked));
    }
    if let Some(merge) = edit.size
        && values != shown
    {
        let tiling = match length {
            Some(_) => tiling_for_length(&fill.projection, size, values.map(|cm| cm / CM_PER_UNIT)).map_or(tiling, |converted| {
                // Keep the stored tiling for axes that did not change
                std::array::from_fn(|axis| if values[axis] == shown[axis] { tiling[axis] } else { converted[axis] })
            }),
            None => values,
        };
        run_projection(app, base, json!({"tiling": tiling, "merge": merge}));
    }
    if let Some(merge) = edit.lower {
        let fields = match hardness {
            Some(hardness) => json!({"sharpness": triplanar_sharpness(hardness), "merge": merge}),
            None => json!({offset_key: lower_offset, "merge": merge}),
        };
        run_projection(app, base, fields);
    }

    let mut degrees = rotation;
    if let Some(merge) = sheet.angle(tr("Rotation"), &mut degrees, ROTATION_RANGE) {
        run_projection(app, base, json!({"rotation": degrees, "merge": merge}));
    }
}

/// One X, Y, Z row of the 3D projection settings
fn xyz_fields(values: [f32; 3], speed: f32, decimals: usize, suffix: &'static str) -> [NumberField<'static>; 3] {
    std::array::from_fn(|axis| NumberField { tag: AXIS_NAMES[axis], tag_color: Some(AXIS_COLORS[axis]), value: values[axis], speed, decimals, suffix })
}

/// The 3D projection settings: the projector's Offset, Rotation and Scale in X, Y and Z; a frame on the surface shows its own (where its centre is, how it is turned, how big it is), the same values its gizmo drags
fn world_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, fill: &Fill, base: &Value, salt: FillKey) {
    let Some(projector) = projector_of(&fill.projection) else { return };
    if !sheet.band(tr("3D projection settings"), true, None, None).open {
        return;
    }

    let edited = |fields: &[NumberField]| -> Vec<f32> { fields.iter().map(|field| field.value).collect() };

    // A placed planar or a warp: the frame, its projector folded in
    if let Some(shown) = frame_placement::placement(&fill.projection, app.viewport_mesh_bounds.get(&app.session.mesh)) {
        let run = |app: &mut TextureCraftApp, key: &str, values: Vec<f32>, merge: bool| {
            app.run("layer.decal.set", with(base, json!({key: values, "merge": merge})));
        };
        let mut fields = xyz_fields(shown.offset, OFFSET_FIELD_SPEED, 2, "");
        if let Some((_, merge)) = sheet.number_fields(tr("Offset"), &mut fields, None, true) {
            run(app, "frameOffset", edited(&fields), merge);
        }
        let mut fields = xyz_fields(shown.rotation, ROTATION_FIELD_SPEED, 0, "°");
        if let Some((_, merge)) = sheet.number_fields(tr("Rotation"), &mut fields, None, true) {
            run(app, "frameRotation", edited(&fields), merge);
        }
        if let Some((scale, merge)) = scale_row(sheet, shown.scale, salt) {
            run(app, "frameScale", scale, merge);
        }
        return;
    }

    let offset = match &fill.projection {
        Projection::Triplanar { offset, .. }
        | Projection::Planar { offset, .. }
        | Projection::Spherical { offset, .. }
        | Projection::Cylindrical { offset, .. } => Some(*offset),
        Projection::Warp { .. } | Projection::Uv { .. } | Projection::UvTile => None,
    };
    if let Some(offset) = offset {
        let mut fields = xyz_fields(offset, OFFSET_FIELD_SPEED, 2, "");
        if let Some((_, merge)) = sheet.number_fields(tr("Offset"), &mut fields, None, true) {
            run_projection(app, base, json!({"offset": edited(&fields), "merge": merge}));
        }
    }
    let mut fields = xyz_fields(projector.rotation, ROTATION_FIELD_SPEED, 0, "°");
    if let Some((_, merge)) = sheet.number_fields(tr("Rotation"), &mut fields, None, true) {
        run_projection(app, base, json!({"projectorRotation": edited(&fields), "merge": merge}));
    }
    if let Some((scale, merge)) = scale_row(sheet, projector.scale, salt) {
        run_projection(app, base, json!({"projectorScale": scale, "merge": merge}));
    }
}

/// The least size of the edited axis's scale a linked edit scales the others from by a factor
const MIN_LINKED_SCALE_BASE: f32 = f32::EPSILON;

/// The Scale X, Y, Z row with its link: the new scale and whether it continues the drag's undo step; linked, one field scales all three by the same factor
fn scale_row(sheet: &mut PropertySheet<'_>, scale: [f32; 3], salt: FillKey) -> Option<(Vec<f32>, bool)> {
    let link_id = sheet.ui.make_persistent_id(("projector_scale_link", salt));
    let [x, y, z] = scale;
    let mut linked = sheet.ui.data(|data| data.get_temp::<bool>(link_id)).unwrap_or(x == y && y == z);
    let was_linked = linked;
    let mut fields = xyz_fields(scale, SCALE_FIELD_SPEED, 2, "");
    let edit = sheet.number_fields(tr("Scale"), &mut fields, Some(&mut linked), false);
    if linked != was_linked {
        sheet.ui.data_mut(|data| data.insert_temp(link_id, linked));
    }
    let (axis, merge) = edit?;
    let mut values: Vec<f32> = fields.iter().map(|field| field.value).collect();
    if linked && let (Some(&value), Some(&before)) = (values.get(axis), scale.get(axis)) {
        // From a scale of about zero no factor reaches the new value: all three take it instead
        let factor = (before.abs() > MIN_LINKED_SCALE_BASE).then(|| value / before);
        values = scale.iter().map(|old| factor.map_or(value, |factor| old * factor)).collect();
    }
    Some((values, merge))
}
