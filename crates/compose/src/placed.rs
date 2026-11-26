//! Evaluating a [`PlacedProjection`]: a surface point → at most three image taps, each with a weight and a prefilter,
//! and the projection's coverage there (volume clip, depth and facing falloffs, cylinder coverage and caps).
//!
//! | Mode | Image coordinates before the placement | Coverage |
//! |---|---|---|
//! | UV | the mesh's UVs placed as a UV projection | 1 |
//! | planar | `(x + ½, ½ − y)` | clip, depth falloff on local z, facing falloff on the normal against +z |
//! | tri-planar | x plane `(½ − z·sign n_x, ½ − y)`, y plane `(x + ½, ½ + z·sign n_y)`, z plane `(½ + x·sign n_z, ½ − y)`; shares [`tri_planar_shares`] | clip |
//! | spherical | longitude from +z towards +x, latitude from the top at +y | clip |
//! | cylindrical | angle around local y from +z over the coverage angle, `½ − y` | clip, outside the angle, facing on the radial direction, caps |
//! | warp | the image coordinate of the grid's surface over the point ([`warp`]) | clip, within the depth of the surface, depth falloff over it |
//!
//! Prefilter: planar and tri-planar carry the texel's own two axes on the surface through the plane and the placement
//! into image units and filter anisotropically ([`anisotropic_footprint`]): `ceil(longer / shorter)` probes (at most
//! [`MAX_ANISOTROPY`]) along the longer axis, each prefiltered by the longer axis over their count, so every texel is
//! filtered by its own size and shape on the surface. Spherical, cylindrical, warp and texels without axes take one
//! isotropic footprint from the texel's size on the surface.

pub mod warp;

use texturecraft_core::{FacingFalloff, Falloff, ImagePlacement, PlacedMode, PlacedProjection, Volume};
use texturecraft_geom::glam::{Mat3, Vec2, Vec3};

/// Half the side of a volume in local coordinates.
const HALF: f32 = 0.5;
/// Below this the normal's share along a cylinder's axis counts as a cap (45°).
const CAP_COS: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// The most probes anisotropic filtering spreads along a texel's longer axis.
pub const MAX_ANISOTROPY: f32 = 16.0;

/// One sample position in the projected image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tap {
    /// Image coordinate (0 … 1 across the image once, v down).
    pub uv: Vec2,
    pub weight: f32,
    /// The texel's size in image units (each probe's when `probes` > 1), for the prefilter.
    pub footprint: f32,
    /// The span in image units over which `probes` samples spread evenly, centred on `uv`.
    pub major: Vec2,
    pub probes: u8,
}

/// Up to three taps and the projection's coverage at the point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Taps {
    pub taps: [Tap; 3],
    pub n: usize,
    pub coverage: f32,
}

impl Taps {
    fn one(tap: Tap, coverage: f32) -> Taps {
        Taps { taps: [tap, Tap::default(), Tap::default()], n: 1, coverage }
    }

    /// The taps in use.
    pub fn used(&self) -> &[Tap] {
        self.taps.get(..self.n.min(3)).unwrap_or(&[])
    }
}

/// What lies under one texel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Point {
    /// The mesh's texture coordinates (for the UV mode).
    pub uv: Vec2,
    /// World position and interpolated normal.
    pub pos: Vec3,
    pub nrm: Vec3,
    /// World displacement for one texel step along x and along y (zero: unknown).
    pub axes: [Vec3; 2],
    /// The texel's size on the surface in world units (0: unknown), the isotropic fallback.
    pub world_len: f32,
    /// The texel's size in UV units (1 / texels across a UV tile).
    pub texel_len: f32,
}

/// The projection's volume in the scene.
#[derive(Clone, Copy, Debug)]
struct VolumeFrame {
    /// World → volume axes.
    to_local: Mat3,
    centre: Vec3,
    /// 1 / (size × scene unit) per axis, signed.
    inv: Vec3,
    /// Local units per world unit along the most compressed axis, for footprints.
    local_per_world: f32,
    clip: bool,
}

fn finite(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

/// The rotation of `degrees` about the fixed x, y and z axes applied in that order (`Rz·Ry·Rx`).
pub fn rotation_xyz(degrees: [f32; 3]) -> Mat3 {
    let [x, y, z] = degrees.map(|d| finite(d, 0.0).to_radians());
    Mat3::from_rotation_z(z) * Mat3::from_rotation_y(y) * Mat3::from_rotation_x(x)
}

impl VolumeFrame {
    fn new(volume: &Volume, bounds: (Vec3, Vec3)) -> VolumeFrame {
        let (lo, hi) = bounds;
        let extent = hi - lo;
        let unit = [extent.x, extent.y, extent.z].into_iter().filter(|v| v.is_finite()).fold(0.0f32, f32::max).max(1e-6);
        let centre = (lo + hi) * 0.5;
        let centre = if centre.is_finite() { centre } else { Vec3::ZERO };
        let offset = Vec3::from(volume.offset.map(|v| finite(v, 0.0) * unit));
        let inv = Vec3::from(volume.size.map(|s| {
            let s = finite(s, 1.0);
            let s = if s.abs() < 1e-6 { 1e-6f32.copysign(s) } else { s };
            1.0 / (s * unit)
        }));
        let local_per_world = inv.abs().max_element();
        VolumeFrame { to_local: rotation_xyz(volume.rotation).transpose(), centre: centre + offset, inv, local_per_world, clip: volume.clip }
    }

    fn local(&self, p: Vec3) -> Vec3 {
        (self.to_local * (p - self.centre)) * self.inv
    }

    /// A world normal in the volume's axes (mirrored axes flip it), unit length; zero for a zero normal.
    fn local_normal(&self, n: Vec3) -> Vec3 {
        let r = (self.to_local * n) * Vec3::new(self.inv.x.signum(), self.inv.y.signum(), self.inv.z.signum());
        r.try_normalize().unwrap_or(Vec3::ZERO)
    }

    fn clip_coverage(&self, l: Vec3) -> f32 {
        let inside = l.x.abs() <= HALF && l.y.abs() <= HALF && l.z.abs() <= HALF;
        if !self.clip || inside { 1.0 } else { 0.0 }
    }

    /// The texel's axes as local displacements (zero without axes).
    fn local_axes(&self, p: &Point) -> [Vec3; 2] {
        p.axes.map(|a| if a == Vec3::ZERO { Vec3::ZERO } else { (self.to_local * a) * self.inv })
    }

    /// The texel's size in local units (its UV size without a surface size).
    fn local_len(&self, p: &Point) -> f32 {
        if p.world_len > 0.0 && p.world_len.is_finite() { p.world_len * self.local_per_world } else { p.texel_len }
    }
}

/// [`ImagePlacement`] as an affine map from projection coordinates (0 … 1, centre ½) to image coordinates.
#[derive(Clone, Copy, Debug)]
struct Placement {
    /// Rows of `image = m · (p − ½ − offset) + ½`.
    m: [Vec2; 2],
    offset: Vec2,
    /// Image units per projection unit (the larger axis).
    scale: f32,
}

impl Placement {
    fn new(p: &ImagePlacement) -> Placement {
        let (tu, tv) = (finite(p.tiling[0], 1.0), finite(p.tiling[1], 1.0));
        let (s, c) = finite(p.rotation, 0.0).to_radians().sin_cos();
        Placement {
            m: [Vec2::new(c * tu, s * tu), Vec2::new(-s * tv, c * tv)],
            offset: Vec2::new(finite(p.offset[0], 0.0), finite(p.offset[1], 0.0)),
            scale: tu.abs().max(tv.abs()),
        }
    }

    fn apply(&self, p: Vec2) -> Vec2 {
        let q = p - Vec2::splat(0.5) - self.offset;
        Vec2::new(self.m[0].dot(q), self.m[1].dot(q)) + Vec2::splat(0.5)
    }

    /// A displacement in projection coordinates in image units.
    fn linear(&self, d: Vec2) -> Vec2 {
        Vec2::new(self.m[0].dot(d), self.m[1].dot(d))
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if e1 <= e0 {
        return if x < e0 { 0.0 } else { 1.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hardness(h: f32) -> f32 {
    if h.is_finite() { h.clamp(0.0, 1.0) } else { 0.5 }
}

/// Depth falloff at signed distance `d` from the middle of a depth of `half` on either side: full within
/// `hardness × half`, a smooth step to 0 at `half` (a hard cut at hardness 1). Off: 1.
pub fn depth_falloff(f: &Falloff, d: f32, half: f32) -> f32 {
    if !f.on {
        return 1.0;
    }
    1.0 - smoothstep(hardness(f.hardness) * half, half, d.abs())
}

/// Facing falloff for a normal at cosine `cos` to the projection direction (see [`FacingFalloff`]). Off: 1.
pub fn facing_falloff(f: &FacingFalloff, cos: f32) -> f32 {
    if !f.on {
        return 1.0;
    }
    let a = if f.angle.is_finite() { f.angle.clamp(0.0, 180.0) } else { 90.0 };
    let t = cos.clamp(-1.0, 1.0).acos().to_degrees();
    let w = (1.0 - hardness(f.hardness)) * a.min(180.0 - a);
    1.0 - smoothstep(a - w, a + w, t)
}

/// The shares of the planes facing x, y and z for a unit local normal (sum 1): each a smooth step of its component
/// over the largest one, from `½ + ½ × hardness` to 1, normalised. A zero normal faces z.
pub fn tri_planar_shares(n: Vec3, hardness_value: f32) -> [f32; 3] {
    let a = n.abs();
    let m = a.max_element();
    if !m.is_finite() || m <= 1e-12 {
        return [0.0, 0.0, 1.0];
    }
    let edge = 0.5 + 0.5 * hardness(hardness_value);
    let w = [a.x, a.y, a.z].map(|c| smoothstep(edge, 1.0, c / m));
    let sum = w[0] + w[1] + w[2];
    if sum > 0.0 { w.map(|v| v / sum) } else { [0.0, 0.0, 1.0] }
}

/// The projection coordinates of local point `l` on the planes facing x, y and z, each as seen from the side the
/// local normal `n` faces.
pub fn tri_planar_planes(l: Vec3, n: Vec3) -> [Vec2; 3] {
    let sign = |c: f32| if c < 0.0 { -1.0 } else { 1.0 };
    [Vec2::new(0.5 - l.z * sign(n.x), 0.5 - l.y), Vec2::new(l.x + 0.5, 0.5 + l.z * sign(n.y)), Vec2::new(0.5 + l.x * sign(n.z), 0.5 - l.y)]
}

/// Anisotropic prefilter of a texel whose image-space axes are `a` and `b`: `ceil(longer / shorter)` probes (at most
/// [`MAX_ANISOTROPY`]), each prefiltered by the longer axis over their count, spread over the rest of the longer axis.
/// Returns the probes' footprint, their span and count.
pub fn anisotropic_footprint(a: Vec2, b: Vec2) -> (f32, Vec2, u8) {
    let (major, minor) = if a.length() >= b.length() { (a, b) } else { (b, a) };
    let (lmaj, lmin) = (major.length(), minor.length());
    if !lmaj.is_finite() || lmaj <= 0.0 {
        return (0.0, Vec2::ZERO, 1);
    }
    let probes = if lmin > 0.0 { (lmaj / lmin).ceil().clamp(1.0, MAX_ANISOTROPY) } else { MAX_ANISOTROPY };
    let footprint = lmaj / probes;
    let k = ((lmaj - footprint) / lmaj).max(0.0);
    (footprint, major * k, probes as u8)
}

/// A tap at projection coordinates `coords`, prefiltered by the texel's local axes `d` through `plane` (a local
/// displacement → projection coordinates) when known, else by the isotropic local length `len`.
fn plane_tap(place: &Placement, coords: Vec2, weight: f32, d: &[Vec3; 2], plane: impl Fn(Vec3) -> Vec2, len: f32) -> Tap {
    let mut tap = Tap { uv: place.apply(coords), weight, footprint: len * place.scale, ..Tap::default() };
    if d.iter().any(|a| *a != Vec3::ZERO) {
        let (footprint, major, probes) = anisotropic_footprint(place.linear(plane(d[0])), place.linear(plane(d[1])));
        if footprint.is_finite() && major.is_finite() {
            (tap.footprint, tap.major, tap.probes) = (footprint, major, probes);
        }
    }
    tap
}

/// The taps of `p` at point `pt` in a scene of `bounds` (the whole mesh's bounding box).
pub fn taps(p: &PlacedProjection, bounds: (Vec3, Vec3), pt: &Point) -> Taps {
    if let PlacedMode::Uv { scale, rotation, offset } = &p.mode {
        let (sn, cs) = finite(*rotation, 0.0).to_radians().sin_cos();
        let s = Vec2::new(finite(scale[0], 1.0), finite(scale[1], 1.0));
        let uv = Vec2::new(pt.uv.x * cs - pt.uv.y * sn, pt.uv.x * sn + pt.uv.y * cs) * s + Vec2::new(finite(offset[0], 0.0), finite(offset[1], 0.0));
        let footprint = s.x.abs().max(s.y.abs()) * pt.texel_len;
        return Taps::one(Tap { uv, weight: 1.0, footprint, ..Tap::default() }, 1.0);
    }
    let vol = VolumeFrame::new(&p.volume, bounds);
    let place = Placement::new(&p.image);
    let l = vol.local(pt.pos);
    let n = vol.local_normal(pt.nrm);
    let len = vol.local_len(pt);
    let d = vol.local_axes(pt);
    match &p.mode {
        PlacedMode::TriPlanar { hardness } => {
            let shares = tri_planar_shares(n, *hardness);
            let coords = tri_planar_planes(l, n);
            let origin = tri_planar_planes(Vec3::ZERO, n);
            let mut out = Taps { coverage: vol.clip_coverage(l), ..Taps::default() };
            for (i, (c, w)) in coords.into_iter().zip(shares).enumerate() {
                if w > 0.0
                    && let Some(slot) = out.taps.get_mut(out.n)
                {
                    let plane = |v: Vec3| {
                        let q = tri_planar_planes(v, n);
                        q.get(i).copied().unwrap_or_default() - origin.get(i).copied().unwrap_or_default()
                    };
                    *slot = plane_tap(&place, c, w, &d, plane, len);
                    out.n += 1;
                }
            }
            out
        }
        PlacedMode::Planar { depth, facing } => {
            let cos = if n == Vec3::ZERO { 1.0 } else { n.z };
            let coverage = vol.clip_coverage(l) * depth_falloff(depth, l.z, HALF) * facing_falloff(facing, cos);
            let tap = plane_tap(&place, Vec2::new(l.x + 0.5, 0.5 - l.y), 1.0, &d, |v| Vec2::new(v.x, -v.y), len);
            Taps::one(tap, coverage)
        }
        PlacedMode::Spherical => {
            let r = l.length();
            let lon = l.x.atan2(l.z);
            let lat = if r > 1e-9 { (l.y / r).clamp(-1.0, 1.0).asin() } else { 0.0 };
            let footprint = len / (std::f32::consts::PI * r.max(1e-3)) * place.scale;
            let coords = Vec2::new(0.5 + lon / std::f32::consts::TAU, 0.5 - lat / std::f32::consts::PI);
            Taps::one(Tap { uv: place.apply(coords), weight: 1.0, footprint, ..Tap::default() }, vol.clip_coverage(l))
        }
        PlacedMode::Cylindrical { facing, coverage, hide_caps } => {
            let angle_range = if coverage.is_finite() { coverage.clamp(1e-3, 360.0) } else { 360.0 }.to_radians();
            let angle = l.x.atan2(l.z);
            let r = (l.x * l.x + l.z * l.z).sqrt();
            let radial_cos = if n != Vec3::ZERO && r > 1e-9 { (n.x * l.x + n.z * l.z) / r } else { 1.0 };
            let inside = angle.abs() <= 0.5 * angle_range + 1e-6;
            let cap = *hide_caps && n.y.abs() > CAP_COS;
            let cover = if inside && !cap { vol.clip_coverage(l) * facing_falloff(facing, radial_cos) } else { 0.0 };
            let footprint = len.max(len / (angle_range * r.max(1e-3))) * place.scale;
            let coords = Vec2::new(0.5 + angle / angle_range, 0.5 - l.y);
            Taps::one(Tap { uv: place.apply(coords), weight: 1.0, footprint, ..Tap::default() }, cover)
        }
        PlacedMode::Warp { depth, grid } => {
            let Ok(surface) = warp::surface_for(grid) else { return Taps::default() };
            let g = (l * warp::GRID_PER_LOCAL).as_dvec3();
            let Some(hit) = surface.locate([g.x, g.y]) else { return Taps::default() };
            let reach = if grid.depth.is_finite() { grid.depth.max(0.0) } else { 0.0 };
            let dz = (g.z - hit.pos[2]) as f32;
            if dz.is_nan() || dz.abs() > reach {
                return Taps::default();
            }
            let cover = vol.clip_coverage(l) * depth_falloff(depth, dz, reach);
            let footprint = len * warp::GRID_PER_LOCAL * hit.image_per_grid as f32 * place.scale;
            let uv = Vec2::new(hit.uv[0] as f32, hit.uv[1] as f32);
            Taps::one(Tap { uv: place.apply(uv), weight: 1.0, footprint, ..Tap::default() }, cover)
        }
        PlacedMode::Uv { .. } => Taps::default(),
    }
}

/// A tap's value through `sample` (the image at a coordinate with a footprint; `None` outside a non-repeating image)
/// for an image `side` texels across: the mean of its probes along the longer axis when it has several and the texel
/// covers more than one image texel, else one sample at its coordinate. Probes outside are left out; `None` if all are.
/// `unfiltered`: one sample at the coordinate, no prefilter.
pub fn filtered(tap: &Tap, side: u32, unfiltered: bool, mut sample: impl FnMut(Vec2, f32) -> Option<[f32; 4]>) -> Option<[f32; 4]> {
    if unfiltered {
        return sample(tap.uv, 0.0);
    }
    let n = tap.probes;
    if n <= 1 || tap.footprint * side as f32 <= 1.0 {
        return sample(tap.uv, tap.footprint);
    }
    let (mut sum, mut k) = ([0.0f32; 4], 0u32);
    for i in 0..n {
        let t = (f32::from(i) + 0.5) / f32::from(n) - 0.5;
        if let Some(v) = sample(tap.uv + tap.major * t, tap.footprint) {
            for (s, v) in sum.iter_mut().zip(v) {
                *s += v;
            }
            k += 1;
        }
    }
    (k > 0).then(|| sum.map(|s| s / k as f32))
}

/// True when `uv` lies where a [`texturecraft_core::Repeat`] of `axes` (u, v repeat) has image.
pub fn covered(uv: Vec2, axes: [bool; 2]) -> bool {
    (axes[0] || (0.0..=1.0).contains(&uv.x)) && (axes[1] || (0.0..=1.0).contains(&uv.y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_core::Repeat;

    fn tri(hardness: f32, volume: Volume, image: ImagePlacement) -> PlacedProjection {
        PlacedProjection { mode: PlacedMode::TriPlanar { hardness }, volume, image, repeat: Repeat::Both }
    }

    /// The tri-planar box the convention was measured on: off-centre, distinct extents.
    const BOX: (Vec3, Vec3) = (Vec3::new(1.2, 0.2, -1.3), Vec3::new(2.8, 1.2, -0.7));

    fn at(pos: Vec3, nrm: Vec3) -> Point {
        Point { pos, nrm, texel_len: 1.0 / 512.0, ..Point::default() }
    }

    #[test]
    fn shares_follow_the_measured_curve() {
        let dir = |deg: f32| {
            let r = deg.to_radians();
            Vec3::new(r.sin(), 0.0, r.cos())
        };
        // hardness 0: from ½ to 1 of the largest component (26.6° … 45° … 63.4°)
        assert!(tri_planar_shares(dir(26.0), 0.0)[0] < 1e-6);
        assert!((tri_planar_shares(dir(45.0), 0.0)[0] - 0.5).abs() < 1e-5);
        assert!((tri_planar_shares(dir(36.0), 0.0)[0] - 0.31).abs() < 0.02);
        assert!(tri_planar_shares(dir(64.0), 0.0)[2] < 1e-6);
        // hardness 0.5: 36.9° … 53.1°
        assert!(tri_planar_shares(dir(36.0), 0.5)[0] < 1e-6);
        assert!((tri_planar_shares(dir(42.0), 0.5)[0] - 0.39).abs() < 0.03);
        assert_eq!(tri_planar_shares(dir(30.0), 1.0), [0.0, 0.0, 1.0]);
        assert_eq!(tri_planar_shares(Vec3::ZERO, 0.5), [0.0, 0.0, 1.0]);
        for h in [0.0, 0.3, 0.9, 1.0, f32::NAN] {
            let w = tri_planar_shares(Vec3::new(-0.5, 0.5, 0.7), h);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn tri_planar_frame_is_the_bounding_box() {
        // the box's centre is the image's centre; one repetition spans the largest extent (1.6 along x)
        let p = tri(1.0, Volume::default(), ImagePlacement::default());
        let t = taps(&p, BOX, &at(Vec3::new(2.0, 0.7, -0.7), Vec3::Z));
        assert_eq!(t.n, 1);
        assert!((t.taps[0].uv - Vec2::new(0.5, 0.5)).length() < 1e-5, "{:?}", t.taps[0]);
        let t = taps(&p, BOX, &at(Vec3::new(2.4, 0.3, -0.7), Vec3::Z));
        assert!((t.taps[0].uv - Vec2::new(0.75, 0.75)).length() < 1e-5, "{:?}", t.taps[0]);
        // seen from −z the image is not mirrored: +x runs to the left
        let t = taps(&p, BOX, &at(Vec3::new(2.4, 0.3, -1.3), -Vec3::Z));
        assert!((t.taps[0].uv - Vec2::new(0.25, 0.75)).length() < 1e-5, "{:?}", t.taps[0]);
        // the offset moves the volume in scene units, the size scales it, the tiling repeats about the centre
        let moved =
            tri(1.0, Volume { offset: [0.25, 0.0, 0.0], size: [0.5; 3], ..Volume::default() }, ImagePlacement { tiling: [2.0, 2.0], ..Default::default() });
        let t = taps(&moved, BOX, &at(Vec3::new(2.4, 0.7, -0.7), Vec3::Z));
        assert!((t.taps[0].uv - Vec2::new(0.5, 0.5)).length() < 1e-5, "{:?}", t.taps[0]);
    }

    #[test]
    fn rotations_apply_x_then_y_then_z() {
        let q = rotation_xyz([90.0, 0.0, 90.0]);
        // first about x (y → z), then about z: x → y
        assert!((q * Vec3::Y - Vec3::Z).length() < 1e-5);
        assert!((q * Vec3::X - Vec3::Y).length() < 1e-5);
    }

    #[test]
    fn planar_falloffs_and_clip_shape_coverage() {
        let p = PlacedProjection {
            mode: PlacedMode::Planar { depth: Falloff { on: true, hardness: 0.5 }, facing: FacingFalloff { on: true, angle: 90.0, hardness: 1.0 } },
            volume: Volume { size: [1.0, 1.0, 0.5], clip: true, ..Volume::default() },
            image: ImagePlacement::default(),
            repeat: Repeat::None,
        };
        let b = (Vec3::splat(-1.0), Vec3::splat(1.0)); // unit 2: local = world / (size × 2)
        assert_eq!(taps(&p, b, &at(Vec3::ZERO, Vec3::Z)).coverage, 1.0);
        // local z 0.375 of a half depth 0.5, hardness 0.5: half way through the fade
        assert!((taps(&p, b, &at(Vec3::new(0.0, 0.0, 0.375), Vec3::Z)).coverage - 0.5).abs() < 1e-5);
        assert_eq!(taps(&p, b, &at(Vec3::ZERO, -Vec3::Z)).coverage, 0.0, "turned away");
        assert_eq!(taps(&p, b, &at(Vec3::new(1.5, 0.0, 0.0), Vec3::Z)).coverage, 0.0, "clipped");
        assert!(!covered(Vec2::new(1.2, 0.5), Repeat::None.axes()) && covered(Vec2::new(1.2, 0.5), Repeat::U.axes()));
    }

    #[test]
    fn cylinder_covers_its_angle_and_hides_caps() {
        let p = PlacedProjection {
            mode: PlacedMode::Cylindrical { facing: FacingFalloff::default(), coverage: 180.0, hide_caps: true },
            volume: Volume::default(),
            image: ImagePlacement::default(),
            repeat: Repeat::Both,
        };
        let b = (Vec3::splat(-0.5), Vec3::splat(0.5));
        let front = taps(&p, b, &at(Vec3::new(0.0, 0.1, 0.4), Vec3::Z));
        assert_eq!(front.coverage, 1.0);
        assert!((front.taps[0].uv.x - 0.5).abs() < 1e-5);
        assert_eq!(taps(&p, b, &at(Vec3::new(0.0, 0.0, -0.4), -Vec3::Z)).coverage, 0.0, "behind");
        assert_eq!(taps(&p, b, &at(Vec3::new(0.0, 0.5, 0.2), Vec3::Y)).coverage, 0.0, "cap");
        let s = PlacedProjection { mode: PlacedMode::Spherical, ..p };
        let top = taps(&s, b, &at(Vec3::new(0.0, 0.4, 0.0), Vec3::Y));
        assert!(top.taps[0].uv.y.abs() < 1e-5, "{:?}", top.taps[0]);
    }

    #[test]
    fn stretched_texels_filter_along_their_long_axis() {
        let (f, major, n) = anisotropic_footprint(Vec2::new(4.0, 0.0), Vec2::new(0.0, 1.0));
        assert_eq!(n, 4);
        assert!((f - 1.0).abs() < 1e-6 && (major - Vec2::new(3.0, 0.0)).length() < 1e-6);
        let (f, _, n) = anisotropic_footprint(Vec2::new(1.1, 0.0), Vec2::new(0.0, 1.0));
        assert_eq!(n, 2, "barely longer: two probes at half the level");
        assert!((f - 0.55).abs() < 1e-6);
        assert_eq!(anisotropic_footprint(Vec2::ZERO, Vec2::ZERO), (0.0, Vec2::ZERO, 1));
        // a planar texel stretched along x on the surface
        let p = PlacedProjection {
            mode: PlacedMode::Planar { depth: Falloff::default(), facing: FacingFalloff::default() },
            volume: Volume::default(),
            image: ImagePlacement::default(),
            repeat: Repeat::Both,
        };
        let pt = Point { axes: [Vec3::new(0.04, 0.0, 0.0), Vec3::new(0.0, 0.01, 0.0)], ..at(Vec3::ZERO, Vec3::Z) };
        let t = taps(&p, (Vec3::splat(-1.0), Vec3::splat(1.0)), &pt).taps[0];
        assert_eq!(t.probes, 4);
        assert!((t.footprint - 0.005).abs() < 1e-6 && t.major.y.abs() < 1e-7, "{t:?}");
        // probes average along the axis; one outside a clipped image is left out
        let ramp = |uv: Vec2, _| covered(uv, [false, true]).then_some([uv.x; 4]);
        let tap = Tap { uv: Vec2::new(0.5, 0.5), weight: 1.0, footprint: 0.01, major: Vec2::new(0.3, 0.0), probes: 4 };
        assert!((filtered(&tap, 512, false, ramp).unwrap()[0] - 0.5).abs() < 1e-5);
        assert_eq!(filtered(&tap, 512, true, ramp).unwrap()[0], 0.5);
        let edge = Tap { uv: Vec2::new(0.95, 0.5), ..tap };
        assert!(filtered(&edge, 512, false, ramp).unwrap()[0] < 0.95);
    }
}
