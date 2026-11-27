//! `effect.generator.pickPoint`: a 3D Distance or 3D Linear Gradient point set from where the mesh is under a viewport point

use serde_json::{Value, json};
use texturecraft_core::{EffectKind, GeneratorKind, LayerId};
use texturecraft_geom::glam::{Vec2, Vec3};

use crate::commands::{CommandSpec, bad, f32p, has_layer, strp, u64p};
use crate::tool_cmds::viewport_of;
use crate::{Result, Session, cmd};

const CMD: &str = "effect.generator.pickPoint";

/// The least extent the mesh's bounds count with, so a flat mesh still places a point (the compositor's own floor)
const MIN_BOUNDS_EXTENT: f32 = 1e-6;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "effect.generator.pickPoint",
        "Pick generator point on mesh",
        [],
        None,
        "{effect, point?: distance|start|end (default distance on 3dDistance, start on 3dLinearGradient), x?, y?: viewport pixels (default its centre), viewport?: [w, h]} sets a 3D Distance or 3D Linear Gradient point to the mesh under that pixel, across the mesh's bounds as the generator reads it; returns the point",
        has_layer,
        pick_point
    )]
}

fn pick_point(session: &mut Session, params: &Value) -> Result<Value> {
    let effect = u64p(params, "effect").ok_or_else(|| bad(CMD, "`effect` is required"))?;
    let kind = match session.doc.project.effect(LayerId(effect)).map(|(_, effect)| &effect.kind) {
        Some(EffectKind::Generator(generator)) => generator.kind,
        Some(_) => return Err(bad(CMD, format!("effect {effect} is not a generator"))),
        None => return Err(bad(CMD, format!("no effect {effect}"))),
    };
    let point = strp(params, "point").map(str::to_ascii_lowercase);
    let key = match (kind, point.as_deref()) {
        (GeneratorKind::ThreeDDistance, None | Some("distance")) => "distance",
        (GeneratorKind::ThreeDLinearGradient, None | Some("start")) => "gradientStart",
        (GeneratorKind::ThreeDLinearGradient, Some("end")) => "gradientEnd",
        (GeneratorKind::ThreeDDistance | GeneratorKind::ThreeDLinearGradient, Some(other)) => {
            return Err(bad(CMD, format!("{} has no point `{other}`", kind.label())));
        }
        _ => return Err(bad(CMD, format!("{} has no point to pick: only 3D Distance and 3D Linear Gradient have one", kind.label()))),
    };

    let viewport = viewport_of(session, params);
    let screen = match (f32p(params, "x"), f32p(params, "y")) {
        (Some(x), Some(y)) => Vec2::new(x, y),
        (None, None) => viewport * 0.5,
        _ => return Err(bad(CMD, "give both `x` and `y`, or neither for the viewport's centre")),
    };
    let hit = session.pick(screen, viewport).ok_or_else(|| bad(CMD, "nothing under that point: pick on the mesh"))?;
    let picked = across_bounds(hit.pos, session.mesh.bounds()).to_array().map(|coordinate| coordinate.clamp(0.0, 1.0));

    let edit = match key {
        "distance" => json!({"effect": effect, "distanceX": picked[0], "distanceY": picked[1], "distanceZ": picked[2]}),
        _ => json!({"effect": effect, key: picked}),
    };
    crate::mask_cmds::set(session, &edit)?;
    Ok(json!({"effect": effect, "point": picked}))
}

/// `position` across `bounds` as the 3D generators read the mesh: scaled evenly by the longest side, centred on 0.5
pub fn across_bounds(position: Vec3, (low, high): (Vec3, Vec3)) -> Vec3 {
    let longest = (high - low).max_element().max(MIN_BOUNDS_EXTENT);
    (position - (low + high) * 0.5) / longest + Vec3::splat(0.5)
}
