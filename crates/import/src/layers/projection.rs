//! A fill's projection settings (`DataActionFill`) as TextureCraft's [`Projection`], with how exact the mapping is.
//!
//! | File | TextureCraft | Status |
//! |---|---|---|
//! | `projection` 6 (texture space) | UV, identity | exact |
//! | `projection` 0, identity UV transform | UV, identity | exact |
//! | `projection` 0 with tiling, offset or rotation | UV with that placement (radians → degrees) | approximated: the rotation and offset conventions are not verified |
//! | `projection` 2 (tri-planar) | tri-planar | approximated: TextureCraft's volume, blend curve and frame differ |
//! | `projection` 1, 3, 4 (planar, spherical, cylindrical) | planar, spherical, cylindrical | approximated |
//! | anything else (warp grids, unknown) | UV, identity | approximated |
//! | `uvSamplingWrap` other than "both" | TextureCraft always repeats | approximated where the placement can show it |

use texturecraft_core::Projection;
use texturecraft_import_reader::model::FillEffect;

use crate::status::Status;

// File-format data: the `projection` enum of a fill.
const PROJECTION_UV: u64 = 0;
const PROJECTION_PLANAR: u64 = 1;
const PROJECTION_TRIPLANAR: u64 = 2;
const PROJECTION_SPHERICAL: u64 = 3;
const PROJECTION_CYLINDRICAL: u64 = 4;
const PROJECTION_TEXTURE_SPACE: u64 = 6;
/// `uvSamplingWrap` value repeating in both directions.
const WRAP_BOTH: u64 = 3;
/// TextureCraft's tri-planar sharpness for the file's default side hardness (0.5).
const DEFAULT_SHARPNESS: f32 = 4.0;

/// The mapped projection and how exact it is.
pub struct MappedProjection {
    pub projection: Projection,
    pub status: Status,
    /// True for a volume projection (planar, tri-planar, spherical, cylindrical): placed in 3D, not on the UVs.
    pub volume: bool,
}

pub fn map(fill: &FillEffect) -> MappedProjection {
    let identity = Projection::default();
    let uv = fill.uv_transform.filter(|t| !t.is_identity());
    let volume_scale = fill.placement.size.map_or(1.0, |s| {
        let m = s.iter().copied().filter(|v| v.is_finite() && *v > 0.0).fold(0.0f32, f32::max);
        if m > 0.0 { 1.0 / m } else { 1.0 }
    });
    let approx = |what: &str| Status::approximated(what.to_string());
    let (projection, status, volume) = match fill.projection {
        PROJECTION_TEXTURE_SPACE => (identity, Status::Exact, false),
        PROJECTION_UV => match uv {
            None => (identity, Status::Exact, false),
            Some(t) => {
                let scale = t.scale.map(|v| if v.is_finite() { v } else { 1.0 });
                let offset = t.translation.map(|v| if v.is_finite() { v } else { 0.0 });
                let rotation = if t.rotation.is_finite() { t.rotation.to_degrees() } else { 0.0 };
                let mut status = approx("UV projection with tiling, offset or rotation (placement convention not verified)");
                if fill.placement.uv_wrap.is_some_and(|w| w != WRAP_BOTH) {
                    status.and(&approx("UV projection without repeat (TextureCraft repeats)"));
                }
                (Projection::Uv { scale, rotation, offset }, status, false)
            }
        },
        PROJECTION_TRIPLANAR => {
            let sharpness =
                fill.placement.side_hardness.filter(|h| h.is_finite()).map_or(DEFAULT_SHARPNESS, |h| 1.0 + h.clamp(0.0, 1.0) * 2.0 * (DEFAULT_SHARPNESS - 1.0));
            (
                Projection::Triplanar { scale: volume_scale, sharpness, offset: [0.0; 3] },
                approx("tri-planar projection (TextureCraft's volume and blend differ)"),
                true,
            )
        }
        PROJECTION_PLANAR => {
            (Projection::Planar { scale: volume_scale, axis: 2, offset: [0.0; 3] }, approx("planar projection (volume placement not taken over)"), true)
        }
        PROJECTION_SPHERICAL => (Projection::Spherical { scale: volume_scale }, approx("spherical projection (volume placement not taken over)"), true),
        PROJECTION_CYLINDRICAL => {
            (Projection::Cylindrical { scale: volume_scale, axis: 1 }, approx("cylindrical projection (volume placement not taken over)"), true)
        }
        other => (identity, Status::approximated(format!("projection mode {other} imported as texture space")), false),
    };
    MappedProjection { projection, status, volume }
}

/// True when a uniform fill's coverage is shaped by its projection (clipped volume, depth or facing falloffs): TextureCraft
/// lays a uniform value on everywhere.
pub fn shapes_coverage(fill: &FillEffect) -> bool {
    let p = &fill.placement;
    let volume = matches!(fill.projection, PROJECTION_PLANAR..=PROJECTION_CYLINDRICAL);
    volume
        && (p.volume_wrap == Some(0)
            || p.depth_culling == Some(true)
            || p.backface_culling == Some(true)
            || (fill.projection == PROJECTION_CYLINDRICAL && p.cylinder_hide_caps == Some(true)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_import_reader::model::{EffectHeader, FillPlacement, UvTransform};

    fn fill(projection: u64, uv: Option<UvTransform>) -> FillEffect {
        FillEffect { header: EffectHeader::default(), sources: Vec::new(), projection, filtering: 2, uv_transform: uv, placement: FillPlacement::default() }
    }

    #[test]
    fn texture_space_and_identity_uv_are_exact() {
        assert!(map(&fill(6, None)).status.is_exact());
        let id = UvTransform { scale: [1.0, 1.0], translation: [0.0, 0.0], rotation: 0.0 };
        let m = map(&fill(0, Some(id)));
        assert!(m.status.is_exact() && m.projection == Projection::default() && !m.volume);
    }

    #[test]
    fn placements_and_volumes_are_approximated() {
        let t = UvTransform { scale: [2.0, 2.0], translation: [0.5, 0.0], rotation: std::f32::consts::FRAC_PI_2 };
        let m = map(&fill(0, Some(t)));
        assert!(!m.status.is_exact());
        let Projection::Uv { scale, rotation, offset } = m.projection else { panic!("uv") };
        assert_eq!((scale, offset), ([2.0, 2.0], [0.5, 0.0]));
        assert!((rotation - 90.0).abs() < 1e-4);
        let tri = map(&fill(2, None));
        assert!(tri.volume && matches!(tri.projection, Projection::Triplanar { .. }) && !tri.status.is_exact());
        assert!(!map(&fill(7, None)).status.is_exact());
        let mut clipped = fill(1, None);
        clipped.placement.volume_wrap = Some(0);
        assert!(shapes_coverage(&clipped) && !shapes_coverage(&fill(0, None)));
    }
}
