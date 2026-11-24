//! Reset of a fill's Fill section: its projection's settings and its sampling back to a new fill's, its projection kind, symmetry and material kept

use serde_json::Value;
use texturecraft_core::{Projection, Sampling};

use crate::commands::{CommandSpec, has_layer};
use crate::layer_cmds::{UNFRAMED_PROJECTION_IDS, fill_target, new_unframed_projection, placement_mut};
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "layer.fill.resetProjection",
        "Reset fill settings",
        [],
        None,
        "{layer? | effect?: content or mask fill effect id} — the Fill section back to a new fill's in one undo step: the same projection kind with its defaults (tiling 1, no offset or rotation, a planar or warp frame stays where it is placed, a warp's corners unpulled), the default filtering, wrap and scale mode (symmetry, sources and material stay)",
        has_layer,
        reset_projection
    )]
}

/// The sampling a reset Fill section has: a new fill's, with `current`'s symmetry (it belongs to the Symmetry section)
pub fn reset_sampling(current: Sampling) -> Sampling {
    Sampling { symmetry: current.symmetry, ..Sampling::default() }
}

/// The defaults of `current`'s projection kind: a new fill's projection of that kind, with the frame a placed planar or a warp projects from kept
pub fn reset_projection_of(current: &Projection) -> Projection {
    if let Projection::Warp { frame, .. } = current {
        return Projection::warp(*frame);
    }

    // The new projection of the same kind; one no id makes (none today) stays as it is
    let same_kind = UNFRAMED_PROJECTION_IDS
        .iter()
        .filter_map(|id| new_unframed_projection(id))
        .find(|fresh| std::mem::discriminant(fresh) == std::mem::discriminant(current));
    let Some(mut fresh) = same_kind else { return current.clone() };

    if let (Projection::Planar { placed, projector, .. }, Projection::Planar { placed: Some(frame), projector: previous_projector, .. }) = (&mut fresh, current)
    {
        *placed = Some(*frame);
        projector.ignore_image_alpha = previous_projector.ignore_image_alpha;
    }

    fresh
}

/// Whether the Fill section of a fill projected by `projection` and sampled by `sampling` is already a new fill's (of the same projection kind)
pub fn at_defaults(projection: &Projection, sampling: Sampling) -> bool {
    *projection == reset_projection_of(projection) && sampling == reset_sampling(sampling)
}

fn reset_projection(s: &mut Session, p: &Value) -> Result<Value> {
    let target = fill_target(s, p, "layer.fill.resetProjection")?;
    let (projection, sampling) = placement_mut(s, target)?;

    if at_defaults(projection, *sampling) {
        return Ok(Value::Null);
    }

    s.edit("Reset fill settings", |s| {
        let (projection, sampling) = placement_mut(s, target)?;
        *projection = reset_projection_of(projection);
        *sampling = reset_sampling(*sampling);
        Ok(())
    })?;

    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use texturecraft_core::{Decal, Projector};

    use super::*;

    #[test]
    fn every_projection_kind_resets_to_its_own_kind() {
        let frame = Decal {
            center: [0.0; 3],
            normal: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            size: [1.0, 1.0],
            rotation: 0.0,
            offset: [0.0; 2],
            depth: 0.5,
            symmetry: None,
            flip: [false; 2],
        };
        let mut kinds: Vec<Projection> = UNFRAMED_PROJECTION_IDS.iter().filter_map(|id| new_unframed_projection(id)).collect();

        assert_eq!(kinds.len(), UNFRAMED_PROJECTION_IDS.len(), "every listed id makes a projection");

        kinds.extend([Projection::warp(frame), Projection::decal(frame)]);
        for kind in kinds {
            let turned = match kind.clone() {
                Projection::Planar { scale, axis, offset, uv, placed, .. } => {
                    Projection::Planar { scale: scale * 2.0, axis, offset, uv, placed, projector: Projector { rotation: [10.0; 3], ..Projector::default() } }
                }
                other => other,
            };

            let reset = reset_projection_of(&turned);

            assert_eq!(std::mem::discriminant(&reset), std::mem::discriminant(&kind), "{kind:?}");
            assert_eq!(reset.frame(), kind.frame(), "the frame stays where it is placed");
        }
    }
}
