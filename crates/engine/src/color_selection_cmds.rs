//! Color selection effects: the ID colors they select, picked from the ID mesh map

use serde_json::{Value, json};
use texturecraft_core::{BakedMap, ColorSelection, EffectKind, LayerId, SelectionBackground};
use texturecraft_geom::glam::Vec2;
use texturecraft_geom::surface_at;

use crate::commands::{CommandSpec, bad, f32p, has_layer, strp};
use crate::mask_cmds::effect_id;
use crate::tool_cmds::viewport_of;
use crate::{Result, Session, cmd};

/// Most colors one color selection holds; an ID map rarely has more distinct parts
const MAX_SELECTED_COLORS: usize = 256;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "effect.colorSelection.pick",
        "Pick ID color",
        [],
        None,
        "{effect?: a color selection (default: the selected effect), x?, y?: viewport pixels, viewport?: [w,h] | uv?: [u,v] texture set atlas UV (the 2D view), index?: replace this color instead of adding one} → {color: [r,g,b] as the ID map stores it, index} — samples the selected ID image or the texture set’s baked ID mesh map under the point",
        has_layer,
        pick
    )]
}

/// Applies `effect.set`'s color selection params; `id_map` is the checked `idMap` ([`id_map_param`])
pub(crate) fn apply_params(params: &Value, selection: &mut ColorSelection, id_map: Option<Option<String>>) -> Result<()> {
    const CMD: &str = "effect.set";
    if let Some(list) = params.get("colors") {
        selection.colors = colors_param(list)?;
    }
    for (key, value) in [("tolerance", &mut selection.tolerance), ("output", &mut selection.output), ("hardness", &mut selection.hardness)] {
        if let Some(given) = f32p(params, key) {
            *value = given.clamp(0.0, 1.0);
        }
    }
    if let Some(id) = strp(params, "background") {
        selection.background =
            SelectionBackground::from_id(id).ok_or_else(|| bad(CMD, format!("unknown background \"{id}\": transparent, black, white or gray")))?;
    }
    if let Some(id_map) = id_map {
        selection.id_map = id_map;
    }
    Ok(())
}

/// `effect.set`'s `idMap`: absent, null (the ID mesh map) or the name of an image resource
pub(crate) fn id_map_param(session: &Session, params: &Value) -> Result<Option<Option<String>>> {
    const CMD: &str = "effect.set";
    match params.get("idMap") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(name)) if session.images.contains_key(name) => Ok(Some(Some(name.clone()))),
        Some(Value::String(name)) => Err(bad(CMD, format!("no image resource \"{name}\": import it first"))),
        Some(_) => Err(bad(CMD, "`idMap` must be an image resource name or null")),
    }
}

/// A `[[r, g, b], ...]` list (empty selects nothing) of finite values, clamped to 0..1
fn colors_param(list: &Value) -> Result<Vec<[f32; 3]>> {
    const CMD: &str = "effect.set";
    let items = list.as_array().ok_or_else(|| bad(CMD, "`colors` must be a list of [r, g, b]"))?;
    if items.len() > MAX_SELECTED_COLORS {
        return Err(bad(CMD, format!("`colors` holds at most {MAX_SELECTED_COLORS} colors")));
    }

    items
        .iter()
        .map(|item| match finite_numbers::<3>(item) {
            Some(color) => Ok(color.map(|value| value.clamp(0.0, 1.0) as f32)),
            _ => Err(bad(CMD, "each color must be [r, g, b] with finite numbers")),
        })
        .collect()
}

/// Samples the ID map under a viewport point or at a UV and adds the color to a color selection
fn pick(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "effect.colorSelection.pick";
    let effect = effect_id(session, params, CMD)?;
    let (owner, color_count, id_map) = match session.doc.project.effect(effect) {
        Some((owner, found)) => match &found.kind {
            EffectKind::ColorSelection(selection) => (owner, selection.colors.len(), selection.id_map.clone()),
            other => return Err(bad(CMD, format!("{} is not a color selection", other.name()))),
        },
        None => return Err(bad(CMD, format!("no effect {}", effect.0))),
    };

    let index = params
        .get("index")
        .filter(|index| !index.is_null())
        .map(|index| index.as_u64().ok_or_else(|| bad(CMD, "`index` must be a non-negative integer")))
        .transpose()?;
    let index = match index {
        Some(index) => Some(usize::try_from(index).ok().filter(|index| *index < color_count).ok_or_else(|| bad(CMD, format!("no color {index} to replace")))?),
        None if color_count >= MAX_SELECTED_COLORS => return Err(bad(CMD, format!("a color selection holds at most {MAX_SELECTED_COLORS} colors"))),
        None => None,
    };

    let color = id_color_at(session, params, owner, id_map.as_deref())?;
    session.edit("Pick ID color", |session| {
        let edited = session.doc.project.effect_mut(effect).ok_or_else(|| bad(CMD, format!("no effect {}", effect.0)))?;
        if let EffectKind::ColorSelection(selection) = &mut edited.kind {
            match index.and_then(|index| selection.colors.get_mut(index)) {
                Some(slot) => *slot = color,
                None => selection.colors.push(color),
            }
        }
        Ok(())
    })?;
    Ok(json!({ "color": color, "index": index.unwrap_or(color_count) }))
}

/// The ID map color of the texel a color selection on `owner` reading `id_map` (none: the ID mesh map) reads at the picked point
fn id_color_at(session: &Session, params: &Value, owner: LayerId, id_map: Option<&str>) -> Result<[f32; 3]> {
    const CMD: &str = "effect.colorSelection.pick";
    let (set_index, _, _) = session.doc.project.locate(owner).ok_or_else(|| bad(CMD, "the effect's layer is not in a texture set"))?;
    let set = session.doc.project.texture_set(set_index)?;
    let baked = session.baked(set_index).filter(|baked| baked.maps.contains_key(&BakedMap::Id));
    let image = match id_map {
        Some(name) => Some(session.images.get(name).ok_or_else(|| bad(CMD, format!("the ID map image \"{name}\" is gone: choose another")))?),
        None => None,
    };
    if image.is_none() && baked.is_none() {
        return Err(bad(CMD, format!("texture set \"{}\" has no ID map: bake one, or set one with textureSet.meshMap.set", set.name)));
    }

    let uv = match params.get("uv") {
        Some(uv) => uv_param(uv)?,
        None => {
            let (Some(x), Some(y)) = (f32p(params, "x"), f32p(params, "y")) else {
                return Err(bad(CMD, "give `x` and `y` (viewport pixels) or `uv`"));
            };

            let (uv, material) = visible_surface(session, Vec2::new(x, y), viewport_of(session, params))
                .ok_or_else(|| bad(CMD, "nothing under that point: pick on the mesh"))?;
            if material != set.material {
                return Err(bad(CMD, "that surface belongs to another texture set"));
            }

            uv
        }
    };

    // The same texel the mask evaluates, so a picked color always selects where it was picked
    let size = set.size();
    let texel = [uv.x * size.0 as f32, uv.y * size.1 as f32];
    if !texel.iter().zip([size.0, size.1]).all(|(position, extent)| (0.0..extent as f32).contains(position)) {
        return Err(bad(CMD, "that point is outside the texture set's UV space"));
    }

    let (x, y) = (texel[0] as u32, texel[1] as u32);
    let color = match (image, baked) {
        (Some(image), _) => {
            // The texel's centre, as the mask reads it
            let [red, green, blue, _] = image.sample_nearest((x as f32 + 0.5) / size.0.max(1) as f32, (y as f32 + 0.5) / size.1.max(1) as f32);
            Some([red, green, blue])
        }
        (None, Some(baked)) => baked.sample_working(BakedMap::Id, x, y, size, &set.block_scales()),
        (None, None) => None,
    };
    color.ok_or_else(|| bad(CMD, "no ID color there"))
}

/// The UV and mesh material under viewport pixel `screen`; hidden texture sets don't stop the ray
fn visible_surface(session: &Session, screen: Vec2, viewport: Vec2) -> Option<(Vec2, usize)> {
    let viewport = viewport.max(Vec2::ONE);
    let device = Vec2::new(screen.x / viewport.x * 2.0 - 1.0, 1.0 - screen.y / viewport.y * 2.0);
    let ray = session.camera.ray(device, viewport.x / viewport.y);
    let sets = &session.doc.project.texture_sets;
    let visible = |triangle: u32| {
        let material = session.mesh.material_of(triangle as usize).unwrap_or(0);
        sets.iter().find(|set| set.material == material).is_none_or(|set| set.visible)
    };

    let hit = session.bvh.closest_filtered(ray.origin, ray.dir, f32::MAX, &visible)?;
    let surface = surface_at(&session.mesh, hit.tri, hit.u, hit.v)?;
    Some((surface.uv, session.mesh.material_of(hit.tri as usize).unwrap_or(0)))
}

/// A `[u, v]` param of finite numbers
fn uv_param(uv: &Value) -> Result<Vec2> {
    match finite_numbers::<2>(uv) {
        Some([u, v]) => Ok(Vec2::new(u as f32, v as f32)),
        _ => Err(bad("effect.colorSelection.pick", "`uv` must be [u, v] with finite numbers")),
    }
}

/// Validate the shape before reading components, without allocating for oversized input
fn finite_numbers<const N: usize>(list: &Value) -> Option<[f64; N]> {
    let items: &[Value; N] = list.as_array()?.as_slice().try_into().ok()?;
    let mut numbers = [0.0; N];
    for (number, item) in numbers.iter_mut().zip(items) {
        *number = item.as_f64().filter(|value| value.is_finite())?;
    }
    Some(numbers)
}
