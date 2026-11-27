//! Warp projections: the frame a new warp starts from, and Reset frame, which puts a warp's frame and corners back there

use serde_json::Value;
use texturecraft_core::{Decal, Projection, Projector};
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
        "{layer? | effect?: content or mask fill effect id, keepCorners?: bool} — refit the warp's frame around the mesh. With `keepCorners: true`, also reset the projector's rotation and scale and keep the corners; otherwise reset the corners. Projection Depth, crop, and culling stay",
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

fn reset_frame(session: &mut Session, params: &Value) -> Result<Value> {
    let target = fill_target(session, params, RESET_FRAME)?;
    let keep_corners = match params.get("keepCorners") {
        Some(value) => value.as_bool().ok_or_else(|| bad(RESET_FRAME, "`keepCorners` must be a boolean"))?,
        None => false,
    };
    let fitted = fitted_frame(&session.mesh);
    let (projection, _) = placement_mut(session, target)?;
    let mut reset = projection.clone();
    let Projection::Warp { frame, corners, projector, .. } = &mut reset else {
        return Err(bad(RESET_FRAME, "the fill's projection is not a warp projection"));
    };

    *frame = fitted;
    if keep_corners {
        // Older warps may still carry a transform separate from the frame
        let defaults = Projector::default();
        projector.rotation = defaults.rotation;
        projector.scale = defaults.scale;
    } else {
        *corners = Projection::FRAME_CORNERS;
    }

    // An unchanged projection adds no undo step
    if reset == *projection {
        return Ok(Value::Null);
    }

    let label = if keep_corners { "Reset warp transform" } else { "Reset frame" };
    session.edit(label, |session| {
        *placement_mut(session, target)?.0 = reset;
        Ok(())
    })?;

    Ok(Value::Null)
}
