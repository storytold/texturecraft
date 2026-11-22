//! A fill's projection settings (`DataActionFill`) as TextureCraft's [`Projection`], with how exact the mapping is.
//!
//! | File | TextureCraft | Status |
//! |---|---|---|
//! | `projection` 6 (texture space) | UV, identity | exact |
//! | `projection` 0, identity UV transform | UV, identity | exact |
//! | `projection` 0 with tiling, offset or rotation | UV with that placement about the texture's centre, v up ([`uv_placement`]) | exact |
//! | `projection` 0 with `uvSamplingWrap` 0, 1, 2 | the same placement as a [`PlacedMode::Uv`] repeating along none, u or v | as above; 1 and 2 approximated (which is u and which v is not verified) |
//! | `projection` 2 (tri-planar) | [`PlacedMode::TriPlanar`]: the volume ([`volume`]), `sideHardness` as the blend hardness, UV tiling and rotation as the image placement | exact (measured, see below); approximated with a UV offset (left out: its direction is not measured on tri-planar), tiling uneven and rotation together (their order is unmeasured), a size mode other than tiling, or a clipped volume |
//! | `projection` 1 (planar) | [`PlacedMode::Planar`] with the depth (`depthCulling*`) and facing (`backfaceCulling*`, angle × 180°) falloffs | exact without falloffs or clipping (the volume and image placement measured as tri-planar's); approximated with them (the falloff curves are not measured) |
//! | `projection` 3, 4 (spherical, cylindrical) | [`PlacedMode::Spherical`], [`PlacedMode::Cylindrical`] (`cylinderCullingAngle` × 360°, `cylinderHideCaps`) | approximated: the modes are not measured |
//! | `projection` 7 (warp) + `dddGrid` | [`PlacedMode::Warp`]: the volume as above, `gridSize` (columns, rows), the vertices row by row with `position3D` as grid positions, `positionUV` (v up) as the image coordinate (u, 1 − v), the four tangents as handles; `depth` the projection depth, `depthCulling*` its falloff | approximated: TextureCraft's own Bézier warp, not measured |
//! | `projection` 7 with a malformed grid, anything else | UV, identity | approximated |
//! | `filtering` 2, 0 | prefiltered, bilinear ([`bilinear`]) | exact; other values approximated as prefiltered |
//!
//! The volume of the volume modes, measured with tri-planar fills on an off-centre box (each plane's image
//! coordinates read back from ramps, exact to the texel): its frame is the whole mesh's bounding box, one scene unit its
//! largest extent; `dddPosition` moves the volume by half scene units, `dddSize` scales it (1: one image repetition
//! across the largest extent), `dddEulerAngles` (radians) turn it about z first, then y, then x. The UV tiling repeats
//! the image about its centre, the rotation turns it the other way from TextureCraft's degrees and the offset's v runs
//! up. The blend shares are `smoothstep(½ + ½·hardness, 1, |n_i| / max|n|)`, normalised (measured on a sphere at
//! hardness 0, 0.25, 0.5, 0.75 and 1 to 0.002 rms in the shares).
//!
//! Warp volume positions refer to the stored scene mesh's normalised coordinates, with a unit of two for the size.
//! Convert that frame into TextureCraft's bounding-box frame, which can have a different centre and extent.

use texturecraft_core::{FacingFalloff, Falloff, ImagePlacement, PlacedMode, PlacedProjection, Projection, Repeat, Volume, WarpGrid, WarpPoint};
use texturecraft_geom::{
    Mesh,
    glam::{Mat3, Vec3},
};
use texturecraft_import_reader::model::{FillEffect, FillPlacement, GridDeformation, UvTransform};

use crate::status::Status;

// File-format data: the `projection` enum of a fill.
const PROJECTION_UV: u64 = 0;
const PROJECTION_PLANAR: u64 = 1;
const PROJECTION_TRIPLANAR: u64 = 2;
const PROJECTION_SPHERICAL: u64 = 3;
const PROJECTION_CYLINDRICAL: u64 = 4;
const PROJECTION_TEXTURE_SPACE: u64 = 6;
const PROJECTION_WARP: u64 = 7;
// File-format data: `uvSamplingWrap` (repeat none, u, v, both).
const WRAP_NONE: u64 = 0;
const WRAP_U: u64 = 1;
const WRAP_V: u64 = 2;
const WRAP_BOTH: u64 = 3;
// File-format data: `filtering` (bilinear, smooth).
const FILTERING_BILINEAR: u64 = 0;
const FILTERING_SMOOTH: u64 = 2;
/// The warp grid where a file stores none: 3 × 3 regular anchors reaching 0.1 grid units on either side.
const DEFAULT_GRID: WarpGrid = WarpGrid { rows: 3, columns: 3, depth: 0.1, points: Vec::new() };
/// The settings' defaults where a file leaves them out (the original's slider defaults).
const DEFAULT_HARDNESS: f32 = 0.5;
const DEFAULT_BACKFACE_ANGLE: f32 = 0.5;
/// The backface culling angle is stored as a share of a half turn, the cylinder's coverage as a share of a whole turn.
const HALF_TURN_DEGREES: f32 = 180.0;
const WHOLE_TURN_DEGREES: f32 = 360.0;

/// The mapped projection and how exact it is.
pub struct MappedProjection {
    pub projection: Projection,
    pub status: Status,
    /// True for a volume projection (planar, tri-planar, spherical, cylindrical): placed in 3D, not on the UVs.
    pub volume: bool,
}

/// The file's UV placement as TextureCraft's `uv · R(rotation) · scale + offset`. The original tiles, offsets and turns
/// about the texture's centre with v up: image = `tiling · R(−θ) · (uv − ½ − offset) + ½` in v-up coordinates
/// (measured: a mask fill tiled 4.37 times with offset 0.22 lands on the stored result only so; a decal tiled −5.6 ×
/// −5.2, turned 1.28 rad with offset (−0.30, −0.07) and not repeated lands on its stored copy only so). In
/// TextureCraft's texel rows (v down) that is `tiling · R(θ) · (uv − ½ − (offset_u, −offset_v)) + ½`. The offset is
/// folded into TextureCraft's and wrapped to 0..1 when the image repeats both ways.
fn uv_placement(t: &UvTransform, wrap: bool) -> ([f32; 2], f32, [f32; 2]) {
    let scale = t.scale.map(|v| if v.is_finite() { v } else { 1.0 });
    let [ox, oy] = t.translation.map(|v| if v.is_finite() { v } else { 0.0 });
    let theta = if t.rotation.is_finite() { t.rotation } else { 0.0 };
    let (s, c) = theta.sin_cos();
    // R(θ) applied to the centre and offset (v down), then scaled per axis
    let (px, py) = (0.5 + ox, 0.5 - oy);
    let r = [c * px - s * py, s * px + c * py];
    let offset = [0.5 - scale[0] * r[0], 0.5 - scale[1] * r[1]].map(|v| match v.is_finite() {
        true if wrap => v.rem_euclid(1.0),
        true => v,
        false => 0.0,
    });
    (scale, theta.to_degrees(), offset)
}

/// The file's fill filtering 0 (bilinear): the fill's graph is read at one point per texel, not prefiltered (2, the
/// default, smooth, prefilters a minified image).
pub fn bilinear(fill: &FillEffect) -> bool {
    fill.filtering == FILTERING_BILINEAR
}

/// The rotation turning by `radians` about z, then y, then x (the original's order), as TextureCraft's degrees about x,
/// y and z applied in that order (`R = Rz·Ry·Rx`). At gimbal lock the turn about x is folded into z.
fn volume_rotation(radians: [f32; 3]) -> [f32; 3] {
    let [x, y, z] = radians.map(|a| if a.is_finite() { a } else { 0.0 });
    let m = Mat3::from_rotation_x(x) * Mat3::from_rotation_y(y) * Mat3::from_rotation_z(z);
    // m.col(c)[r] is row r, column c; R = Rz Ry Rx has m[2][0] = −sin y, m[2][1] = cos y sin x, m[2][2] = cos y cos x,
    // m[1][0] = cos y sin z, m[0][0] = cos y cos z
    let at = |r: usize, c: usize| m.col(c)[r];
    let sy = (-at(2, 0)).clamp(-1.0, 1.0);
    let angles =
        if sy.abs() < 0.999_999 { [at(2, 1).atan2(at(2, 2)), sy.asin(), at(1, 0).atan2(at(0, 0))] } else { [0.0, sy.asin(), (-at(0, 1)).atan2(at(1, 1))] };
    angles.map(|a| if a.is_finite() { a.to_degrees() } else { 0.0 })
}

/// The volume of a fill's `ddd*` fields (see the module notes).
fn volume(p: &FillPlacement) -> Volume {
    let finite3 = |v: [f32; 3], d: f32| v.map(|c| if c.is_finite() { c } else { d });
    Volume {
        offset: finite3(p.position.unwrap_or([0.0; 3]), 0.0).map(|v| v * 0.5),
        rotation: volume_rotation(p.euler_angles.unwrap_or([0.0; 3])),
        size: finite3(p.size.unwrap_or([1.0; 3]), 1.0),
        clip: p.volume_wrap == Some(0),
    }
}

/// The image placement of a volume-mode fill: tiling as is, rotation the other way, offset with v up.
fn image(uv: Option<UvTransform>, offset: bool) -> ImagePlacement {
    let Some(t) = uv else { return ImagePlacement::default() };
    let finite = |v: f32, d: f32| if v.is_finite() { v } else { d };
    ImagePlacement {
        tiling: t.scale.map(|v| finite(v, 1.0)),
        rotation: -finite(t.rotation, 0.0).to_degrees(),
        offset: if offset { [finite(t.translation[0], 0.0), -finite(t.translation[1], 0.0)] } else { [0.0; 2] },
    }
}

fn falloff(on: Option<bool>, hardness: Option<f32>) -> Falloff {
    Falloff { on: on.unwrap_or(false), hardness: hardness.filter(|h| h.is_finite()).unwrap_or(DEFAULT_HARDNESS) }
}

fn facing(p: &FillPlacement) -> FacingFalloff {
    FacingFalloff {
        on: p.backface_culling.unwrap_or(false),
        angle: p.backface_culling_angle.filter(|a| a.is_finite()).unwrap_or(DEFAULT_BACKFACE_ANGLE) * HALF_TURN_DEGREES,
        hardness: p.backface_culling_hardness.filter(|h| h.is_finite()).unwrap_or(DEFAULT_HARDNESS),
    }
}

/// TextureCraft's warp grid for a fill's `dddGrid`, or why it cannot be used.
fn warp_grid(g: &GridDeformation) -> Result<WarpGrid, String> {
    let [columns, rows] = g.size;
    let (Ok(columns), Ok(rows)) = (u32::try_from(columns), u32::try_from(rows)) else {
        return Err(format!("a warp grid of size {:?}", g.size));
    };
    let points = g.vertices.iter().map(|v| WarpPoint { pos: v.position, uv: [v.uv[0], 1.0 - v.uv[1]], tangents: v.tangents }).collect();
    let grid = WarpGrid { rows, columns, depth: if g.depth.is_finite() { g.depth.max(0.0) } else { DEFAULT_GRID.depth }, points };
    texturecraft_compose::placed::warp::anchors(&grid)?;
    Ok(grid)
}

fn repeat(wrap: Option<u64>) -> Repeat {
    match wrap {
        Some(WRAP_NONE) => Repeat::None,
        Some(WRAP_U) => Repeat::U,
        Some(WRAP_V) => Repeat::V,
        _ => Repeat::Both,
    }
}

pub fn map(fill: &FillEffect, mesh: Option<&Mesh>) -> MappedProjection {
    let identity = Projection::default();
    let uv = fill.uv_transform.filter(|t| !t.is_identity());
    let p = &fill.placement;
    let approx = |what: &str| Status::approximated(what.to_string());
    let placed = |mode: PlacedMode, offset: bool| {
        let mut volume = volume(p);
        if matches!(mode, PlacedMode::Warp { .. })
            && let Some(mesh) = mesh
        {
            let (lo, hi) = mesh.bounds();
            let centre = (lo + hi) * 0.5;
            let extent = (hi - lo).max_element().max(1e-6);
            // `volume` has the file position in half-units. The compositor adds its own
            // mesh centre and multiplies by the extent, so undo that frame here.
            volume.offset = ((Vec3::from_array(volume.offset) * 2.0 - centre) / extent).to_array();
            volume.size = volume.size.map(|size| size * (2.0 / extent));
        }
        Projection::Placed(Box::new(PlacedProjection { mode, volume, image: image(uv, offset), repeat: repeat(p.uv_wrap) }))
    };
    let wrap = p.uv_wrap.unwrap_or(WRAP_BOTH);
    let one_axis =
        || if matches!(wrap, WRAP_U | WRAP_V) { approx("repetition along one image axis (which is u and which v is not verified)") } else { Status::Exact };
    let (projection, mut status, volume) = match fill.projection {
        PROJECTION_TEXTURE_SPACE => (identity, Status::Exact, false),
        PROJECTION_UV => match uv {
            None if wrap == WRAP_BOTH => (identity, Status::Exact, false),
            uv => {
                let t = uv.unwrap_or(UvTransform { scale: [1.0, 1.0], translation: [0.0, 0.0], rotation: 0.0 });
                let (scale, rotation, offset) = uv_placement(&t, wrap == WRAP_BOTH);
                let status = one_axis();
                let projection = match wrap {
                    WRAP_BOTH => Projection::Uv { scale, rotation, offset },
                    _ => Projection::Placed(Box::new(PlacedProjection {
                        mode: PlacedMode::Uv { scale, rotation, offset },
                        volume: Volume::default(),
                        image: ImagePlacement::default(),
                        repeat: repeat(Some(wrap)),
                    })),
                };
                (projection, status, false)
            }
        },
        PROJECTION_TRIPLANAR => {
            let hardness = p.side_hardness.filter(|h| h.is_finite()).unwrap_or(DEFAULT_HARDNESS).clamp(0.0, 1.0);
            let mut status = Status::Exact;
            if let Some(t) = uv {
                if t.translation != [0.0, 0.0] {
                    status.and(&approx("UV offset of a tri-planar fill left out (its direction is not measured on tri-planar)"));
                }
                if t.rotation != 0.0 && t.scale[0] != t.scale[1] {
                    status.and(&approx("tri-planar UV rotation with uneven tiling (their order is not measured)"));
                }
            }
            if p.uv_sizing.as_ref().and_then(|z| z.scale_mode).is_some_and(|m| m != 0) {
                status.and(&approx("tri-planar UV size mode other than tiling"));
            }
            if p.volume_wrap == Some(0) {
                status.and(&approx("tri-planar volume clipped at its edge (not measured)"));
            }
            (placed(PlacedMode::TriPlanar { hardness }, false), status, true)
        }
        PROJECTION_PLANAR => {
            let (depth, facing) = (falloff(p.depth_culling, p.depth_culling_hardness), facing(p));
            let mut status = one_axis();
            if depth.on || facing.on || p.volume_wrap == Some(0) {
                status.and(&approx("planar projection's depth or facing falloff or clipping (their curves are not measured)"));
            }
            (placed(PlacedMode::Planar { depth, facing }, true), status, true)
        }
        PROJECTION_SPHERICAL => (placed(PlacedMode::Spherical, true), approx("spherical projection (the mode is not measured)"), true),
        PROJECTION_CYLINDRICAL => {
            let coverage = p.cylinder_culling_angle.filter(|a| a.is_finite()).unwrap_or(1.0) * WHOLE_TURN_DEGREES;
            let mode = PlacedMode::Cylindrical { facing: facing(p), coverage, hide_caps: p.cylinder_hide_caps.unwrap_or(false) };
            (placed(mode, true), approx("cylindrical projection (the mode is not measured)"), true)
        }
        PROJECTION_WARP => match p.grid.as_ref().map_or(Ok(DEFAULT_GRID), warp_grid) {
            Ok(grid) => {
                let mode = PlacedMode::Warp { depth: falloff(p.depth_culling, p.depth_culling_hardness), grid };
                (placed(mode, true), approx("warp grid laid on by TextureCraft's own Bézier warp (not measured)"), true)
            }
            Err(why) => (identity, Status::approximated(format!("warp projection with {why}; imported as texture space")), false),
        },
        other => (identity, Status::approximated(format!("projection mode {other} imported as texture space")), false),
    };
    let resampled =
        volume || !matches!(&projection, Projection::Uv { scale, rotation, offset } if *scale == [1.0, 1.0] && *rotation == 0.0 && *offset == [0.0, 0.0]);
    if resampled && !matches!(fill.filtering, FILTERING_BILINEAR | FILTERING_SMOOTH) {
        status.and(&Status::approximated(format!("fill filtering {} read as prefiltered", fill.filtering)));
    }
    MappedProjection { projection, status, volume }
}

/// True when a uniform fill's coverage is shaped by its projection (clipped volume, depth or facing falloffs, a
/// cylinder's angle or caps): the value is laid on through the projection's coverage, as exactly as the projection.
pub fn shapes_coverage(fill: &FillEffect) -> bool {
    let p = &fill.placement;
    let volume = matches!(fill.projection, PROJECTION_PLANAR..=PROJECTION_CYLINDRICAL | PROJECTION_WARP);
    volume
        && (p.volume_wrap == Some(0)
            || fill.projection == PROJECTION_WARP
            || (fill.projection == PROJECTION_PLANAR && (p.depth_culling == Some(true) || p.backface_culling == Some(true)))
            || (fill.projection == PROJECTION_CYLINDRICAL
                && (p.backface_culling == Some(true) || p.cylinder_hide_caps == Some(true) || p.cylinder_culling_angle.is_some_and(|a| a < 1.0))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_compose::placed::{self, Point};
    use texturecraft_geom::glam::{Vec2, Vec3};
    use texturecraft_import_reader::model::{EffectHeader, FillPlacement, UvTransform};

    fn fill(projection: u64, uv: Option<UvTransform>) -> FillEffect {
        FillEffect { header: EffectHeader::default(), sources: Vec::new(), projection, filtering: 2, uv_transform: uv, placement: FillPlacement::default() }
    }

    fn map(fill: &FillEffect) -> MappedProjection {
        super::map(fill, None)
    }

    fn placed_of(m: &MappedProjection) -> &PlacedProjection {
        let Projection::Placed(p) = &m.projection else { panic!("placed: {:?}", m.projection) };
        p
    }

    #[test]
    fn only_bilinear_filtering_reads_graphs_unfiltered() {
        assert!(!bilinear(&fill(0, None)), "smooth, the default");
        assert!(bilinear(&FillEffect { filtering: 0, ..fill(0, None) }));
        let unknown = map(&FillEffect { filtering: 1, ..fill(2, None) });
        assert!(!unknown.status.is_exact());
        assert!(map(&FillEffect { filtering: 1, ..fill(6, None) }).status.is_exact(), "laid on 1:1, nothing is resampled");
    }

    #[test]
    fn texture_space_and_identity_uv_are_exact() {
        assert!(map(&fill(6, None)).status.is_exact());
        let id = UvTransform { scale: [1.0, 1.0], translation: [0.0, 0.0], rotation: 0.0 };
        let m = map(&fill(0, Some(id)));
        assert!(m.status.is_exact() && m.projection == Projection::default() && !m.volume);
    }

    /// TextureCraft's placement of `uv` (as the compositor applies [`Projection::Uv`]).
    fn placed_uv(p: &Projection, uv: [f32; 2]) -> [f32; 2] {
        let Projection::Uv { scale, rotation, offset } = p else { panic!("uv") };
        let (sn, cs) = rotation.to_radians().sin_cos();
        [(uv[0] * cs - uv[1] * sn) * scale[0] + offset[0], (uv[0] * sn + uv[1] * cs) * scale[1] + offset[1]]
    }

    #[test]
    fn tiling_and_offset_place_about_the_centre_as_the_original() {
        let t = UvTransform { scale: [4.37, 3.0], translation: [0.22, -0.1], rotation: 0.0 };
        let m = map(&fill(0, Some(t)));
        assert!(m.status.is_exact());
        for uv in [[0.0, 0.0], [0.5, 0.5], [0.13, 0.71], [0.9, 0.2]] {
            // the original: tiling · (uv − ½ − offset) + ½ with v up, the same texel of the repeating image
            let want = [4.37 * (uv[0] - 0.5 - 0.22) + 0.5, 3.0 * (uv[1] - 0.5 - 0.1) + 0.5];
            let got = placed_uv(&m.projection, uv);
            for k in 0..2 {
                let d = (got[k] - want[k]).rem_euclid(1.0);
                assert!(d.min(1.0 - d) < 1e-4, "{uv:?}: {got:?} vs {want:?}");
            }
        }
        // a turn about the centre keeps the centre in place
        let turned = map(&fill(0, Some(UvTransform { scale: [1.0, 1.0], translation: [0.0, 0.0], rotation: 0.7 })));
        let c = placed_uv(&turned.projection, [0.5, 0.5]);
        assert!((c[0].rem_euclid(1.0) - 0.5).abs() < 1e-4 && (c[1].rem_euclid(1.0) - 0.5).abs() < 1e-4, "{c:?}");
        assert!(turned.status.is_exact());
    }

    /// A decal laid on once with negative tiling, a turn and an offset (a vase's moth): placed where the original's
    /// stored result shows it only with the turn and the offset's v read with v up (texel rows run down).
    #[test]
    fn turned_decals_follow_the_original_v_up() {
        let (theta, o, sc) = (1.278_377_4_f32, [-0.302_425_83_f32, -0.065_312_92], [-5.590_565_f32, -5.162_55]);
        let mut decal = fill(0, Some(UvTransform { scale: sc, translation: o, rotation: theta }));
        decal.placement.uv_wrap = Some(0);
        let m = map(&decal);
        assert!(m.status.is_exact());
        let PlacedMode::Uv { scale, rotation, offset } = placed_of(&m).mode else { panic!("uv") };
        let place = |uv: [f32; 2]| placed_uv(&Projection::Uv { scale, rotation, offset }, uv);
        // the image's centre sits at uv (½ + offset_u, ½ − offset_v) in texel rows: the moth's body on the stored result
        let centre = place([0.5 + o[0], 0.5 - o[1]]);
        assert!((centre[0] - 0.5).abs() < 1e-4 && (centre[1] - 0.5).abs() < 1e-4, "{centre:?}");
        let (s, c) = theta.sin_cos();
        for uv in [[0.1, 0.6], [0.25, 0.5], [0.2, 0.62]] {
            let (dx, dy) = (uv[0] - 0.5 - o[0], uv[1] - 0.5 + o[1]);
            let want = [sc[0] * (c * dx - s * dy) + 0.5, sc[1] * (s * dx + c * dy) + 0.5];
            let got = place(uv);
            assert!((got[0] - want[0]).abs() < 1e-3 && (got[1] - want[1]).abs() < 1e-3, "{uv:?}: {got:?} vs {want:?}");
        }
    }

    #[test]
    fn uv_fills_without_repeat_keep_their_image_once() {
        let mut once = fill(0, Some(UvTransform { scale: [2.0, 2.0], translation: [0.1, 0.0], rotation: 0.0 }));
        once.placement.uv_wrap = Some(0);
        let m = map(&once);
        assert!(m.status.is_exact() && !m.volume);
        let p = placed_of(&m);
        assert_eq!(p.repeat, Repeat::None);
        let PlacedMode::Uv { scale, offset, .. } = p.mode else { panic!("uv") };
        // not wrapped: the image's centre sits where the original put it (uv ½ + offset)
        assert_eq!(scale, [2.0, 2.0]);
        assert!((0.6 * 2.0 + offset[0] - 0.5).abs() < 1e-5 && (0.5 * 2.0 + offset[1] - 0.5).abs() < 1e-5, "{offset:?}");
        once.placement.uv_wrap = Some(1);
        assert!(!map(&once).status.is_exact());
        // no transform, no repeat: still placed (the image covers the UV square once)
        let mut plain = fill(0, None);
        plain.placement.uv_wrap = Some(0);
        assert_eq!(placed_of(&map(&plain)).repeat, Repeat::None);
    }

    /// Image coordinate and share of the plane facing `nrm` at `pos` in `bounds`, as the compositor evaluates it.
    fn tri_at(p: &PlacedProjection, bounds: (Vec3, Vec3), pos: Vec3, nrm: Vec3) -> Vec2 {
        let t = placed::taps(p, bounds, &Point { pos, nrm, texel_len: 1.0 / 512.0, ..Point::default() });
        assert_eq!(t.n, 1, "{t:?}");
        t.taps[0].uv
    }

    /// The tri-planar reading measured on controlled projects (module notes): the volume in the bounding box, angles z,
    /// then y, then x, the UV rotation turned the other way.
    #[test]
    fn tri_planar_follows_the_measured_convention() {
        let mut tri = fill(2, Some(UvTransform { scale: [2.0, 2.0], translation: [0.0, 0.0], rotation: std::f32::consts::FRAC_PI_6 }));
        tri.placement.side_hardness = Some(0.9);
        tri.placement.position = Some([0.4, 0.0, -0.2]);
        tri.placement.size = Some([0.5, 1.0, 2.0]);
        tri.placement.euler_angles = Some([20f32, 30.0, 40.0].map(f32::to_radians));
        let m = map(&tri);
        assert!(m.status.is_exact() && m.volume, "{:?}", m.status);
        let p = placed_of(&m);
        assert_eq!(p.mode, PlacedMode::TriPlanar { hardness: 0.9 });
        assert_eq!(p.volume.offset, [0.2, 0.0, -0.1]);
        assert_eq!(p.volume.size, [0.5, 1.0, 2.0]);
        assert!((p.image.rotation + 30.0).abs() < 1e-4 && p.image.tiling == [2.0, 2.0]);
        // R = Rx(20°)·Ry(30°)·Rz(40°), as TextureCraft's x-y-z degrees
        let r = [20f32, 30.0, 40.0].map(f32::to_radians);
        let want = Mat3::from_rotation_x(r[0]) * Mat3::from_rotation_y(r[1]) * Mat3::from_rotation_z(r[2]);
        let got = placed::rotation_xyz(p.volume.rotation);
        assert!(want.abs_diff_eq(got, 1e-5), "{want:?} vs {got:?}");
        // the original's local point: R^T (p − centre − position × ½ × extent) / (size × extent), here on the z plane
        let bounds = (Vec3::new(1.2, 0.2, -1.3), Vec3::new(2.8, 1.2, -0.7));
        let unrotated = PlacedProjection { volume: Volume { rotation: [0.0; 3], ..p.volume }, image: ImagePlacement::default(), ..p.clone() };
        let pos = Vec3::new(2.5, 0.5, -0.7);
        let l = (pos - Vec3::new(2.0, 0.7, -1.0) - Vec3::new(0.4, 0.0, -0.2) * 0.5 * 1.6) / (Vec3::new(0.5, 1.0, 2.0) * 1.6);
        let uv = tri_at(&unrotated, bounds, pos, Vec3::Z);
        assert!((uv - Vec2::new(0.5 + l.x, 0.5 - l.y)).length() < 1e-5, "{uv:?} vs {l:?}");
        // the offset is left out and reported
        let moved = map(&fill(2, Some(UvTransform { scale: [1.0, 1.0], translation: [0.1, 0.2], rotation: 0.0 })));
        assert!(!moved.status.is_exact());
        assert_eq!(placed_of(&moved).image.offset, [0.0, 0.0]);
        assert!(placed_of(&map(&fill(2, None))).mode == PlacedMode::TriPlanar { hardness: DEFAULT_HARDNESS });
    }

    #[test]
    fn gimbal_lock_keeps_the_rotation() {
        for r in [[0.3f32, std::f32::consts::FRAC_PI_2, -0.4], [1.0, -std::f32::consts::FRAC_PI_2, 0.2], [0.0; 3], [f32::NAN, 0.1, 0.0]] {
            let finite = r.map(|a| if a.is_finite() { a } else { 0.0 });
            let want = Mat3::from_rotation_x(finite[0]) * Mat3::from_rotation_y(finite[1]) * Mat3::from_rotation_z(finite[2]);
            assert!(want.abs_diff_eq(placed::rotation_xyz(volume_rotation(r)), 1e-4), "{r:?}");
        }
    }

    #[test]
    fn volume_modes_carry_their_falloffs() {
        let mut planar = fill(1, Some(UvTransform { scale: [2.0, 2.0], translation: [0.1, 0.2], rotation: 0.0 }));
        planar.placement.volume_wrap = Some(0);
        planar.placement.depth_culling = Some(true);
        planar.placement.depth_culling_hardness = Some(0.75);
        planar.placement.backface_culling = Some(true);
        planar.placement.backface_culling_angle = Some(0.25);
        let m = map(&planar);
        assert!(!m.status.is_exact() && m.volume && shapes_coverage(&planar));
        let p = placed_of(&m);
        let PlacedMode::Planar { depth, facing } = p.mode else { panic!("planar") };
        assert!(depth.on && depth.hardness == 0.75 && facing.on && facing.angle == 45.0);
        assert!(p.volume.clip);
        assert_eq!(p.image.offset, [0.1, -0.2], "v runs up");
        let plain = map(&fill(1, None));
        assert!(plain.status.is_exact() && !shapes_coverage(&fill(1, None)));
        let mut cyl = fill(4, None);
        cyl.placement.cylinder_culling_angle = Some(0.5);
        let c = map(&cyl);
        assert!(matches!(placed_of(&c).mode, PlacedMode::Cylindrical { coverage: 180.0, .. }) && !c.status.is_exact());
        assert!(shapes_coverage(&cyl));
        assert!(matches!(placed_of(&map(&fill(3, None))).mode, PlacedMode::Spherical));
        assert!(!map(&fill(8, None)).status.is_exact());
    }

    #[test]
    fn warp_keeps_its_placement_when_the_mesh_bounds_are_asymmetric() {
        let bounds = (Vec3::new(-0.6, -1.0, -0.4), Vec3::new(0.6, 0.96, 0.4));
        let mesh = Mesh { positions: vec![bounds.0, bounds.1], ..Mesh::default() };
        let mut warp = fill(7, None);
        let origin = Vec3::new(0.03, 0.64, 0.32);
        warp.placement.position = Some(origin.to_array());
        warp.placement.size = Some([0.17; 3]);
        warp.placement.grid = Some(GridDeformation { size: [3, 3], depth: 0.01, vertices: Vec::new() });
        warp.placement.depth_culling = Some(true);

        let mapped = super::map(&warp, Some(&mesh));
        let projection = placed_of(&mapped);
        let at = |pos| placed::taps(projection, bounds, &Point { pos, nrm: Vec3::Z, texel_len: 1.0 / 2048.0, ..Point::default() });
        let centre = at(origin);
        assert!(centre.coverage > 0.99);
        assert!((centre.taps[0].uv - Vec2::splat(0.5)).length() < 1e-5, "{centre:?}");
        let right = at(origin + Vec3::X * (0.34 * 0.25));
        assert!((right.taps[0].uv - Vec2::new(0.75, 0.5)).length() < 1e-5, "{right:?}");
    }

    #[test]
    fn warp_grids_are_imported_as_stored() {
        use texturecraft_import_reader::model::GridVertex;
        // the original's default 3 × 3 grid: rows from the bottom, image v up, handles a third of the spacing
        let mut vertices = Vec::new();
        for r in 0..3 {
            for c in 0..3 {
                let (x, y) = (c as f32 - 1.0, r as f32 - 1.0);
                let third = 1.0 / 3.0;
                vertices.push(GridVertex {
                    position: [x, y, 0.0],
                    uv: [c as f32 / 2.0, r as f32 / 2.0],
                    tangents: [[-third, 0.0, 0.0], [third, 0.0, 0.0], [0.0, -third, 0.0], [0.0, third, 0.0]],
                });
            }
        }
        let mut warp = fill(7, None);
        warp.placement.grid = Some(GridDeformation { size: [3, 3], depth: 0.1, vertices });
        warp.placement.depth_culling = Some(true);
        warp.placement.size = Some([1.0, 1.0, 0.5]);
        let m = map(&warp);
        assert!(m.volume && !m.status.is_exact() && shapes_coverage(&warp));
        let p = placed_of(&m);
        let PlacedMode::Warp { depth, grid } = &p.mode else { panic!("warp: {:?}", p.mode) };
        assert!(depth.on && (grid.rows, grid.columns, grid.depth) == (3, 3, 0.1));
        assert_eq!(grid.points.first().map(|q| q.uv), Some([0.0, 1.0]), "the bottom-left anchor shows the image's bottom-left");
        // laid on as the planar projection would: a point a quarter in from the bottom left
        let bounds = (Vec3::splat(-1.0), Vec3::splat(1.0));
        let t = placed::taps(p, bounds, &Point { pos: Vec3::new(-0.5, -0.5, 0.0), nrm: Vec3::Z, texel_len: 1.0 / 512.0, ..Point::default() });
        assert!(t.n == 1 && (t.taps[0].uv - Vec2::new(0.25, 0.75)).length() < 1e-4, "{t:?}");
        // a grid whose points do not fill it falls back to texture space; none stored: the regular grid
        if let Some(g) = &mut warp.placement.grid {
            g.vertices.truncate(4);
        }
        let broken = map(&warp);
        assert!(broken.projection == Projection::default() && !broken.status.is_exact());
        assert!(matches!(placed_of(&map(&fill(7, None))).mode, PlacedMode::Warp { .. }));
    }
}
