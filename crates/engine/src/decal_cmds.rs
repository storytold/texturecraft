//! Geometry decals: an image resource projected onto the surface from a frame placed on it
//! (a decal fill with a planar projection), as a fill layer with a decal projection, and moving
//! the frame of any planar or warp projection

use serde_json::{Value, json};
use texturecraft_compose::axis_frame::{placed_from_axis, placing_moves_pattern};
use texturecraft_compose::frame_placement::{MIN_FRAME_SIDE, culling_folds, folded, longer_side, placed, placement as frame_placement};
use texturecraft_core::{
    Channel, Decal, DecalFlip, DecalSymmetry, Effect, EffectKind, Fill, Layer, LayerId, LayerKind, Mask, MaskBackground, Projection, Source,
};
use texturecraft_geom::glam::{Quat, Vec2, Vec3};

use crate::commands::{CommandSpec, always, bad, boolp, channel_param, f32p, layer_param, strp, u64p, vec3};
use crate::layer_cmds::{FULL_TURN_DEGREES, MAX_PROJECTION_DEPTH};
use crate::tool_cmds::{pair, viewport_of};
use crate::{Result, Session, StackTarget, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layer.add.decal",
            "Add decal",
            [],
            None,
            "{image: resource name, channel?: a channel id (default baseColor) | \"mask\" (a new fill layer masked by the image's gray), x?, y?: viewport pixels (default: the view center; a point on another texture set selects it), viewport?: [w,h] | position?: [x,y,z], normal?: [x,y,z] (on the selected texture set), up?: [x,y,z] (the image's up; default the view's up, or the view direction when that is along the normal), size?: height | [w,h] (world units; default a quarter of the mesh), keepProportions?: true makes the height follow the width by the image's aspect, flipU?, flipV?: mirror the image left to right / top to bottom inside the frame, rotation?: degrees, depth?, symmetry?: x|y|z|off (also place a mirrored copy across the mesh center, perpendicular to that axis; it follows this decal), symmetryFlip?: none|u|v (flip the copy's image on top of the reflection), symmetryOffset?: world units from the mesh center along the axis to the mirror plane (default 0, within the mesh), name?} → {layer, decal, maskEffect: the mask fill effect holding the image with channel \"mask\", else null} — a fill layer projecting the image from a frame placed on the surface; seen from the front, the image reads the right way round",
            always,
            add
        ),
        cmd!(
            "layer.decal.set",
            "Edit decal",
            [],
            None,
            "{layer? | effect?: a mask fill effect id (a decal placed as a mask; with `layer`, it must be in that layer) — any fill with a planar or warp projection: moves the frame it projects from (an axis planar is first placed as the frame it projects from, reading images exactly as before; a warp keeps its corners), x?, y?, viewport? | position?, normal? (re-place it on its own texture set), up?: [x,y,z] (default: the decal's up carried along when it moves onto a surface facing another way), size?: height | [w,h], keepProportions?: true keeps the image's aspect (the current ratio without an image): a height sets the width, a [w,h] pair or no size sets the height from the width, flipU?, flipV?: mirror the image left to right / top to bottom inside the frame, rotation?: degrees, offset?: [right, up] (world units from where it was placed), depth?: world units (a warp: its Projection Depth becomes this over the frame's longer side), frameOffset?: [x, y, z] (the frame's centre in mesh radii from the mesh's centre), frameRotation?: [x, y, z] degrees (its turn: unturned it faces +Z with +Y up), frameScale?: [width, height, reach] (in mesh diagonals; reach = how far it projects in front of and behind it, a warp's Projection Depth times its longer side) — the frame as the 3D projection settings show it, applied before the other fields; any projector turn or scale is first folded into the frame (it lands the same); refused: an axis planar reading a procedural (placing it would move the pattern) and, under backface culling with an uneven or mirrored projector scale, placing an axis planar or the frame fields (even the scale out or turn culling off), symmetry?: x|y|z|off, symmetryFlip?: none|u|v, symmetryOffset? (the mirrored copy; the flip and offset alone keep the axis), image?: replaces the image in the channel or mask it was placed in, merge?: true continues the previous Edit decal undo step (drags); a call that changes nothing adds no undo step}",
            always,
            set
        ),
    ]
}

/// Resolve a viewport hit or explicit world position, using `texture_set` for explicit positions.
/// Returns None when placement is optional and no position or viewport point is given.
fn placement(session: &Session, command: &str, params: &Value, texture_set: usize, optional: bool) -> Result<Option<(Vec3, Vec3, usize)>> {
    if let Some(position) = vec3(params, "position") {
        let normal = vec3(params, "normal").and_then(Vec3::try_normalize).ok_or_else(|| bad(command, "`position` needs a non-zero `normal`"))?;
        return Ok(Some((position, normal, texture_set)));
    }

    let viewport = viewport_of(session, params);
    let point = match (f32p(params, "x"), f32p(params, "y")) {
        (Some(x), Some(y)) => Vec2::new(x, y),
        _ if optional => return Ok(None),
        _ => viewport * 0.5,
    };
    let hit = session.pick(point, viewport).ok_or_else(|| bad(command, "nothing under that point: place the decal on the mesh"))?;
    let hit_set = session.texture_set_of_material(hit.material).ok_or_else(|| bad(command, "that surface has no texture set"))?;
    Ok(Some((hit.pos, hit.nrm, hit_set)))
}

/// A new decal's height, relative to the mesh diagonal
const DEFAULT_HEIGHT: f32 = 0.25;

/// The smallest decal side or depth, relative to the mesh diagonal
const MIN_SIZE: f32 = 1e-4;

/// The largest decal side or depth, relative to the mesh diagonal
const MAX_SIZE: f32 = 10.0;

/// The smallest mesh diagonal decal sizes count in, so a collapsed mesh still places one
const MIN_MESH_DIAGONAL: f32 = 1e-3;

/// A given `up` this near the normal (squared sine of the angle between them) leaves no upright direction
const MIN_GIVEN_UP_CROSS_SQUARED: f32 = 1e-8;

/// The camera's up this near the normal (squared sine) is a top face seen from the front: up is where the camera looks
const MIN_CAMERA_UP_CROSS_SQUARED: f32 = 1e-4;

/// The mesh's diagonal (decal sizes are relative to it).
fn diagonal_of((minimum, maximum): (Vec3, Vec3)) -> f32 {
    (maximum - minimum).length().max(MIN_MESH_DIAGONAL)
}

/// Width and height from a `size` param (a height keeping `aspect`, or `[w, h]`), within the decal size limits
fn size_param(command: &str, params: &Value, aspect: f32, mesh_diagonal: f32) -> Result<Option<[f32; 2]>> {
    let Some(value) = params.get("size") else { return Ok(None) };
    let size = if value.is_array() { pair(params, "size") } else { value.as_f64().map(|height| [height as f32 * aspect, height as f32]) };
    let size = size.filter(|size| size.iter().all(|side| side.is_finite())).ok_or_else(|| bad(command, "`size` must be a finite height or [width, height]"))?;

    Ok(Some(size.map(|side| side.clamp(mesh_diagonal * MIN_SIZE, mesh_diagonal * MAX_SIZE))))
}

/// The decal's up direction: `up` when given, else the camera's (decals are placed upright on
/// screen), or where the camera looks when that is along `normal` (a top face seen from the
/// front: its far edge is up, as on a table).
fn decal_up(session: &Session, command: &str, params: &Value, normal: Vec3) -> Result<Vec3> {
    if params.get("up").is_some() {
        return vec3(params, "up")
            .filter(|up| up.cross(normal).length_squared() > MIN_GIVEN_UP_CROSS_SQUARED)
            .ok_or_else(|| bad(command, "`up` must be [x, y, z], not along the normal"));
    }

    let to_world = session.camera.view().inverse();
    let up = to_world.transform_vector3(Vec3::Y);
    Ok(if up.normalize_or_zero().cross(normal).length_squared() > MIN_CAMERA_UP_CROSS_SQUARED { up } else { to_world.transform_vector3(Vec3::NEG_Z) })
}

/// The mirrored copy asked for by `symmetry` (x, y, z or off) and `symmetryFlip` (none, u or v),
/// starting from `current` for what the params leave out
fn symmetry_param(cmd: &str, p: &Value, current: Option<DecalSymmetry>, bounds: (Vec3, Vec3)) -> Result<Option<DecalSymmetry>> {
    let axis = match strp(p, "symmetry").map(str::to_ascii_lowercase).as_deref() {
        None => current.map(|symmetry| symmetry.axis.min(2)),
        Some("off" | "none") => None,
        Some("x") => Some(0),
        Some("y") => Some(1),
        Some("z") => Some(2),
        Some(other) => return Err(bad(cmd, format!("unknown symmetry `{other}` (x, y, z or off)"))),
    };

    let flip = match strp(p, "symmetryFlip").map(str::to_ascii_lowercase).as_deref() {
        None => current.map_or(DecalFlip::None, |symmetry| symmetry.flip),
        Some("none" | "off") => DecalFlip::None,
        Some("u") => DecalFlip::U,
        Some("v") => DecalFlip::V,
        Some(other) => return Err(bad(cmd, format!("unknown symmetryFlip `{other}` (none, u or v)"))),
    };

    // The plane stays within the mesh, as the brush symmetry's offset does
    let (lo, hi) = bounds;
    let half_extent = (hi - lo) * 0.5;
    let offset = match (f32p(p, "symmetryOffset"), current) {
        (Some(offset), _) if !offset.is_finite() => return Err(bad(cmd, "`symmetryOffset` must be a number")),
        (Some(offset), _) => offset,
        (None, Some(symmetry)) => symmetry.offset,
        (None, None) => 0.0,
    };

    Ok(axis.map(|axis| {
        let limit = half_extent[usize::from(axis)];
        DecalSymmetry { axis, flip, offset: offset.clamp(-limit, limit) }
    }))
}

/// The up of a decal moved from a surface facing `from.0` (up `from.1`) onto one facing `to`: the
/// old up turned by the smallest rotation that takes the old normal to the new one (parallel
/// transport), so it follows the surface smoothly and never degenerates. A surface turned
/// exactly away has no smallest rotation: the camera rule of a new decal decides there.
fn carried_up(s: &Session, from: (Vec3, Vec3), to: Vec3) -> Vec3 {
    let (old_normal, old_up) = from;
    let (Some(old_normal), Some(new_normal)) = (old_normal.try_normalize(), to.try_normalize()) else { return old_up };
    if old_normal.dot(new_normal) < -0.9999 {
        return decal_up(s, "", &Value::Null, new_normal).unwrap_or(old_up);
    }

    Quat::from_rotation_arc(old_normal, new_normal) * old_up
}

fn add(session: &mut Session, params: &Value) -> Result<Value> {
    let command = "layer.add.decal";
    let image = strp(params, "image").ok_or_else(|| bad(command, "`image` is required"))?.to_string();
    let resource = session.images.get(&image).ok_or_else(|| bad(command, format!("no image resource `{image}` (import it first)")))?;
    let aspect = resource.width as f32 / resource.height.max(1) as f32;
    let (center, normal, texture_set) = placement(session, command, params, session.ts(), false)?.ok_or_else(|| bad(command, "no placement"))?;

    // None: the image masks a new fill layer
    let target = match strp(params, "channel") {
        Some("mask") => None,
        _ => Some(channel_param(command, params, "channel")?.unwrap_or(Channel::BaseColor)),
    };
    let set_channels = session.doc.project.texture_set(texture_set)?.channel_list();
    if let Some(channel) = target
        && !set_channels.contains(&channel)
    {
        return Err(bad(command, format!("the texture set has no {} channel", channel.label())));
    }

    let bounds = session.mesh.bounds();
    let mesh_diagonal = diagonal_of(bounds);
    let default_height = mesh_diagonal * DEFAULT_HEIGHT;
    let mut size = size_param(command, params, aspect, mesh_diagonal)?.unwrap_or([default_height * aspect, default_height]);
    if boolp(params, "keepProportions") == Some(true) && params.get("size").is_some_and(Value::is_array) {
        size[1] = (size[0] / aspect).clamp(mesh_diagonal * MIN_SIZE, mesh_diagonal * MAX_SIZE);
    }
    let decal = Decal {
        center: center.to_array(),
        normal: normal.to_array(),
        up: decal_up(session, command, params, normal)?.to_array(),
        size,
        rotation: f32p(params, "rotation").unwrap_or(0.0).rem_euclid(FULL_TURN_DEGREES),
        offset: [0.0; 2],
        flip: [boolp(params, "flipU").unwrap_or(false), boolp(params, "flipV").unwrap_or(false)],

        // By default it reaches half its larger side into the surface
        depth: f32p(params, "depth").map_or(size[0].max(size[1]) * 0.5, |depth| depth.clamp(mesh_diagonal * MIN_SIZE, mesh_diagonal * MAX_SIZE)),
        symmetry: symmetry_param(command, params, None, bounds)?,
    };

    // A decal dropped on another texture set goes there and selects it, as a dropped material does
    if texture_set != session.ts() {
        crate::set_cmds::select_set(session, texture_set)?;
    }

    let base = strp(params, "name")
        .map(str::to_string)
        .unwrap_or_else(|| std::path::Path::new(&image).file_stem().and_then(|stem| stem.to_str()).unwrap_or("Decal").to_string());
    let name = session.doc.project.next_layer_name(texture_set, &base);
    let (id, mask_effect) = session.edit("Add decal", |session| {
        let id = session.doc.project.alloc_id();
        let Some(channel) = target else {
            let effect_id = session.doc.project.alloc_id();
            let mut layer = Layer::new(id, name, LayerKind::Fill(Fill::default()), &set_channels);
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(
                effect_id,
                EffectKind::Fill { source: Source::image(image.clone()), projection: Projection::decal(decal), sampling: Default::default() },
            ));
            layer.mask = Some(mask);
            crate::layer_cmds::insert_selected(session, layer)?;
            session.target = StackTarget::Effect(effect_id);
            return Ok((id, Some(effect_id)));
        };

        let mut fill = Fill { projection: Projection::decal(decal), ..Fill::default() };
        fill.sources.insert(channel, Source::image(image.clone()));
        crate::layer_cmds::insert_selected(session, Layer::new(id, name, LayerKind::Fill(fill), &[channel]))?;
        Ok((id, None))
    })?;
    Ok(json!({ "layer": id.0, "decal": decal, "maskEffect": mask_effect.map(|effect| effect.0) }))
}

/// The projection of fill layer `id`, or of mask fill effect `effect` when given, with the frame it projects from: an axis planar is placed as the frame it already projects from (it reads images exactly as before), a turned or scaled projector is folded into its frame
fn framed(session: &Session, command: &str, id: LayerId, effect: Option<LayerId>, bounds: (Vec3, Vec3)) -> Result<(Projection, Decal)> {
    let projection = stored_projection(session, command, id, effect)?;
    if matches!(projection, Projection::Planar { placed: None, .. }) && placing_moves_pattern(stored_sources(session, id, effect).into_iter()) {
        return Err(bad(command, "an axis planar reading a procedural pattern keeps its axis: placing its frame would move the pattern"));
    }

    // Keep the active projection while folding scaling would change which faces cull
    if matches!(projection, Projection::Planar { placed: None, .. }) && !culling_keeps_facing(projection) {
        return Err(bad(command, UNFOLDABLE_CULLING));
    }

    let projection = placed_from_axis(projection, bounds).unwrap_or_else(|| projection.clone());

    // The frame moves as it lands: a projector's own turn and scale go into it first
    let projection = folded(&projection).unwrap_or(projection);
    let frame = projection
        .frame()
        .copied()
        .ok_or_else(|| bad(command, "the projection has no frame: a planar or warp projection has one (layer.add.decal places one)"))?;
    Ok((projection, frame))
}

/// Whether folding the projector's transform into its frame preserves culling
fn culling_keeps_facing(projection: &Projection) -> bool {
    match projection {
        Projection::Planar { projector, .. } | Projection::Warp { projector, .. } => culling_folds(projector),
        _ => true,
    }
}

/// Explain why the projector transform cannot be folded into its frame
const UNFOLDABLE_CULLING: &str = "the projector scale would change culling when moved into a frame. For uneven scale, even out the projector scale or turn backface culling off. For a mirrored projector, restore a positive scale or turn both backface and depth culling off";

/// The projection fill layer `id`, or mask fill effect `effect` when given, holds
fn stored_projection<'a>(session: &'a Session, command: &str, id: LayerId, effect: Option<LayerId>) -> Result<&'a Projection> {
    match effect {
        Some(effect) => match session.doc.project.effect(effect).map(|(_, effect)| &effect.kind) {
            Some(EffectKind::Fill { projection, .. }) => Ok(projection),
            _ => Err(bad(command, "the effect is not a mask fill effect")),
        },
        None => match &session.doc.project.layer(id)?.kind {
            LayerKind::Fill(fill) => Ok(&fill.projection),
            _ => Err(bad(command, "the layer is not a fill layer")),
        },
    }
}

/// The sources fill layer `id`, or mask fill effect `effect` when given, reads
fn stored_sources(session: &Session, id: LayerId, effect: Option<LayerId>) -> Vec<&Source> {
    match effect {
        Some(effect) => match session.doc.project.effect(effect).map(|(_, effect)| &effect.kind) {
            Some(EffectKind::Fill { source, .. }) => vec![source],
            _ => Vec::new(),
        },
        None => match session.doc.project.layer(id).map(|layer| &layer.kind) {
            Ok(LayerKind::Fill(fill)) => fill.sources.values().collect(),
            _ => Vec::new(),
        },
    }
}

/// The image a decal projects: the mask fill effect's, or the fill layer's (the channel it was placed in)
fn placed_image(session: &Session, id: LayerId, effect: Option<LayerId>) -> Option<String> {
    let image_name = |source: &Source| match source {
        Source::Image { name, .. } => Some(name.clone()),
        _ => None,
    };
    match effect {
        Some(effect) => match session.doc.project.effect(effect).map(|(_, effect)| &effect.kind) {
            Some(EffectKind::Fill { source, .. }) => image_name(source),
            _ => None,
        },
        None => match &session.doc.project.layer(id).ok()?.kind {
            LayerKind::Fill(fill) => fill.sources.values().find_map(image_name),
            _ => None,
        },
    }
}

/// `projection` with its frame where `frameOffset`, `frameRotation` and `frameScale` put it (None when none is given)
fn placed_as_shown(command: &str, params: &Value, projection: &Projection, bounds: (Vec3, Vec3)) -> Result<Option<Projection>> {
    let fields = ["frameOffset", "frameRotation", "frameScale"];
    if fields.iter().all(|key| params.get(*key).is_none()) {
        return Ok(None);
    }

    let read = |key: &str| match params.get(key) {
        None => Ok(None),
        Some(_) => vec3(params, key).map(Some).ok_or_else(|| bad(command, format!("`{key}` must be [x, y, z]"))),
    };
    if !culling_keeps_facing(projection) {
        return Err(bad(command, UNFOLDABLE_CULLING));
    }

    let mut shown = frame_placement(projection, bounds).ok_or_else(|| bad(command, "the frame is degenerate"))?;
    if let Some(offset) = read("frameOffset")? {
        shown.offset = offset.clamp(Vec3::splat(-MAX_FRAME_OFFSET), Vec3::splat(MAX_FRAME_OFFSET)).to_array();
    }
    if let Some(rotation) = read("frameRotation")? {
        shown.rotation = rotation.clamp(Vec3::splat(-FULL_TURN_DEGREES), Vec3::splat(FULL_TURN_DEGREES)).to_array();
    }
    if let Some(scale) = read("frameScale")? {
        shown.scale = scale.clamp(Vec3::splat(MIN_SIZE), Vec3::splat(MAX_SIZE)).to_array();
    }
    placed(projection, &shown, bounds).map(Some).ok_or_else(|| bad(command, "that frame is degenerate"))
}

/// How far from the mesh's centre `frameOffset` may put a frame, in mesh radii
const MAX_FRAME_OFFSET: f32 = 100.0;

/// `projection` projecting from `frame`, its other settings kept
fn move_frame(projection: &mut Projection, frame: Decal) {
    match projection {
        Projection::Planar { placed: Some(placed), .. } => *placed = frame,
        Projection::Warp { frame: warp, .. } => *warp = frame,
        _ => {}
    }
}

/// Width over height of the image the decal of fill layer `id` (or of mask fill effect `effect`) shows
/// (`new`: the image a call replaces it with, which then counts instead)
fn image_aspect(s: &Session, id: LayerId, effect: Option<LayerId>, new: Option<&str>) -> Option<f32> {
    let name = match effect {
        _ if new.is_some() => new?.to_string(),
        Some(effect) => match &s.doc.project.effect(effect)?.1.kind {
            EffectKind::Fill { source: Source::Image { name, .. }, .. } => name.clone(),
            _ => return None,
        },
        None => match &s.doc.project.layer(id).ok()?.kind {
            LayerKind::Fill(fill) => fill.sources.values().find_map(|source| match source {
                Source::Image { name, .. } => Some(name.clone()),
                _ => None,
            })?,
            _ => return None,
        },
    };
    let image = s.images.get(&name)?;
    Some(image.width as f32 / image.height.max(1) as f32)
}

fn set(session: &mut Session, params: &Value) -> Result<Value> {
    let command = "layer.decal.set";
    let effect = u64p(params, "effect").map(LayerId);
    let layer_id = match effect {
        Some(effect) => session.doc.project.effect(effect).map(|(owner, _)| owner).ok_or_else(|| bad(command, format!("no effect {}", effect.0)))?,
        None => session.doc.project.content_layer(layer_param(session, command, params)?).map(|layer| layer.id).ok_or_else(|| bad(command, "no layer"))?,
    };
    if let (Some(effect), Some(layer)) = (effect, u64p(params, "layer"))
        && layer != layer_id.0
    {
        return Err(bad(command, format!("effect {} is not in layer {layer}", effect.0)));
    }

    let bounds = session.mesh.bounds();
    let (mut projection, mut decal) = framed(session, command, layer_id, effect, bounds)?;
    let texture_set =
        session.doc.project.locate(layer_id).map(|(texture_set, _, _)| texture_set).ok_or_else(|| bad(command, "the decal has no texture set"))?;
    let mesh_diagonal = diagonal_of(bounds);
    if let Some(moved) = placed_as_shown(command, params, &projection, bounds)? {
        projection = moved;
        decal = projection.frame().copied().ok_or_else(|| bad(command, "the projection lost its frame"))?;

        // A reach from frameScale keeps the Projection Depth in the range every other path keeps
        if let Projection::Warp { depth: warp_depth, .. } = &mut projection {
            *warp_depth = warp_depth.clamp(0.0, MAX_PROJECTION_DEPTH);
        }
    }
    if let Some((center, normal, hit_set)) = placement(session, command, params, texture_set, true)? {
        // Selecting another set must not let this decal leave its layer's stack
        if hit_set != texture_set {
            return Err(bad(command, "that surface belongs to another texture set"));
        }

        // Carry up so a stale axis cannot lie in or flip on the new surface
        let previous = (Vec3::from(decal.normal), Vec3::from(decal.up));
        (decal.center, decal.normal, decal.offset) = (center.to_array(), normal.to_array(), [0.0; 2]);
        if params.get("up").is_none() {
            decal.up = carried_up(session, previous, normal).to_array();
        }
    }
    if params.get("up").is_some() {
        decal.up = decal_up(session, command, params, Vec3::from(decal.normal))?.to_array();
    }
    let keep = boolp(params, "keepProportions") == Some(true);
    let ratio = decal.size[0] / decal.size[1].max(MIN_FRAME_SIDE);
    let aspect = if keep { image_aspect(session, layer_id, effect, strp(params, "image")).unwrap_or(ratio) } else { ratio };
    if let Some(mut size) = size_param(command, params, aspect, mesh_diagonal)?.or_else(|| keep.then_some(decal.size)) {
        // A height sets the width through `aspect`; a pair or the current size sets the height from the width
        if keep && params.get("size").is_none_or(Value::is_array) {
            size[1] = (size[0] / aspect).clamp(mesh_diagonal * MIN_SIZE, mesh_diagonal * MAX_SIZE);
        }
        decal.size = size;
    }
    if let Some(flip) = boolp(params, "flipU") {
        decal.flip[0] = flip;
    }
    if let Some(flip) = boolp(params, "flipV") {
        decal.flip[1] = flip;
    }
    if let Some(rotation) = f32p(params, "rotation") {
        decal.rotation = rotation.rem_euclid(FULL_TURN_DEGREES);
    }
    if params.get("offset").is_some() {
        let offset = pair(params, "offset").ok_or_else(|| bad(command, "`offset` must be [right, up]"))?;
        decal.offset = offset.map(|component| component.clamp(-mesh_diagonal, mesh_diagonal));
    }

    let depth = f32p(params, "depth").map(|depth| depth.clamp(mesh_diagonal * MIN_SIZE, mesh_diagonal * MAX_SIZE));

    // A warp reaches by its Projection Depth (set below); its frame's own depth is unused, so it stays
    if let Some(depth) = depth
        && !matches!(projection, Projection::Warp { .. })
    {
        decal.depth = depth;
    }
    decal.symmetry = symmetry_param(command, params, decal.symmetry, bounds)?;

    let image = strp(params, "image").map(str::to_string);
    if let Some(name) = &image
        && !session.doc.resources.contains_key(name)
    {
        return Err(bad(command, format!("no image resource `{name}`")));
    }

    move_frame(&mut projection, decal);

    // A warp reaches by its own Projection Depth, in lengths of the frame's longer side
    if let (Some(depth), Projection::Warp { depth: warp_depth, .. }) = (depth, &mut projection) {
        *warp_depth = (depth / longer_side(decal.size)).clamp(0.0, MAX_PROJECTION_DEPTH);
    }

    // Nothing moved and the image is the one it shows: no undo step
    let unchanged = *stored_projection(session, command, layer_id, effect)? == projection
        && image.as_ref().is_none_or(|image| placed_image(session, layer_id, effect).as_ref() == Some(image));
    if !unchanged {
        session.edit("Edit decal", |session| store_decal(session, layer_id, effect, projection, image))?;
    }
    Ok(json!({ "layer": layer_id.0, "decal": decal }))
}

/// Store `projection` (and `image`, in the channel or mask it was placed in) on fill layer `layer_id`, or on mask fill effect `effect` when given
fn store_decal(session: &mut Session, layer_id: LayerId, effect: Option<LayerId>, projection: Projection, image: Option<String>) -> Result<()> {
    if let Some(effect) = effect {
        if let Some(EffectKind::Fill { source, projection: stored, .. }) = session.doc.project.effect_mut(effect).map(|effect| &mut effect.kind) {
            *stored = projection;
            if let Some(image) = image {
                *source = Source::image(image);
            }
        }
        return Ok(());
    }

    let LayerKind::Fill(fill) = &mut session.doc.project.layer_mut(layer_id)?.kind else { return Ok(()) };
    fill.projection = projection;

    let Some(image) = image else { return Ok(()) };

    // Replace the original image in every channel that shared it
    let placed = fill.sources.values().find_map(|source| match source {
        Source::Image { name, .. } => Some(name.clone()),
        _ => None,
    });
    match placed {
        Some(placed) if placed == image => return Ok(()),
        Some(placed) => {
            for source in fill.sources.values_mut() {
                if let Source::Image { name, .. } = source
                    && *name == placed
                {
                    name.clone_from(&image);
                }
            }
        }
        None => {
            fill.sources.insert(Channel::BaseColor, Source::image(image));
        }
    }
    fill.material = None;
    Ok(())
}
