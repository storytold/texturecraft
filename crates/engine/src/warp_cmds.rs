//! Warp projections: the frame a new warp starts from, and Reset frame, which puts a warp's frame and corners back there

use serde_json::Value;
use texturecraft_core::{Decal, Projection};
use texturecraft_geom::Mesh;

use crate::commands::{CommandSpec, bad, has_layer};
use crate::layer_cmds::{fill_target, placement_mut};
use crate::{Result, Session, cmd};

/// Reset frame's command id, the prefix of its errors
const RESET_FRAME: &str = "layer.fill.resetFrame";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "layer.fill.resetFrame",
        "Reset frame",
        [],
        None,
        "{layer? | effect?: content or mask fill effect id} — a warp projection's frame back around the mesh, seen from the front, with its corners unpulled (Projection Depth and the other settings stay)",
        has_layer,
        reset_frame
    )]
}

/// The frame a new warp projection starts from: around the whole mesh, seen from the front (+Z), up along +Y
pub(crate) fn fitted_frame(mesh: &Mesh) -> Decal {
    let (lo, hi) = mesh.bounds();
    let smallest = ((hi - lo).length() * SMALLEST_SIDE).max(f32::MIN_POSITIVE);
    let size = [(hi.x - lo.x).max(smallest), (hi.y - lo.y).max(smallest)];
    Decal {
        center: ((lo + hi) * 0.5).to_array(),
        normal: [0.0, 0.0, 1.0],
        up: [0.0, 1.0, 0.0],
        size,
        rotation: 0.0,
        offset: [0.0; 2],
        depth: 0.0,
        symmetry: None,
        flip: [false; 2],
    }
}

/// A fitted frame's smallest side, relative to the mesh diagonal (a flat mesh still gets a frame)
const SMALLEST_SIDE: f32 = 1e-3;

fn reset_frame(s: &mut Session, p: &Value) -> Result<Value> {
    let target = fill_target(s, p, RESET_FRAME)?;
    let fitted = fitted_frame(&s.mesh);
    let (Projection::Warp { frame, corners, .. }, _) = placement_mut(s, target)? else {
        return Err(bad(RESET_FRAME, "the fill's projection is not a warp projection"));
    };

    // A frame already fitted with unpulled corners adds no undo step
    if *frame == fitted && *corners == Projection::FRAME_CORNERS {
        return Ok(Value::Null);
    }

    s.edit("Reset frame", |s| {
        let (Projection::Warp { frame, corners, .. }, _) = placement_mut(s, target)? else {
            return Err(bad(RESET_FRAME, "the fill's projection is not a warp projection"));
        };

        *frame = fitted;
        *corners = Projection::FRAME_CORNERS;
        Ok(())
    })?;
    Ok(Value::Null)
}
