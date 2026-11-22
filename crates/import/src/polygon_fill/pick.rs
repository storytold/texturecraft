//! One polygon-fill hit → the triangles it covers. A click picks the triangle under it (in the 2D view the triangle
//! whose UVs contain the point, in the 3D view the nearest one the camera ray hits). A dragged rectangle in the 2D view
//! picks every triangle it touches in UV space (fitted against the original's stored result, where touching and "a
//! corner inside" pick the same and "the centre inside" misses triangles the original picked); with `collisionMode` 0
//! ("Contained center", newer files) it picks the units (triangles in triangle mode, else source polygons) whose UV
//! centre lies inside. The texture-space V axis of the 2D view points up (V flipped against the mesh's glTF convention).
//!
//! In the 3D view positions are screen coordinates with y up in a centred square as wide as the view's shorter side
//! (so one coordinate may leave 0…1); the stored camera is square, so they map to device coordinates as `2q - 1`. A
//! rectangle tests units (each triangle in triangle mode, else each source polygon) projected by the hit's camera
//! (measured on a two-cube test mesh, every case triangle for triangle):
//! - `collisionMode` 0 ("Contained center"): the unit's centre (mean of its distinct corners) lies inside;
//!   1 ("Intersection", the default): any of its triangles overlaps the rectangle.
//! - `occlusionCulling` ("Visible polygons only"): with the centre test the unit's centre must be what the camera sees;
//!   with the intersection test some part of the unit must be seen inside the rectangle, or the camera must see the
//!   centre of one of its triangles that overlap it. Off, hidden and back-facing units are picked too.
//! - Older files store neither and pick as "Intersection" with hidden units (fitted on a real project's mask groups).
//! - A rectangle of zero width or height picks nothing.
//!
//! Each picked unit then grows by the hit's granularity: to its source polygon, its connected piece of mesh or its UV
//! island.
//!
//! Symmetry copies (`duplicateCount`, `duplicateMatrix`, 3D view): the k-th copy picks as the hit would through the k-th
//! power of the matrix, in the space of the stored mesh: a click casts the mirrored or rotated camera ray and takes the
//! nearest triangle; a rectangle tests the units through the transformed camera. "Visible" is still judged by the hit's
//! own camera on the unit itself, and only the hit itself uses what is seen inside the rectangle.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use glam::{Mat4, Vec3, Vec4};
use texturecraft_import_reader::model::{DecalHit, HitGranularity, HitShape, StrokeView};

use super::geometry::{GeometryPart, SetGeometry};

/// Symmetry copies followed at most.
const MAX_COPIES: u64 = 64;

/// Why a hit could not be resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum PickProblem {
    /// A view type or granularity this importer does not know.
    Unknown(String),
    /// The hit's camera matrix cannot be inverted.
    BadCamera,
}

/// Picked triangles by part index (of [`SetGeometry::parts`]), local to the part.
pub type Picked = BTreeMap<usize, BTreeSet<u32>>;

fn mat(m: &[f32; 16]) -> Option<Mat4> {
    m.iter().all(|v| v.is_finite()).then(|| Mat4::from_cols_array(m))
}

fn inverse(m: &Mat4) -> Option<Mat4> {
    let d = m.determinant();
    (d.is_finite() && d.abs() > 1e-20).then(|| m.inverse()).filter(|i| i.is_finite())
}

/// The triangles `hit` covers.
pub fn pick(geometry: &SetGeometry, hit: &DecalHit) -> Result<Picked, PickProblem> {
    let mut picked = Picked::new();
    match hit.view {
        StrokeView::Uv => uv_pick(geometry, hit, &mut picked),
        StrokeView::Mesh => {
            let inv = mat(&hit.proj_view_inv).ok_or(PickProblem::BadCamera)?;
            let proj_view = inverse(&inv).ok_or(PickProblem::BadCamera)?;
            for copy in copies(hit)? {
                mesh_pick(geometry, hit, (&proj_view, &inv), &copy, &mut picked);
            }
        }
        StrokeView::Other(v) => return Err(PickProblem::Unknown(format!("view type {v}"))),
    }
    grow(geometry, hit.granularity, picked)
}

/// The mesh transforms one hit picks through: the identity, then each symmetry copy (the k-th copy maps the mesh by the
/// inverse of the duplicate matrix to the k-th power).
fn copies(hit: &DecalHit) -> Result<Vec<Mat4>, PickProblem> {
    let mut out = vec![Mat4::IDENTITY];
    if hit.duplicate_count == 0 {
        return Ok(out);
    }
    let back = mat(&hit.duplicate_matrix).as_ref().and_then(inverse).ok_or(PickProblem::BadCamera)?;
    let mut m = Mat4::IDENTITY;
    for _ in 0..hit.duplicate_count.min(MAX_COPIES) {
        m = back * m;
        out.push(m);
    }
    Ok(out)
}

/// Each triangle of a part as its own unit, or each source polygon.
fn units(part: &GeometryPart, per_triangle: bool) -> Vec<Range<usize>> {
    match per_triangle {
        true => (0..part.count).map(|t| t..t + 1).collect(),
        false => (0..part.faces.count()).map(|f| part.faces.triangles(f)).collect(),
    }
}

fn uv_pick(geometry: &SetGeometry, hit: &DecalHit, picked: &mut Picked) {
    if let (HitShape::Rect(a, b), Collision::Centre) = (hit.shape, rect_rules(hit).0) {
        return uv_centre_pick(geometry, hit.granularity, Rect::new(a, b), picked);
    }
    for (p, part) in geometry.parts.iter().enumerate() {
        for t in 0..part.count {
            let Some(uv) = geometry.uvs(part, t) else { continue };
            let inside = match hit.shape {
                HitShape::Point(q) => contains(uv, q),
                HitShape::Rect(a, b) => overlaps(uv, a, b),
            };
            if inside {
                picked.entry(p).or_default().insert(t as u32);
                if matches!(hit.shape, HitShape::Point(_)) {
                    return;
                }
            }
        }
    }
}

/// A 2D-view rectangle with "Contained center": each unit whose UV centre, the mean of its distinct corners, lies inside.
fn uv_centre_pick(geometry: &SetGeometry, granularity: HitGranularity, rect: Rect, picked: &mut Picked) {
    for (p, part) in geometry.parts.iter().enumerate() {
        for tris in units(part, granularity == HitGranularity::Triangle) {
            let mut corners: Vec<[f32; 2]> = Vec::new();
            for t in tris.clone() {
                for c in geometry.uvs(part, t).into_iter().flatten() {
                    if !corners.contains(&c) {
                        corners.push(c);
                    }
                }
            }
            if corners.is_empty() {
                continue;
            }
            let n = corners.len() as f32;
            let centre = [corners.iter().map(|c| c[0]).sum::<f32>() / n, corners.iter().map(|c| c[1]).sum::<f32>() / n];
            if rect.contains(centre) {
                picked.entry(p).or_default().extend(tris.map(|t| t as u32));
            }
        }
    }
}

fn mesh_pick(geometry: &SetGeometry, hit: &DecalHit, (proj_view, inv): (&Mat4, &Mat4), copy: &Mat4, picked: &mut Picked) {
    let ndc = |q: [f32; 2]| [2.0 * q[0] - 1.0, 2.0 * q[1] - 1.0];
    let back = inverse(copy).unwrap_or(Mat4::IDENTITY);
    // the ray through device point `q` in the mesh's frame for this copy
    let ray = |q: [f32; 2]| -> Option<(Vec3, Vec3)> {
        let (near, far) = (unproject(inv, q, -1.0)?, unproject(inv, q, 1.0)?);
        let (o, f) = (point(&back, near), point(&back, far));
        Some((o, f - o))
    };
    // the same through the hit's own camera, unmirrored: what "visible" is judged by, for copies too
    let camera_ray = |q: [f32; 2]| -> Option<(Vec3, Vec3)> {
        let (o, f) = (unproject(inv, q, -1.0)?, unproject(inv, q, 1.0)?);
        Some((o, f - o))
    };
    // true if the camera sees the point `centre` of the triangles `tris` of part `p` (nothing nearer on its ray)
    let seen = |p: usize, tris: &Range<usize>, centre: Vec3| -> bool {
        let h = *proj_view * centre.extend(1.0);
        h.w > 0.0
            && camera_ray([h.x / h.w, h.y / h.w]).is_some_and(|(o, dir)| {
                geometry.ray_hit(o, dir).is_none_or(|(hp, ht, d)| (hp == p && tris.contains(&ht)) || d >= distance(o, dir, centre) * 0.999)
            })
    };
    let is_identity = *copy == Mat4::IDENTITY;
    match hit.shape {
        HitShape::Point(q) => {
            let Some((o, dir)) = ray(ndc(q)) else { return };
            if let Some((p, t, _)) = geometry.ray_hit(o, dir) {
                picked.entry(p).or_default().insert(t as u32);
            }
        }
        HitShape::Rect(a, b) => {
            let rect = Rect::new(ndc(a), ndc(b));
            let m = *proj_view * *copy;
            let (collision, occlusion) = rect_rules(hit);
            // what the camera sees inside the rectangle (the hit itself only, not its symmetry copies)
            let visible = match (occlusion, collision, is_identity) {
                (true, Collision::Intersection, true) => Some(seen_in(geometry, &rect, &camera_ray)),
                _ => None,
            };
            for (p, part) in geometry.parts.iter().enumerate() {
                for tris in units(part, hit.granularity == HitGranularity::Triangle) {
                    let ok = match collision {
                        Collision::Centre => {
                            centre_inside(geometry, part, tris.clone(), &m, &rect)
                                && (!occlusion || unit_centre(geometry, part, tris.clone()).is_some_and(|c| seen(p, &tris, c)))
                        }
                        Collision::Intersection => {
                            let overlapping: Vec<usize> = tris
                                .clone()
                                .filter(|&t| {
                                    geometry.corners(part, t).is_some_and(|c| {
                                        let clip = c.map(|v| m * v.extend(1.0));
                                        clip.iter().all(|h| h.w > 0.0) && rect.overlaps(clip.map(|h| [h.x / h.w, h.y / h.w]))
                                    })
                                })
                                .collect();
                            !overlapping.is_empty()
                                && (!occlusion
                                    || overlapping.iter().any(|&t| unit_centre(geometry, part, t..t + 1).is_some_and(|c| seen(p, &tris, c)))
                                    || visible.as_ref().is_some_and(|v| tris.clone().any(|t| v.contains(&(p, t)))))
                        }
                    };
                    if ok {
                        picked.entry(p).or_default().extend(tris.map(|t| t as u32));
                    }
                }
            }
        }
    }
}

/// How a rectangle picks: a polygon's centre inside it, or any part of the polygon.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Collision {
    Centre,
    Intersection,
}

/// The rectangle rules of `hit`: its stored `collisionMode` and `occlusionCulling`; older files store neither and pick
/// any part of a polygon, hidden ones too.
fn rect_rules(hit: &DecalHit) -> (Collision, bool) {
    let collision = match hit.collision {
        Some(0) => Collision::Centre,
        _ => Collision::Intersection,
    };
    (collision, hit.occlusion_culling.unwrap_or(false))
}

/// A screen rectangle in device coordinates (or a UV rectangle).
struct Rect {
    lo: [f32; 2],
    hi: [f32; 2],
}

impl Rect {
    fn new(a: [f32; 2], b: [f32; 2]) -> Rect {
        Rect { lo: [a[0].min(b[0]), a[1].min(b[1])], hi: [a[0].max(b[0]), a[1].max(b[1])] }
    }

    /// Zero width or height (or not a number): the original picks nothing (a click recorded as a rectangle).
    fn is_empty(&self) -> bool {
        !(self.hi[0] > self.lo[0] && self.hi[1] > self.lo[1])
    }

    fn contains(&self, q: [f32; 2]) -> bool {
        !self.is_empty() && (self.lo[0]..=self.hi[0]).contains(&q[0]) && (self.lo[1]..=self.hi[1]).contains(&q[1])
    }

    fn overlaps(&self, t: [[f32; 2]; 3]) -> bool {
        !self.is_empty() && overlaps(t, self.lo, self.hi)
    }
}

/// True if the centre of polygon `tris` (the mean of its distinct corners) lies in front of the camera and inside `rect`.
fn centre_inside(geometry: &SetGeometry, part: &GeometryPart, tris: Range<usize>, m: &Mat4, rect: &Rect) -> bool {
    unit_centre(geometry, part, tris).is_some_and(|c| {
        let h = *m * c.extend(1.0);
        h.w > 0.0 && rect.contains([h.x / h.w, h.y / h.w])
    })
}

/// The centre of the triangles `tris` (the mean of their distinct corners).
fn unit_centre(geometry: &SetGeometry, part: &GeometryPart, tris: Range<usize>) -> Option<Vec3> {
    let mut seen = BTreeSet::new();
    let (mut sum, mut n) = (Vec3::ZERO, 0.0f32);
    for t in tris {
        let ids = geometry.mesh.tri(part.first + t)?;
        for (k, v) in ids.iter().enumerate() {
            if seen.insert(*v) {
                sum += *geometry.corners(part, t)?.get(k)?;
                n += 1.0;
            }
        }
    }
    (n > 0.0).then(|| sum / n)
}

/// The distance along `dir` (in units of `dir`, as the BVH reports hits) from `o` to `p`.
fn distance(o: Vec3, dir: Vec3, p: Vec3) -> f32 {
    (p - o).dot(dir) / dir.length_squared().max(1e-20)
}

/// Rays per two units of screen height (the view's pixels at 512 high; inferred, not measured).
const SEEN_RAYS_PER_HEIGHT: f32 = 512.0;
/// At most about this many rays per pick; the grid coarsens by [`SEEN_COARSEN`] until it fits (a cost guard).
const SEEN_MAX_RAYS: f32 = 1.0e6;
const SEEN_COARSEN: f32 = 1.5;

/// The triangles the camera sees inside `rect`: rays through a grid of 512 samples per unit of screen height, at most
/// about a million.
fn seen_in(geometry: &SetGeometry, rect: &Rect, ray: &impl Fn([f32; 2]) -> Option<(Vec3, Vec3)>) -> BTreeSet<(usize, usize)> {
    let mut out = BTreeSet::new();
    if rect.is_empty() {
        return out;
    }
    let (w, h) = (rect.hi[0] - rect.lo[0], rect.hi[1] - rect.lo[1]);
    if !(w.is_finite() && h.is_finite()) {
        return out;
    }
    let mut step = 2.0 / SEEN_RAYS_PER_HEIGHT;
    while (w / step) * (h / step) > SEEN_MAX_RAYS {
        step *= SEEN_COARSEN;
    }
    let (nx, ny) = ((w / step).ceil().max(1.0) as usize, (h / step).ceil().max(1.0) as usize);
    for j in 0..ny {
        for i in 0..nx {
            let q = [rect.lo[0] + (i as f32 + 0.5) * w / nx as f32, rect.lo[1] + (j as f32 + 0.5) * h / ny as f32];
            if let Some((o, dir)) = ray(q)
                && let Some((p, t, _)) = geometry.ray_hit(o, dir)
            {
                out.insert((p, t));
            }
        }
    }
    out
}

/// The world point at normalised device coordinates `q` and depth `z` (−1 near, 1 far).
fn unproject(inv: &Mat4, q: [f32; 2], z: f32) -> Option<Vec3> {
    let h = *inv * Vec4::new(q[0], q[1], z, 1.0);
    (h.w.abs() > 1e-12).then(|| h.truncate() / h.w).filter(|v| v.is_finite())
}

fn point(m: &Mat4, p: Vec3) -> Vec3 {
    let h = *m * p.extend(1.0);
    let w = if h.w.abs() > 1e-12 { h.w } else { 1.0 };
    h.truncate() / w
}

/// True if the triangle and the rectangle spanned by `a` and `b` share a point (separating axes).
fn overlaps(t: [[f32; 2]; 3], a: [f32; 2], b: [f32; 2]) -> bool {
    let lo = [a[0].min(b[0]), a[1].min(b[1])];
    let hi = [a[0].max(b[0]), a[1].max(b[1])];
    for k in 0..2 {
        if t.iter().all(|c| c[k] < lo[k]) || t.iter().all(|c| c[k] > hi[k]) {
            return false;
        }
    }
    let corners = [lo, [hi[0], lo[1]], hi, [lo[0], hi[1]]];
    for e in 0..3 {
        let (p, q, r) = (t[e], t[(e + 1) % 3], t[(e + 2) % 3]);
        let n = [q[1] - p[1], p[0] - q[0]];
        let side = |c: [f32; 2]| n[0] * (c[0] - p[0]) + n[1] * (c[1] - p[1]);
        let inner = side(r);
        if corners.iter().all(|c| side(*c) * inner < 0.0) {
            return false;
        }
    }
    true
}

/// True if `q` lies in the triangle (either winding, edges included).
fn contains(t: [[f32; 2]; 3], q: [f32; 2]) -> bool {
    let side = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]);
    let s = [side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0])];
    s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
}

/// Every picked triangle grown by `granularity`.
fn grow(geometry: &SetGeometry, granularity: HitGranularity, picked: Picked) -> Result<Picked, PickProblem> {
    let mut out = Picked::new();
    for (p, tris) in picked {
        let Some(part) = geometry.parts.get(p) else { continue };
        let grown = match granularity {
            HitGranularity::Triangle => tris,
            HitGranularity::Polygon => polygons(part, &tris),
            HitGranularity::Mesh => components(part, &tris, |t| geometry.topology(part).piece_of(t)),
            HitGranularity::UvIsland => components(part, &tris, |t| geometry.topology(part).island_of(t)),
            HitGranularity::Other(v) => return Err(PickProblem::Unknown(format!("granularity {v}"))),
        };
        out.insert(p, grown);
    }
    Ok(out)
}

fn polygons(part: &GeometryPart, tris: &BTreeSet<u32>) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for t in tris {
        match part.faces.face_of(*t as usize) {
            Some(f) => out.extend(part.faces.triangles(f).map(|t| t as u32)),
            None => {
                out.insert(*t);
            }
        }
    }
    out
}

fn components(part: &GeometryPart, tris: &BTreeSet<u32>, of: impl Fn(usize) -> Option<u32>) -> BTreeSet<u32> {
    let groups: BTreeSet<u32> = tris.iter().filter_map(|t| of(*t as usize)).collect();
    (0..part.count).filter(|t| of(*t).is_some_and(|g| groups.contains(&g))).map(|t| t as u32).collect()
}

#[cfg(test)]
mod tests;
