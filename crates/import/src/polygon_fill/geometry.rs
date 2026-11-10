//! [`SetGeometry`]: the mesh parts of one texture set as polygon-fill picks need them: corners in UV space (the original's
//! convention, V up) and in 3D, a BVH for clicks in the 3D view, and per part the source polygons, connected pieces and
//! UV islands a picked triangle grows to. Topology and BVH are built on first use and kept.
//!
//! Vertices are welded by exact position, so UV seams and hard edges (where vertices are split) still connect in 3D;
//! two triangles join across an edge only when exactly two triangles share it. A UV island joins them only where both
//! sides have the same UVs at the edge's ends.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;

use glam::Vec3;
use texturecraft_geom::{Bvh, Mesh};

/// UVs closer than this are the same point (UVs round-trip through text and floats).
const UV_EPS: f32 = 1e-6;

/// A part's source polygons: consecutive triangles, each polygon starting at a flagged triangle.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FaceGroups {
    /// Start triangle of each polygon, ascending, then the triangle count.
    starts: Vec<usize>,
}

impl FaceGroups {
    /// One polygon per triangle.
    pub fn per_triangle(count: usize) -> FaceGroups {
        FaceGroups { starts: (0..=count).collect() }
    }

    /// From the per-triangle flags (true on a polygon's first triangle); the first triangle always starts one. Flags of
    /// another length than `count` give one polygon per triangle.
    pub fn from_starts(flags: &[bool], count: usize) -> FaceGroups {
        if flags.len() != count {
            return FaceGroups::per_triangle(count);
        }
        let mut starts: Vec<usize> = flags.iter().enumerate().filter(|(t, f)| **f || *t == 0).map(|(t, _)| t).collect();
        starts.push(count);
        FaceGroups { starts }
    }

    pub fn count(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    /// The triangles of polygon `f`.
    pub fn triangles(&self, f: usize) -> Range<usize> {
        match (self.starts.get(f), self.starts.get(f + 1)) {
            (Some(a), Some(b)) => *a..*b,
            _ => 0..0,
        }
    }

    /// The polygon holding triangle `t`.
    pub fn face_of(&self, t: usize) -> Option<usize> {
        if t >= *self.starts.last()? {
            return None;
        }
        Some(self.starts.partition_point(|s| *s <= t).saturating_sub(1))
    }
}

/// Connected pieces and UV islands of one part: a group id per triangle.
#[derive(Debug, Clone, PartialEq)]
pub struct Topology {
    pieces: Vec<u32>,
    islands: Vec<u32>,
}

impl Topology {
    pub fn piece_of(&self, t: usize) -> Option<u32> {
        self.pieces.get(t).copied()
    }

    pub fn island_of(&self, t: usize) -> Option<u32> {
        self.islands.get(t).copied()
    }
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while let Some(&p) = parent.get(x as usize) {
        if p == x {
            break;
        }
        let gp = parent.get(p as usize).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(x as usize) {
            *slot = gp;
        }
        x = p;
    }
    x
}

fn join(parent: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb
        && let Some(slot) = parent.get_mut(ra.max(rb) as usize)
    {
        *slot = ra.min(rb);
    }
}

fn roots(mut parent: Vec<u32>) -> Vec<u32> {
    (0..parent.len() as u32).map(|t| find(&mut parent, t)).collect()
}

/// One part of the set.
pub struct GeometryPart {
    pub name: String,
    /// The part's first triangle in the mesh.
    pub first: usize,
    /// Its triangle count.
    pub count: usize,
    pub faces: FaceGroups,
    topology: OnceLock<Topology>,
}

/// The set's parts on `mesh`.
pub struct SetGeometry<'m> {
    pub mesh: &'m Mesh,
    pub parts: Vec<GeometryPart>,
    bvh: OnceLock<Bvh>,
}

impl<'m> SetGeometry<'m> {
    /// The submeshes of `mesh` on material `slot`; `polygon_starts` holds per submesh (in the mesh's order) the per-triangle
    /// flags of its source polygons (missing or mismatched ones count one polygon per triangle).
    pub fn new(mesh: &'m Mesh, slot: usize, polygon_starts: &[Vec<bool>]) -> SetGeometry<'m> {
        let parts = mesh
            .submeshes
            .iter()
            .enumerate()
            .filter(|(_, s)| s.material == slot)
            .map(|(i, s)| {
                let count = (s.count / 3) as usize;
                let faces = polygon_starts.get(i).map_or_else(|| FaceGroups::per_triangle(count), |f| FaceGroups::from_starts(f, count));
                GeometryPart { name: s.name.clone(), first: (s.start / 3) as usize, count, faces, topology: OnceLock::new() }
            })
            .collect();
        SetGeometry { mesh, parts, bvh: OnceLock::new() }
    }

    /// The corner positions of triangle `t` of part `p`.
    pub fn corners(&self, p: &GeometryPart, t: usize) -> Option<[Vec3; 3]> {
        let [a, b, c] = self.corner_ids(p, t)?;
        Some([*self.mesh.positions.get(a)?, *self.mesh.positions.get(b)?, *self.mesh.positions.get(c)?])
    }

    /// The corner UVs of triangle `t` of part `p` in the original's convention (origin bottom-left, V up); the mesh
    /// stores glTF's (V down).
    pub fn uvs(&self, p: &GeometryPart, t: usize) -> Option<[[f32; 2]; 3]> {
        let ids = self.corner_ids(p, t)?;
        let uv = |i: usize| self.mesh.uvs.get(i).map(|v| [v.x, 1.0 - v.y]);
        Some([uv(ids[0])?, uv(ids[1])?, uv(ids[2])?])
    }

    fn corner_ids(&self, p: &GeometryPart, t: usize) -> Option<[usize; 3]> {
        if t >= p.count {
            return None;
        }
        self.mesh.tri(p.first + t)
    }

    /// The topology of part `p`.
    pub fn topology<'p>(&self, p: &'p GeometryPart) -> &'p Topology {
        p.topology.get_or_init(|| self.build_topology(p))
    }

    fn build_topology(&self, p: &GeometryPart) -> Topology {
        let n = p.count;
        let mut weld: HashMap<[u32; 3], u32> = HashMap::new();
        // welded vertex per corner, per triangle
        let mut tris: Vec<Option<[u32; 3]>> = Vec::with_capacity(n);
        for t in 0..n {
            let ids = self.corner_ids(p, t);
            tris.push(ids.and_then(|ids| {
                let mut w = [0u32; 3];
                for (k, i) in ids.iter().enumerate() {
                    let pos = self.mesh.positions.get(*i)?;
                    let key = pos.to_array().map(f32::to_bits);
                    let next = weld.len() as u32;
                    *w.get_mut(k)? = *weld.entry(key).or_insert(next);
                }
                Some(w)
            }));
        }
        // each welded edge: the triangles and their sides using it
        let mut edges: HashMap<[u32; 2], Vec<(u32, u8)>> = HashMap::new();
        for (t, w) in tris.iter().enumerate() {
            let Some(w) = w else { continue };
            for k in 0..3usize {
                let (a, b) = (w[k], w[(k + 1) % 3]);
                if a != b {
                    edges.entry([a.min(b), a.max(b)]).or_default().push((t as u32, k as u8));
                }
            }
        }
        let mut pieces: Vec<u32> = (0..n as u32).collect();
        let mut islands = pieces.clone();
        // deterministic order
        let mut list: Vec<([u32; 2], Vec<(u32, u8)>)> = edges.into_iter().collect();
        list.sort_by_key(|(k, _)| *k);
        for (key, users) in list {
            let [(a, ka), (b, kb)] = users.as_slice() else { continue };
            join(&mut pieces, *a, *b);
            if self.uv_continuous(p, key, (*a, *ka), (*b, *kb), &tris) {
                join(&mut islands, *a, *b);
            }
        }
        Topology { pieces: roots(pieces), islands: roots(islands) }
    }

    /// True if both sides of the welded edge `key` have the same UVs at its two ends.
    fn uv_continuous(&self, p: &GeometryPart, key: [u32; 2], a: (u32, u8), b: (u32, u8), tris: &[Option<[u32; 3]>]) -> bool {
        let side = |(t, k): (u32, u8)| -> Option<[[f32; 2]; 2]> {
            let (t, k) = (t as usize, k as usize);
            let ids = self.corner_ids(p, t)?;
            let w = (*tris.get(t)?)?;
            let (i, j) = (*ids.get(k)?, *ids.get((k + 1) % 3)?);
            let (ui, uj) = (self.mesh.uvs.get(i)?.to_array(), self.mesh.uvs.get(j)?.to_array());
            Some(if *w.get(k)? == key[0] { [ui, uj] } else { [uj, ui] })
        };
        let same = |x: [f32; 2], y: [f32; 2]| (x[0] - y[0]).abs() <= UV_EPS && (x[1] - y[1]).abs() <= UV_EPS;
        match (side(a), side(b)) {
            (Some(x), Some(y)) => same(x[0], y[0]) && same(x[1], y[1]),
            _ => false,
        }
    }

    /// The part and local triangle of mesh triangle `global`.
    pub fn locate(&self, global: u32) -> Option<(usize, usize)> {
        let g = global as usize;
        self.parts.iter().enumerate().find(|(_, p)| (p.first..p.first + p.count).contains(&g)).map(|(i, p)| (i, g - p.first))
    }

    /// The nearest triangle of the set the ray `origin + t * dir` (t > 0) hits: (part, triangle, distance in units of
    /// `dir`).
    pub fn ray_hit(&self, origin: Vec3, dir: Vec3) -> Option<(usize, usize, f32)> {
        let bvh = self.bvh.get_or_init(|| Bvh::new(self.mesh));
        let ranges: Vec<Range<usize>> = self.parts.iter().map(|p| p.first..p.first + p.count).collect();
        let hit = bvh.closest_filtered(origin, dir, f32::MAX, &|t| ranges.iter().any(|r| r.contains(&(t as usize))))?;
        let (p, t) = self.locate(hit.tri)?;
        Some((p, t, hit.t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_groups_follow_the_flags() {
        let f = FaceGroups::from_starts(&[true, false, false, true, true, false], 6);
        assert_eq!(f.count(), 3);
        assert_eq!(f.triangles(0), 0..3);
        assert_eq!(f.triangles(2), 4..6);
        assert_eq!((f.face_of(2), f.face_of(3), f.face_of(5), f.face_of(6)), (Some(0), Some(1), Some(2), None));
        assert_eq!(FaceGroups::from_starts(&[true], 3), FaceGroups::per_triangle(3), "wrong length");
        assert_eq!(FaceGroups::from_starts(&[false, false], 2).count(), 1, "the first triangle always starts one");
    }
}
