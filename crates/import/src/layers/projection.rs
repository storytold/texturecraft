//! A fill's projection settings (`DataActionFill`) as TextureCraft's [`Projection`], with how exact the mapping is.
//!
//! | File | TextureCraft | Status |
//! |---|---|---|
//! | `projection` 6 (texture space) | UV, identity | exact |
//! | `projection` 0, identity UV transform | UV, identity | exact |
//! | `projection` 0 with tiling, offset or rotation | UV with that placement about the texture's centre ([`uv_placement`]) | exact; approximated with a rotation (its direction is not verified) |
//! | `projection` 2 (tri-planar) | tri-planar | approximated: TextureCraft's volume, blend curve and frame differ |
//! | `projection` 1, 3, 4 (planar, spherical, cylindrical) | planar, spherical, cylindrical | approximated |
//! | anything else (warp grids, unknown) | UV, identity | approximated |
//! | `uvSamplingWrap` other than "both" | TextureCraft always repeats | approximated where the placement can show it |

use texturecraft_core::Projection;
use texturecraft_import_reader::model::{FillEffect, UvTransform};

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

/// The file's UV placement as TextureCraft's `uv · R(rotation) · scale + offset`. The original tiles, offsets and turns
/// about the texture's centre: image = `tiling · R(−θ) · (uv − ½ − offset) + ½` (measured: a mask fill tiled 4.37 times
/// with offset 0.22 lands on the stored result only so). The offset is folded into TextureCraft's and wrapped to 0..1
/// (the image repeats).
fn uv_placement(t: &UvTransform) -> ([f32; 2], f32, [f32; 2]) {
    let scale = t.scale.map(|v| if v.is_finite() { v } else { 1.0 });
    let [ox, oy] = t.translation.map(|v| if v.is_finite() { v } else { 0.0 });
    let theta = if t.rotation.is_finite() { t.rotation } else { 0.0 };
    let (s, c) = theta.sin_cos();
    // R(−θ) applied to the centre and offset, then scaled per axis
    let (px, py) = (0.5 + ox, 0.5 + oy);
    let r = [c * px + s * py, -s * px + c * py];
    let offset = [0.5 - scale[0] * r[0], 0.5 - scale[1] * r[1]].map(|v| if v.is_finite() { v.rem_euclid(1.0) } else { 0.0 });
    (scale, -theta.to_degrees(), offset)
}

/// `volume_unit`: [`super::SetInput::volume_unit`].
pub fn map(fill: &FillEffect, volume_unit: f32) -> MappedProjection {
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
                let (scale, rotation, offset) = uv_placement(&t);
                let mut status = match rotation == 0.0 {
                    true => Status::Exact,
                    false => approx("UV projection with a rotation (its direction not verified)"),
                };
                if fill.placement.uv_wrap.is_some_and(|w| w != WRAP_BOTH) {
                    status.and(&approx("UV projection without repeat (TextureCraft repeats)"));
                }
                (Projection::Uv { scale, rotation, offset }, status, false)
            }
        },
        PROJECTION_TRIPLANAR => {
            let sharpness =
                fill.placement.side_hardness.filter(|h| h.is_finite()).map_or(DEFAULT_SHARPNESS, |h| 1.0 + h.clamp(0.0, 1.0) * 2.0 * (DEFAULT_SHARPNESS - 1.0));
            // the original's volume of size 1 is the scene's largest extent, centred on the scene (TextureCraft's positions are
            // centred too, over the bounding radius: `volume_unit`), and holds one repetition of the image; the UV tiling
            // repeats it as on a UV fill. TextureCraft's tri-planar has one scale, so uneven tiling takes the mean; the half
            // offset puts the image's centre on the scene's centre
            let tiling = uv.map_or(1.0, |t| {
                let s = t.scale.map(|v| if v.is_finite() { v.abs() } else { 1.0 });
                (s[0] + s[1]) * 0.5
            });
            (
                Projection::Triplanar { scale: volume_scale * tiling * volume_unit, sharpness, offset: [0.5; 3] },
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
        assert!(map(&fill(6, None), 1.0).status.is_exact());
        let id = UvTransform { scale: [1.0, 1.0], translation: [0.0, 0.0], rotation: 0.0 };
        let m = map(&fill(0, Some(id)), 1.0);
        assert!(m.status.is_exact() && m.projection == Projection::default() && !m.volume);
    }

    /// TextureCraft's placement of `uv` (as the compositor applies [`Projection::Uv`]).
    fn placed(p: &Projection, uv: [f32; 2]) -> [f32; 2] {
        let Projection::Uv { scale, rotation, offset } = p else { panic!("uv") };
        let (sn, cs) = rotation.to_radians().sin_cos();
        [(uv[0] * cs - uv[1] * sn) * scale[0] + offset[0], (uv[0] * sn + uv[1] * cs) * scale[1] + offset[1]]
    }

    #[test]
    fn tiling_and_offset_place_about_the_centre_as_the_original() {
        let t = UvTransform { scale: [4.37, 3.0], translation: [0.22, -0.1], rotation: 0.0 };
        let m = map(&fill(0, Some(t)), 1.0);
        assert!(m.status.is_exact());
        for uv in [[0.0, 0.0], [0.5, 0.5], [0.13, 0.71], [0.9, 0.2]] {
            // the original: tiling · (uv − ½ − offset) + ½, the same texel of the repeating image
            let want = [4.37 * (uv[0] - 0.5 - 0.22) + 0.5, 3.0 * (uv[1] - 0.5 + 0.1) + 0.5];
            let got = placed(&m.projection, uv);
            for k in 0..2 {
                let d = (got[k] - want[k]).rem_euclid(1.0);
                assert!(d.min(1.0 - d) < 1e-4, "{uv:?}: {got:?} vs {want:?}");
            }
        }
        // a turn about the centre keeps the centre in place
        let turned = map(&fill(0, Some(UvTransform { scale: [1.0, 1.0], translation: [0.0, 0.0], rotation: 0.7 })), 1.0);
        let c = placed(&turned.projection, [0.5, 0.5]);
        assert!((c[0].rem_euclid(1.0) - 0.5).abs() < 1e-4 && (c[1].rem_euclid(1.0) - 0.5).abs() < 1e-4, "{c:?}");
        assert!(!turned.status.is_exact());
    }

    #[test]
    fn tri_planar_repeats_by_its_tiling_in_scene_units() {
        let t = UvTransform { scale: [3.0, 3.0], translation: [0.0, 0.0], rotation: 0.0 };
        let Projection::Triplanar { scale, offset, .. } = map(&fill(2, Some(t)), 0.75).projection else { panic!("tri-planar") };
        assert!((scale - 3.0 * 0.75).abs() < 1e-5, "{scale}");
        assert_eq!(offset, [0.5; 3], "the image's centre on the scene's centre");
        let Projection::Triplanar { scale, .. } = map(&fill(2, None), 1.0).projection else { panic!("tri-planar") };
        assert_eq!(scale, 1.0);
    }

    #[test]
    fn placements_and_volumes_are_approximated() {
        let t = UvTransform { scale: [2.0, 2.0], translation: [0.5, 0.0], rotation: std::f32::consts::FRAC_PI_2 };
        let m = map(&fill(0, Some(t)), 1.0);
        assert!(!m.status.is_exact());
        let Projection::Uv { scale, rotation, .. } = m.projection else { panic!("uv") };
        assert_eq!(scale, [2.0, 2.0]);
        assert!((rotation + 90.0).abs() < 1e-4);
        let tri = map(&fill(2, None), 1.0);
        assert!(tri.volume && matches!(tri.projection, Projection::Triplanar { .. }) && !tri.status.is_exact());
        assert!(!map(&fill(7, None), 1.0).status.is_exact());
        let mut clipped = fill(1, None);
        clipped.placement.volume_wrap = Some(0);
        assert!(shapes_coverage(&clipped) && !shapes_coverage(&fill(0, None)));
    }
}
