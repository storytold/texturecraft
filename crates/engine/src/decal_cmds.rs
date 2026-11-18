//! Geometry decals: an image resource projected onto the surface from a frame placed on it
//! (a decal fill with a planar projection), as a fill layer with a decal projection.

use serde_json::{Value, json};
use texturecraft_core::{
    Channel, Decal, DecalFlip, DecalSymmetry, Effect, EffectKind, Fill, Layer, LayerId, LayerKind, Mask, MaskBackground, Projection, Source,
};
use texturecraft_geom::glam::{Quat, Vec2, Vec3};

use crate::commands::{CommandSpec, always, bad, boolp, channel_param, f32p, layer_param, strp, u64p, vec3};
use crate::tool_cmds::viewport_of;
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
            "{layer? | effect?: a mask fill effect id (a decal placed as a mask; with `layer`, it must be in that layer), x?, y?, viewport? | position?, normal? (re-place it on its own texture set), up?: [x,y,z] (default: the decal's up carried along when it moves onto a surface facing another way), size?: height | [w,h], keepProportions?: true keeps the image's aspect (the current ratio without an image): a height sets the width, a [w,h] pair or no size sets the height from the width, flipU?, flipV?: mirror the image left to right / top to bottom inside the frame, rotation?: degrees, offset?: [right, up] (world units from where it was placed), depth?, symmetry?: x|y|z|off, symmetryFlip?: none|u|v, symmetryOffset? (the mirrored copy; the flip and offset alone keep the axis), image?: replaces the image in the channel or mask it was placed in, merge?: true continues the previous Edit decal undo step (drags)}",
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

/// The mesh's diagonal (decal sizes are relative to it).
fn mesh_size(s: &Session) -> f32 {
    let (lo, hi) = s.mesh.bounds();
    (hi - lo).length().max(1e-3)
}

/// Width and height from a `size` param (a height keeping `aspect`, or `[w, h]`), within the decal size limits
fn size_param(p: &Value, aspect: f32, mesh_diagonal: f32) -> Option<[f32; 2]> {
    let value = p.get("size")?;
    let size = match value.as_array() {
        Some(pair) => [pair.first()?.as_f64()? as f32, pair.get(1)?.as_f64()? as f32],
        None => {
            let height = value.as_f64()? as f32;
            [height * aspect, height]
        }
    };
    size.iter().all(|side| side.is_finite()).then(|| size.map(|side| side.clamp(mesh_diagonal * MIN_SIZE, mesh_diagonal * MAX_SIZE)))
}

/// The decal's up direction: `up` when given, else the camera's (decals are placed upright on
/// screen), or where the camera looks when that is along `normal` (a top face seen from the
/// front: its far edge is up, as on a table).
fn decal_up(s: &Session, cmd: &str, p: &Value, normal: Vec3) -> Result<Vec3> {
    if p.get("up").is_some() {
        return vec3(p, "up").filter(|u| u.cross(normal).length_squared() > 1e-8).ok_or_else(|| bad(cmd, "`up` must be [x, y, z], not along the normal"));
    }
    let to_world = s.camera.view().inverse();
    let up = to_world.transform_vector3(Vec3::Y);
    Ok(if up.normalize_or_zero().cross(normal).length_squared() > 1e-4 { up } else { to_world.transform_vector3(Vec3::NEG_Z) })
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

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "layer.add.decal";
    let image = strp(p, "image").ok_or_else(|| bad(cmd, "`image` is required"))?.to_string();
    let resource = s.images.get(&image).ok_or_else(|| bad(cmd, format!("no image resource `{image}` (import it first)")))?;
    let aspect = resource.width as f32 / resource.height.max(1) as f32;
    let (center, normal, ts) = placement(s, cmd, p, s.ts(), false)?.ok_or_else(|| bad(cmd, "no placement"))?;

    // None: the image masks a new fill layer
    let target = match strp(p, "channel") {
        Some("mask") => None,
        _ => Some(channel_param(cmd, p, "channel")?.unwrap_or(Channel::BaseColor)),
    };
    let set_channels = s.doc.project.texture_set(ts)?.channel_list();
    if let Some(channel) = target
        && !set_channels.contains(&channel)
    {
        return Err(bad(cmd, format!("the texture set has no {} channel", channel.label())));
    }

    let mesh_diagonal = mesh_size(s);
    let default_height = mesh_diagonal * DEFAULT_HEIGHT;
    let mut size = size_param(p, aspect, mesh_diagonal).unwrap_or([default_height * aspect, default_height]);
    if boolp(p, "keepProportions") == Some(true) && p.get("size").is_some_and(Value::is_array) {
        size[1] = (size[0] / aspect).clamp(mesh_diagonal * MIN_SIZE, mesh_diagonal * MAX_SIZE);
    }
    let decal = Decal {
        center: center.to_array(),
        normal: normal.to_array(),
        up: decal_up(s, cmd, p, normal)?.to_array(),
        size,
        rotation: f32p(p, "rotation").unwrap_or(0.0).rem_euclid(360.0),
        offset: [0.0; 2],
        flip: [boolp(p, "flipU").unwrap_or(false), boolp(p, "flipV").unwrap_or(false)],

        // By default it reaches half its larger side into the surface
        depth: f32p(p, "depth").map_or(size[0].max(size[1]) * 0.5, |depth| depth.clamp(mesh_diagonal * MIN_SIZE, mesh_diagonal * MAX_SIZE)),
        symmetry: symmetry_param(cmd, p, None, s.mesh.bounds())?,
    };

    // A decal dropped on another texture set goes there and selects it, as a dropped material does
    if ts != s.ts() {
        crate::set_cmds::select_set(s, ts)?;
    }

    let base = strp(p, "name")
        .map(str::to_string)
        .unwrap_or_else(|| std::path::Path::new(&image).file_stem().and_then(|stem| stem.to_str()).unwrap_or("Decal").to_string());
    let name = s.doc.project.next_layer_name(ts, &base);
    let (id, mask_effect) = s.edit("Add decal", |s| {
        let id = s.doc.project.alloc_id();
        let Some(channel) = target else {
            let effect_id = s.doc.project.alloc_id();
            let mut layer = Layer::new(id, name, LayerKind::Fill(Fill::default()), &set_channels);
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(effect_id, EffectKind::Fill { source: Source::image(image.clone()), projection: Projection::Decal(decal) }));
            layer.mask = Some(mask);
            crate::layer_cmds::insert_selected(s, layer)?;
            s.target = StackTarget::Effect(effect_id);
            return Ok((id, Some(effect_id)));
        };

        let mut fill = Fill { projection: Projection::Decal(decal), ..Fill::default() };
        fill.sources.insert(channel, Source::image(image.clone()));
        crate::layer_cmds::insert_selected(s, Layer::new(id, name, LayerKind::Fill(fill), &[channel]))?;
        Ok((id, None))
    })?;
    Ok(json!({ "layer": id.0, "decal": decal, "maskEffect": mask_effect.map(|effect| effect.0) }))
}

/// The decal of fill layer `id`, or of mask fill effect `effect` when given
fn decal_of(s: &Session, cmd: &str, id: LayerId, effect: Option<LayerId>) -> Result<Decal> {
    if let Some(effect) = effect {
        return match s.doc.project.effect(effect).map(|(_, effect)| &effect.kind) {
            Some(EffectKind::Fill { projection: Projection::Decal(decal), .. }) => Ok(*decal),
            _ => Err(bad(cmd, "the effect is not a decal (layer.add.decal with channel mask)")),
        };
    }

    match &s.doc.project.layer(id)?.kind {
        LayerKind::Fill(Fill { projection: Projection::Decal(decal), .. }) => Ok(*decal),
        _ => Err(bad(cmd, "the layer is not a decal (layer.add.decal)")),
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

    let mut decal = decal_of(session, command, layer_id, effect)?;
    let texture_set =
        session.doc.project.locate(layer_id).map(|(texture_set, _, _)| texture_set).ok_or_else(|| bad(command, "the decal has no texture set"))?;
    let mesh_diagonal = mesh_size(session);
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
    let ratio = decal.size[0] / decal.size[1].max(1e-6);
    let aspect = if keep { image_aspect(session, layer_id, effect, strp(params, "image")).unwrap_or(ratio) } else { ratio };
    if let Some(mut size) = size_param(params, aspect, mesh_diagonal).or_else(|| keep.then_some(decal.size)) {
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
        decal.rotation = rotation.rem_euclid(360.0);
    }
    if let Some(offset) = params.get("offset").and_then(Value::as_array) {
        let values: Vec<f32> = offset.iter().filter_map(Value::as_f64).map(|value| value as f32).filter(|value| value.is_finite()).collect();
        let [right, up] = values.as_slice() else { return Err(bad(command, "`offset` must be [right, up]")) };

        decal.offset = [right.clamp(-mesh_diagonal, mesh_diagonal), up.clamp(-mesh_diagonal, mesh_diagonal)];
    }
    if let Some(depth) = f32p(params, "depth") {
        decal.depth = depth.clamp(mesh_diagonal * MIN_SIZE, mesh_diagonal * MAX_SIZE);
    }
    decal.symmetry = symmetry_param(command, params, decal.symmetry, session.mesh.bounds())?;

    let image = strp(params, "image").map(str::to_string);
    if let Some(name) = &image
        && !session.doc.resources.contains_key(name)
    {
        return Err(bad(command, format!("no image resource `{name}`")));
    }

    session.edit("Edit decal", |session| {
        if let Some(effect) = effect {
            if let Some(EffectKind::Fill { source, projection }) = session.doc.project.effect_mut(effect).map(|effect| &mut effect.kind) {
                *projection = Projection::Decal(decal);
                if let Some(image) = image {
                    *source = Source::image(image);
                }
            }
        } else if let LayerKind::Fill(fill) = &mut session.doc.project.layer_mut(layer_id)?.kind {
            fill.projection = Projection::Decal(decal);

            // The new image goes where the decal's image is (the channel it was placed in)
            let placed = fill.sources.values().find_map(|source| match source {
                Source::Image { name, .. } => Some(name.clone()),
                _ => None,
            });
            match (image, placed) {
                (Some(image), Some(placed)) => {
                    for source in fill.sources.values_mut() {
                        if let Source::Image { name, .. } = source
                            && *name == placed
                        {
                            name.clone_from(&image);
                        }
                    }
                }
                (Some(image), None) => {
                    fill.sources.insert(Channel::BaseColor, Source::image(image));
                }
                (None, _) => {}
            }
        }
        Ok(())
    })?;
    Ok(json!({ "layer": layer_id.0, "decal": decal }))
}
