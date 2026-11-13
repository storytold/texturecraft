//! The internal triangle mesh.

use glam::{Vec2, Vec3, Vec4};

use crate::{GeomError, Result};

/// An edge by the bits of its two end positions, the lower one first
type EdgePositions = ([u32; 3], [u32; 3]);

/// A contiguous index range drawn with one material.
#[derive(Clone, Debug, PartialEq)]
pub struct SubMesh {
    pub name: String,
    pub material: usize,
    /// First index (into [`Mesh::indices`]) and index count.
    pub start: u32,
    pub count: u32,
}

/// A material's base color (what a color-map bake transfers): `factor` (linear RGBA) times an
/// optional texture sampled at the mesh UVs.
#[derive(Clone, Debug, PartialEq)]
pub struct BaseColor {
    pub factor: Vec4,
    pub width: u32,
    pub height: u32,
    /// `width × height` RGBA8 texels (color sRGB-encoded, alpha linear), rows top → bottom like
    /// the UVs' v; empty without a texture.
    pub texels: Vec<[u8; 4]>,
}

/// Triangle mesh in world space. UVs use the glTF convention (origin top-left, v down).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    /// xyz tangent, w bitangent sign.
    pub tangents: Vec<Vec4>,
    pub uvs: Vec<Vec2>,
    /// RGBA vertex colors in 0..1 (glTF: linear; OBJ, PLY: as stored): one per vertex, or empty.
    pub colors: Vec<Vec4>,
    pub indices: Vec<u32>,
    /// Index ranges per material and part (node / object); `name` is the part name.
    pub submeshes: Vec<SubMesh>,
    pub materials: Vec<String>,
    /// Per triangle, bit `i` set when edge `i` (corner `i` to corner `i + 1`) is a diagonal the
    /// importer added to split a polygon, which wireframes leave out. Empty: every edge is real.
    pub diagonals: Vec<u8>,
}

/// Hard cap on mesh size accepted from files (triangles).
pub const MAX_TRIANGLES: usize = 20_000_000;

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Axis-aligned bounds (zero box for an empty mesh).
    pub fn bounds(&self) -> (Vec3, Vec3) {
        if self.positions.is_empty() {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        self.positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
    }

    /// Bounding-sphere radius around the bounds center (at least a small epsilon).
    pub fn radius(&self) -> f32 {
        let (lo, hi) = self.bounds();
        ((hi - lo).length() * 0.5).max(1e-4)
    }

    /// Vertex indices of triangle `t`.
    pub fn tri(&self, t: usize) -> Option<[usize; 3]> {
        let i = self.indices.get(t * 3..t * 3 + 3)?;
        Some([i[0] as usize, i[1] as usize, i[2] as usize])
    }

    /// The edges wireframes draw: each vertex pair once (smaller index first), leaving out
    /// polygon diagonals ([`Mesh::diagonals`]) unless only one triangle has the edge (a UV seam
    /// split it from its twin), per material in `0..materials.len().max(1)`.
    pub fn wire_edges(&self) -> Vec<Vec<(u32, u32)>> {
        (0..self.materials.len().max(1))
            .map(|m| {
                // Edge → (real in some triangle, triangles using it), in first-seen order.
                let mut edges: Vec<((u32, u32), bool, u8)> = Vec::new();
                let mut at: std::collections::HashMap<(u32, u32), usize> = std::collections::HashMap::new();
                for t in self.triangles_of(m) {
                    let Some(v) = self.indices.get(t * 3..t * 3 + 3) else { continue };
                    let hidden = self.diagonals.get(t).copied().unwrap_or(0);
                    for (i, (a, b)) in [(v[0], v[1]), (v[1], v[2]), (v[2], v[0])].into_iter().enumerate() {
                        let key = (a.min(b), a.max(b));
                        let real = hidden >> i & 1 == 0;
                        let e = *at.entry(key).or_insert_with(|| {
                            edges.push((key, false, 0));
                            edges.len() - 1
                        });
                        if let Some((_, r, n)) = edges.get_mut(e) {
                            *r |= real;
                            *n = n.saturating_add(1);
                        }
                    }
                }
                edges.into_iter().filter(|(_, real, n)| *real || *n == 1).map(|(k, ..)| k).collect()
            })
            .collect()
    }

    /// Edges where the UV layout splits: two triangles meet there in 3D but not in UV space, per material in `0..materials.len().max(1)`
    pub fn uv_seam_edges(&self) -> Vec<Vec<(u32, u32)>> {
        // Signed zero denotes the same position on either side of an imported seam
        let bits = |position: Vec3| position.to_array().map(|coordinate| if coordinate == 0.0 { 0 } else { coordinate.to_bits() });
        let mut seams = vec![Vec::new(); self.materials.len().max(1)];
        // Match across materials before grouping the lines for texture-set visibility
        let mut sides: std::collections::HashMap<EdgePositions, Vec<(u32, u32, usize)>> = std::collections::HashMap::new();
        for material in 0..seams.len() {
            for triangle in self.triangles_of(material) {
                let Some(corners) = self.indices.get(triangle * 3..triangle * 3 + 3) else { continue };

                for (from, to) in [(corners[0], corners[1]), (corners[1], corners[2]), (corners[2], corners[0])] {
                    let (Some(from_position), Some(to_position)) = (self.positions.get(from as usize), self.positions.get(to as usize)) else { continue };

                    let (from_bits, to_bits) = (bits(*from_position), bits(*to_position));
                    let (edge, pair) = if from_bits <= to_bits { ((from_bits, to_bits), (from, to)) } else { ((to_bits, from_bits), (to, from)) };
                    sides.entry(edge).or_default().push((pair.0, pair.1, material));
                }
            }
        }

        let uv = |index: u32| self.uvs.get(index as usize).copied();
        for pairs in sides.into_values() {
            let Some(&(first_from, first_to, _)) = pairs.first() else { continue };

            if pairs.iter().any(|&(from, to, _)| (uv(from), uv(to)) != (uv(first_from), uv(first_to))) {
                for (from, to, material) in pairs {
                    if let Some(edges) = seams.get_mut(material) {
                        edges.push((from.min(to), from.max(to)));
                    }
                }
            }
        }
        for edges in &mut seams {
            edges.sort_unstable();
            edges.dedup();
        }

        seams
    }

    /// Material of triangle `t`.
    pub fn material_of(&self, t: usize) -> Option<usize> {
        let idx = (t * 3) as u32;
        self.submeshes.iter().find(|s| idx >= s.start && idx < s.start.saturating_add(s.count)).map(|s| s.material)
    }

    /// Triangle index range belonging to material `m` (all submeshes of that material).
    pub fn triangles_of(&self, m: usize) -> impl Iterator<Item = usize> + '_ {
        self.submeshes.iter().filter(move |s| s.material == m).flat_map(|s| (s.start / 3) as usize..((s.start + s.count) / 3) as usize)
    }

    /// Check that attribute arrays agree and every index is in range.
    pub fn validate(&self) -> Result<()> {
        let n = self.positions.len();
        if self.normals.len() != n || self.uvs.len() != n || self.tangents.len() != n {
            return Err(GeomError::Invalid(format!(
                "attribute counts differ: {} positions, {} normals, {} uvs, {} tangents",
                n,
                self.normals.len(),
                self.uvs.len(),
                self.tangents.len()
            )));
        }
        if !self.colors.is_empty() && self.colors.len() != n {
            return Err(GeomError::Invalid(format!("{} vertex colors for {n} vertices", self.colors.len())));
        }
        if !self.indices.len().is_multiple_of(3) {
            return Err(GeomError::Invalid("index count is not a multiple of 3".into()));
        }
        if self.triangle_count() > MAX_TRIANGLES {
            return Err(GeomError::Invalid(format!("mesh has more than {MAX_TRIANGLES} triangles")));
        }
        if let Some(bad) = self.indices.iter().find(|i| **i as usize >= n) {
            return Err(GeomError::Invalid(format!("index {bad} out of range ({n} vertices)")));
        }
        if self.positions.iter().any(|p| !p.is_finite()) {
            return Err(GeomError::Invalid("mesh has non-finite positions".into()));
        }
        for s in &self.submeshes {
            if s.start as usize + s.count as usize > self.indices.len() || s.material >= self.materials.len().max(1) {
                return Err(GeomError::Invalid(format!("submesh {} is out of range", s.name)));
            }
        }
        Ok(())
    }

    /// Area-weighted smooth normals over the index buffer.
    pub fn compute_normals(&mut self) {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for &[a, b, c] in self.indices.as_chunks::<3>().0 {
            let (a, b, c) = (a as usize, b as usize, c as usize);
            let (Some(pa), Some(pb), Some(pc)) = (self.positions.get(a), self.positions.get(b), self.positions.get(c)) else { continue };
            let face = (*pb - *pa).cross(*pc - *pa);
            for v in [a, b, c] {
                if let Some(x) = normals.get_mut(v) {
                    *x += face;
                }
            }
        }

        self.normals = normals.into_iter().map(|v| v.try_normalize().unwrap_or(Vec3::Y)).collect();
    }

    /// MikkTSpace tangents (falls back to per-triangle tangents if generation fails).
    ///
    /// MikkTSpace gives every triangle corner its own tangent frame. A vertex whose corners get
    /// different frames (where mirrored UVs meet) is split into one vertex per frame, as
    /// MikkTSpace expects, so the corners of every triangle agree on the bitangent sign: the bake
    /// takes a triangle's sign from its first corner and the viewport interpolates it, and they
    /// then build the same frame. Copies keep the vertex's position, normal, UV and color, so
    /// nothing but the vertex count changes.
    ///
    /// A triangle MikkTSpace cannot orient can still get corners of both signs: one with two
    /// corners at the same position (a UV sphere's pole triangle on the seam), or one with area
    /// whose UVs collapse onto a line or a point. Each of its corners carries the frame of a
    /// neighbour that reaches it, or, where none does, MikkTSpace's initial frame (w = −1),
    /// which says nothing about the surface. All its corners take the sign most of its reached
    /// corners carry (the first reached corner's on a tie), so a vertex it shares with an
    /// oriented neighbour keeps that neighbour's sign and is not split.
    pub fn compute_tangents(&mut self) {
        self.tangents = vec![Vec4::new(1.0, 0.0, 0.0, 1.0); self.positions.len()];
        if self.normals.len() != self.positions.len() {
            self.compute_normals();
        }
        if self.uvs.len() != self.positions.len() {
            self.uvs = vec![Vec2::ZERO; self.positions.len()];
        }
        // mikktspace crashes (out-of-bounds) when every triangle is degenerate; skip it then.
        let usable = !crate::unwrap::all_triangles_degenerate(self);
        let mut g = Mikk { mesh: self, corners: vec![UNTOUCHED_CORNER; self.indices.len()], reached: vec![false; self.indices.len()] };
        if usable && mikktspace::generate_tangents(&mut g) {
            let (mut corners, reached) = (g.corners, g.reached);
            for (triangle, reached) in corners.as_chunks_mut::<3>().0.iter_mut().zip(reached.as_chunks::<3>().0) {
                let w = triangle_sign(triangle, reached);
                triangle.iter_mut().for_each(|t| t.w = w);
            }
            self.split_by_corner_tangents(&corners);
        } else {
            self.simple_tangents();
        }
        for (t, n) in self.tangents.iter_mut().zip(&self.normals) {
            // Orthonormalize against the normal; repair degenerate results.
            let v = t.truncate();
            let o = (v - *n * n.dot(v)).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
            *t = o.extend(if t.w < 0.0 { -1.0 } else { 1.0 });
        }
    }

    /// Give every index its corner's tangent: the first corner of a vertex sets its tangent,
    /// and each further distinct tangent gets a copy of the vertex.
    fn split_by_corner_tangents(&mut self, corners: &[Vec4]) {
        let mut first: Vec<Option<Vec4>> = vec![None; self.positions.len()];
        let mut copies: std::collections::HashMap<(u32, [u32; 4]), u32> = std::collections::HashMap::new();
        for (k, tangent) in corners.iter().enumerate() {
            let Some(&v) = self.indices.get(k) else { continue };
            let Some(seen) = first.get_mut(v as usize) else { continue };
            let vertex = match seen {
                None => {
                    *seen = Some(*tangent);
                    v
                }
                Some(t) if t.to_array().map(f32::to_bits) == tangent.to_array().map(f32::to_bits) => continue,
                Some(_) => *copies.entry((v, tangent.to_array().map(f32::to_bits))).or_insert_with(|| self.duplicate_vertex(v as usize)),
            };
            if let Some(t) = self.tangents.get_mut(vertex as usize) {
                *t = *tangent;
            }
            if let Some(i) = self.indices.get_mut(k) {
                *i = vertex;
            }
        }
    }

    fn simple_tangents(&mut self) {
        let mut tangents = vec![Vec3::ZERO; self.positions.len()];
        for &[a, b, c] in self.indices.as_chunks::<3>().0 {
            let (a, b, c) = (a as usize, b as usize, c as usize);
            let (Some(pa), Some(pb), Some(pc)) = (self.positions.get(a), self.positions.get(b), self.positions.get(c)) else { continue };
            let (Some(ua), Some(ub), Some(uc)) = (self.uvs.get(a), self.uvs.get(b), self.uvs.get(c)) else { continue };
            let (e1, e2) = (*pb - *pa, *pc - *pa);
            let (d1, d2) = (*ub - *ua, *uc - *ua);
            let determinant = d1.x * d2.y - d2.x * d1.y;
            if determinant.abs() < 1e-12 {
                continue;
            }

            let tangent = (e1 * d2.y - e2 * d1.y) / determinant;
            for v in [a, b, c] {
                if let Some(x) = tangents.get_mut(v) {
                    *x += tangent;
                }
            }
        }

        self.tangents = tangents.into_iter().map(|t| t.try_normalize().unwrap_or(Vec3::X).extend(1.0)).collect();
    }

    /// The UV tiles material `m` covers: integer parts of its triangles' UV centroids (v down),
    /// ascending. Non-finite UVs are skipped.
    pub fn uv_tiles(&self, m: usize) -> std::collections::BTreeSet<(i32, i32)> {
        self.triangles_of(m)
            .filter_map(|t| {
                let [a, b, c] = self.tri(t)?;
                let uv = (*self.uvs.get(a)? + *self.uvs.get(b)? + *self.uvs.get(c)?) / 3.0;
                uv.is_finite().then(|| (uv.x.floor() as i32, uv.y.floor() as i32))
            })
            .collect()
    }

    /// Remap the UVs of material `m`'s triangles with `f`. Vertices they share with other
    /// materials are duplicated first, so the other triangles keep their UVs.
    pub fn map_material_uvs(&mut self, m: usize, f: impl Fn(Vec2) -> Vec2) {
        self.map_material_uvs_with(m, |_, uv| f(uv));
    }

    /// [`Mesh::map_material_uvs`] where `f` also gets the UV centroid of the first of the
    /// material's triangles using the vertex: piecewise maps (UV tiles of different sizes) then
    /// place a vertex on a tile border with its own triangle's tile.
    pub fn map_material_uvs_with(&mut self, m: usize, f: impl Fn(Vec2, Vec2) -> Vec2) {
        let material_triangles: Vec<usize> = self.triangles_of(m).collect();
        let mut in_material = vec![false; self.triangle_count()];
        for t in &material_triangles {
            if let Some(x) = in_material.get_mut(*t) {
                *x = true;
            }
        }

        let mut shared = vec![false; self.positions.len()];
        for (_, tri) in self.indices.as_chunks::<3>().0.iter().enumerate().filter(|(t, _)| !in_material.get(*t).copied().unwrap_or(false)) {
            for v in tri {
                if let Some(s) = shared.get_mut(*v as usize) {
                    *s = true;
                }
            }
        }

        let mut duplicates = std::collections::HashMap::new();
        for k in material_triangles.iter().flat_map(|t| t * 3..t * 3 + 3) {
            let Some(&v) = self.indices.get(k) else { continue };
            let v =
                if shared.get(v as usize).copied().unwrap_or(false) { *duplicates.entry(v).or_insert_with(|| self.duplicate_vertex(v as usize)) } else { v };
            if let Some(i) = self.indices.get_mut(k) {
                *i = v;
            }
        }

        // (vertex, centroid of its first triangle), each vertex once
        let mut vertices = Vec::with_capacity(material_triangles.len() * 3);
        for t in &material_triangles {
            let Some(tri) = self.tri(*t) else { continue };
            let centroid = tri.iter().map(|v| self.uvs.get(*v).copied().unwrap_or_default()).sum::<Vec2>() / 3.0;
            vertices.extend(tri.map(|v| (v, centroid)));
        }

        vertices.sort_by_key(|(v, _)| *v);
        vertices.dedup_by_key(|(v, _)| *v);
        for (v, centroid) in vertices {
            if let Some(uv) = self.uvs.get_mut(v) {
                *uv = f(centroid, *uv);
            }
        }
    }

    /// Append a copy of vertex `v`; returns its index.
    fn duplicate_vertex(&mut self, v: usize) -> u32 {
        let i = self.positions.len() as u32;
        self.positions.push(self.positions.get(v).copied().unwrap_or_default());
        self.normals.push(self.normals.get(v).copied().unwrap_or(Vec3::Y));
        self.tangents.push(self.tangents.get(v).copied().unwrap_or(Vec4::new(1.0, 0.0, 0.0, 1.0)));
        self.uvs.push(self.uvs.get(v).copied().unwrap_or_default());
        if !self.colors.is_empty() {
            self.colors.push(self.colors.get(v).copied().unwrap_or(Vec4::ONE));
        }
        i
    }

    /// Append `other`, offsetting indices; its submeshes keep their material indices shifted by
    /// this mesh's material count.
    pub fn append(&mut self, other: &Mesh) {
        let base = self.positions.len() as u32;
        let ibase = self.indices.len() as u32;
        let mbase = self.materials.len();
        self.positions.extend_from_slice(&other.positions);
        self.normals.extend_from_slice(&other.normals);
        self.tangents.extend_from_slice(&other.tangents);
        self.uvs.extend_from_slice(&other.uvs);
        if !(self.colors.is_empty() && other.colors.is_empty()) {
            // Vertices of the side without colors become white.
            self.colors.resize(base as usize, Vec4::ONE);
            self.colors.extend(other.colors.iter().copied().chain(std::iter::repeat(Vec4::ONE)).take(other.positions.len()));
        }
        if !(self.diagonals.is_empty() && other.diagonals.is_empty()) {
            self.diagonals.resize(ibase as usize / 3, 0);
            self.diagonals.extend(other.diagonals.iter().copied().chain(std::iter::repeat(0)).take(other.triangle_count()));
        }
        self.indices.extend(other.indices.iter().map(|i| i + base));
        self.materials.extend(other.materials.iter().cloned());
        self.submeshes.extend(other.submeshes.iter().map(|s| SubMesh {
            name: s.name.clone(),
            material: s.material + mbase,
            start: s.start + ibase,
            count: s.count,
        }));
    }

    /// Uniformly scale and translate so the mesh is centered with the given bounding radius.
    pub fn normalize_to(&mut self, radius: f32) {
        let (lo, hi) = self.bounds();
        let c = (lo + hi) * 0.5;
        let s = radius / self.radius();
        for p in &mut self.positions {
            *p = (*p - c) * s;
        }
    }

    /// A single-material mesh from raw attributes; computes missing normals and tangents.
    pub fn from_parts(positions: Vec<Vec3>, normals: Option<Vec<Vec3>>, uvs: Vec<Vec2>, indices: Vec<u32>, material: &str) -> Result<Mesh> {
        let count = indices.len() as u32;
        let mut m = Mesh {
            normals: normals.unwrap_or_default(),
            uvs,
            positions,
            indices,
            tangents: Vec::new(),
            colors: Vec::new(),
            submeshes: vec![SubMesh { name: material.into(), material: 0, start: 0, count }],
            materials: vec![material.into()],
            diagonals: Vec::new(),
        };
        if m.normals.len() != m.positions.len() {
            m.compute_normals();
        }
        m.compute_tangents();
        m.validate()?;
        Ok(m)
    }
}

/// The tangent frame mikktspace 0.3 gives a corner it never reaches (`genTangSpace` fills its
/// frames with this before grouping): tangent +X, bitangent +Y, both magnitudes 1, not
/// orientation preserving, so w = −1. A reached corner can carry exactly this frame too (a face
/// along the axes, UVs at unit scale and wound clockwise); it then counts as unreached, which
/// can only matter in a triangle whose other reached corners disagree on the sign.
const UNTOUCHED_TANGENT: [f32; 3] = [1.0, 0.0, 0.0];
/// The untouched frame's bitangent; see [`UNTOUCHED_TANGENT`].
const UNTOUCHED_BITANGENT: [f32; 3] = [0.0, 1.0, 0.0];
/// The untouched frame's tangent and bitangent magnitudes; see [`UNTOUCHED_TANGENT`].
const UNTOUCHED_MAGNITUDE: f32 = 1.0;
/// The untouched frame as `set_tangent_encoded` would see it.
const UNTOUCHED_CORNER: Vec4 = Vec4::new(1.0, 0.0, 0.0, -1.0);

/// The bitangent sign all corners of a triangle take: the sign most of the corners MikkTSpace
/// reached carry (the first reached corner's on a tie), or the first corner's when it reached
/// none. Only a triangle MikkTSpace cannot orient (two corners at one position, or collapsed
/// UVs) mixes signs; its unreached corners carry the initial w = −1, which says nothing about
/// its neighbours.
fn triangle_sign(corners: &[Vec4; 3], reached: &[bool; 3]) -> f32 {
    let [first, ..] = corners;
    let mut signs = corners.iter().zip(reached).filter(|(_, r)| **r).map(|(t, _)| t.w);
    let Some(first_reached) = signs.next() else { return first.w };
    let vote = |w: f32| if w < 0.0 { -1 } else { 1 };
    match (signs.map(vote).sum::<i32>() + vote(first_reached)).cmp(&0) {
        std::cmp::Ordering::Greater => 1.0,
        std::cmp::Ordering::Less => -1.0,
        std::cmp::Ordering::Equal => first_reached,
    }
}

/// MikkTSpace's view of a mesh; it writes each triangle corner's tangent to `corners`.
struct Mikk<'a> {
    mesh: &'a Mesh,
    /// Each triangle corner's tangent, w the bitangent sign (16 bytes per corner)
    corners: Vec<Vec4>,
    /// Whether MikkTSpace gave the corner a frame (false: it kept the untouched one)
    reached: Vec<bool>,
}

impl Mikk<'_> {
    fn vi(&self, face: usize, vert: usize) -> usize {
        self.mesh.indices.get(face * 3 + vert).copied().unwrap_or(0) as usize
    }
}

impl mikktspace::Geometry for Mikk<'_> {
    fn num_faces(&self) -> usize {
        self.mesh.indices.len() / 3
    }
    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }
    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.mesh.positions.get(self.vi(face, vert)).copied().unwrap_or_default().to_array()
    }
    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.mesh.normals.get(self.vi(face, vert)).copied().unwrap_or(Vec3::Y).to_array()
    }
    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        // MikkTSpace expects OpenGL texture coordinates (v up); our UVs are glTF (v down). With
        // v flipped, the bitangent `cross(N, T) * w` points up the image, as glTF requires.
        let uv = self.mesh.uvs.get(self.vi(face, vert)).copied().unwrap_or_default();
        [uv.x, 1.0 - uv.y]
    }
    /// Record the corner's tangent with its sign, and whether MikkTSpace reached it.
    fn set_tangent(&mut self, tangent: [f32; 3], bi_tangent: [f32; 3], f_mag_s: f32, f_mag_t: f32, preserves_orientation: bool, face: usize, vert: usize) {
        let Some(k) = face.checked_mul(3).and_then(|k| k.checked_add(vert)) else { return };
        if let Some(t) = self.corners.get_mut(k) {
            *t = Vec3::from_array(tangent).extend(if preserves_orientation { 1.0 } else { -1.0 });
        }
        let untouched = tangent == UNTOUCHED_TANGENT
            && bi_tangent == UNTOUCHED_BITANGENT
            && f_mag_s == UNTOUCHED_MAGNITUDE
            && f_mag_t == UNTOUCHED_MAGNITUDE
            && !preserves_orientation;
        if let Some(r) = self.reached.get_mut(k) {
            *r = !untouched;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> Mesh {
        Mesh::from_parts(
            vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.0, 1.0, 0.0)],
            None,
            vec![Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 0.0)],
            vec![0, 1, 2, 0, 2, 3],
            "Default",
        )
        .unwrap()
    }

    #[test]
    fn normals_and_tangents_of_a_quad() {
        let m = quad();
        for n in &m.normals {
            assert!((*n - Vec3::Z).length() < 1e-5);
        }
        for t in &m.tangents {
            assert!((t.truncate() - Vec3::X).length() < 1e-4, "{t:?}");
        }
        assert_eq!(m.material_of(1), Some(0));
        assert_eq!(m.triangles_of(0).collect::<Vec<_>>(), vec![0, 1]);
    }

    /// Two triangles in the plane y = 0 that share only their apex at the origin, the second
    /// with its UVs mirrored in v: MikkTSpace gives the apex a frame with w = +1 in one and
    /// w = −1 in the other.
    fn bowtie() -> Mesh {
        let positions = [Vec3::ZERO, Vec3::new(-1.0, 0.0, -1.0), Vec3::new(-1.0, 0.0, 1.0), Vec3::new(1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, 1.0)];
        let uvs = positions.map(|p| Vec2::new(0.5 + 0.5 * p.x, 0.5 + 0.5 * p.z * if p.x > 0.0 { -1.0 } else { 1.0 }));
        Mesh::from_parts(positions.to_vec(), Some(vec![Vec3::Y; 5]), uvs.to_vec(), vec![0, 1, 2, 0, 4, 3], "Bowtie").unwrap()
    }

    /// `mesh` with every vertex merged into the first one with the same position, normal and UV:
    /// the layout before tangents split any vertex.
    fn welded(mesh: &Mesh) -> Mesh {
        let key = |v: usize| [mesh.positions[v].to_array(), mesh.normals[v].to_array(), [mesh.uvs[v].x, mesh.uvs[v].y, 0.0]].map(|a| a.map(f32::to_bits));
        let mut first = std::collections::HashMap::new();
        let indices = mesh.indices.iter().map(|v| *first.entry(key(*v as usize)).or_insert(*v)).collect();
        Mesh { indices, ..mesh.clone() }
    }

    /// MikkTSpace's tangent for every triangle corner of `mesh`, and whether it reached the corner
    fn mikktspace_frames(mesh: &Mesh) -> (Vec<Vec4>, Vec<bool>) {
        let mut g = Mikk { mesh, corners: vec![UNTOUCHED_CORNER; mesh.indices.len()], reached: vec![false; mesh.indices.len()] };
        assert!(mikktspace::generate_tangents(&mut g));
        (g.corners, g.reached)
    }

    /// MikkTSpace's tangent for every triangle corner of `mesh`
    fn mikktspace_corners(mesh: &Mesh) -> Vec<Vec4> {
        mikktspace_frames(mesh).0
    }

    /// A triangle A = (0, 1, 2) with real UVs, and two triangles with area whose UVs collapse
    /// onto the line v = 0.5: C1 = (3, 1, 0) shares A's edge 0-1, C2 = (0, 4, 3) shares C1's
    /// edge 0-3. MikkTSpace reaches C1 at 0 and 1 (from A) and C2 at 0 only; vertices 3 and 4
    /// belong to no triangle it can orient. A's UVs keep their orientation, so A has w = +1,
    /// the opposite of the untouched corners' w = −1: C1's first corner is untouched, and C2's
    /// only reached corner is its first.
    fn collapsed_uv_fan() -> Mesh {
        let positions = vec![Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(0.5, 0.0, -1.0), Vec3::new(-1.0, 0.0, -0.5)];
        let uvs = vec![Vec2::new(0.0, 0.5), Vec2::new(0.5, 0.5), Vec2::new(0.0, 0.0), Vec2::new(0.25, 0.5), Vec2::new(0.75, 0.5)];
        Mesh::from_parts(positions, Some(vec![Vec3::Y; 5]), uvs, vec![0, 1, 2, 3, 1, 0, 0, 4, 3], "Fan").unwrap()
    }

    /// Triangles with area but collapsed UVs take the sign MikkTSpace gave their reached
    /// corners, whichever corner comes first and however many corners it left untouched, so
    /// no vertex they share with a real triangle is split and no polygon diagonal shows.
    #[test]
    fn collapsed_uv_triangles_take_their_reached_corners_sign() {
        let m = collapsed_uv_fan();
        let (raw, reached) = mikktspace_frames(&m);
        assert_eq!(raw.iter().map(|t| t.w).collect::<Vec<_>>(), [1.0, 1.0, 1.0, -1.0, 1.0, 1.0, 1.0, -1.0, -1.0], "MikkTSpace mixes C1 and C2");
        assert_eq!(reached, [true, true, true, false, true, true, true, false, false]);
        assert!(m.indices.iter().all(|v| m.tangents[*v as usize].w == 1.0), "every corner takes A's sign: {:?}", m.tangents);
        assert_eq!(m.vertex_count(), 5, "no vertex is split");
        let mut shown = m.clone();
        shown.diagonals = vec![0b001, 0b010, 0];
        assert!(!shown.wire_edges()[0].contains(&(0, 1)), "the diagonal A and C1 share stays hidden");
    }

    /// The sign of most reached corners wins, the first reached one on a tie; untouched
    /// corners count only when no corner was reached.
    #[test]
    fn triangle_sign_follows_the_reached_corners() {
        let t = |w: f32| Vec4::new(1.0, 0.0, 0.0, w);
        assert_eq!(triangle_sign(&[t(-1.0), t(1.0), t(1.0)], &[false, true, true]), 1.0);
        assert_eq!(triangle_sign(&[t(1.0), t(-1.0), t(-1.0)], &[true, false, false]), 1.0);
        assert_eq!(triangle_sign(&[t(-1.0), t(1.0), t(-1.0)], &[true, true, true]), -1.0);
        assert_eq!(triangle_sign(&[t(-1.0), t(1.0), t(-1.0)], &[false, true, true]), 1.0);
        assert_eq!(triangle_sign(&[t(1.0), t(-1.0), t(-1.0)], &[false, false, false]), 1.0);
    }

    /// After `compute_tangents`, every triangle corner of a bowtie with a mirrored half and of a
    /// UV sphere carries MikkTSpace's own frame for that corner, and no triangle mixes signs.
    #[test]
    fn every_triangle_corner_keeps_its_mikktspace_frame() {
        for (name, split) in [("bowtie", bowtie()), ("UV sphere", crate::primitives::sphere(1.0, 32, 16))] {
            let unsplit = welded(&split);
            let corners = mikktspace_corners(&unsplit);
            let shared = || unsplit.indices.iter().zip(&corners);
            let vertex_needs_two_signs = shared().any(|(v, t)| shared().any(|(u, s)| u == v && s.w != t.w));
            let triangle_mixes_signs = corners.as_chunks::<3>().0.iter().any(|c| c.iter().any(|t| t.w != c[0].w));
            assert!(vertex_needs_two_signs || triangle_mixes_signs, "{name}: one tangent per vertex would mix signs in a triangle");
            let mut m = unsplit.clone();
            m.compute_tangents();
            m.validate().unwrap();
            for (t, (tri, wanted)) in m.indices.as_chunks::<3>().0.iter().zip(corners.as_chunks::<3>().0).enumerate() {
                let w = tri.iter().map(|v| m.tangents[*v as usize].w).collect::<Vec<_>>();
                assert!(w.iter().all(|sign| Some(sign) == w.first()), "{name}: triangle {t} mixes bitangent signs {w:?}");
                let [a, b, c] = unsplit.tri(t).unwrap().map(|v| unsplit.positions[v]);
                let zero_area = (b - a).cross(c - a).length() < 1e-12;
                for (k, (corner, wanted)) in tri.iter().zip(wanted).enumerate() {
                    let (v, old) = (*corner as usize, unsplit.indices[t * 3 + k] as usize);
                    assert_eq!(
                        (m.positions[v], m.normals[v], m.uvs[v]),
                        (unsplit.positions[old], unsplit.normals[old], unsplit.uvs[old]),
                        "{name}: corner moved"
                    );
                    let tangent = m.tangents[v];
                    assert!((tangent.truncate() - wanted.truncate().normalize()).length() < 1e-4, "{name}: tangent {tangent} is not MikkTSpace's {wanted}");
                    assert!(zero_area || tangent.w == wanted.w, "{name}: triangle {t} has the bitangent sign of another triangle");
                }
            }
        }
        let m = bowtie();
        assert_eq!(m.vertex_count(), 6, "the apex is split, nothing else");
        let mut again = m.clone();
        again.compute_tangents();
        assert_eq!(again, m, "a mesh already split splits no further");
    }

    #[test]
    fn wire_edges_leave_out_polygon_diagonals() {
        let mut m = quad();
        assert_eq!(m.wire_edges(), vec![vec![(0, 1), (1, 2), (0, 2), (2, 3), (0, 3)]], "triangles show every edge");
        // Edge 2 of (0, 1, 2) and edge 0 of (0, 2, 3) split the quad.
        m.diagonals = vec![0b100, 0b001];
        assert_eq!(m.wire_edges(), vec![vec![(0, 1), (1, 2), (2, 3), (0, 3)]]);
        // A diagonal whose twin a seam split off (vertex 4 copies vertex 0) stays.
        let mut seam = m.clone();
        seam.indices[3] = 4;
        assert_eq!(seam.wire_edges()[0].len(), 6);
        // Appending keeps each side's diagonals on its own triangles.
        let mut a = Mesh::from_parts(m.positions.clone(), None, m.uvs.clone(), vec![0, 1, 2], "A").unwrap();
        a.append(&m);
        assert_eq!(a.diagonals, vec![0, 0b100, 0b001]);
        assert_eq!(crate::primitives::plane(1.0, 2).wire_edges()[0].len(), 12, "a 2×2 grid of quads has 12 edges");
    }

    #[test]
    fn uv_seams_are_where_the_uv_layout_splits() {
        let mesh = quad();
        assert_eq!(mesh.uv_seam_edges(), vec![Vec::<(u32, u32)>::new()], "one UV island has no seam");

        // Vertex 4 copies vertex 0's position with another UV: the diagonal (0, 2) / (4, 2) is cut
        let mut seam = mesh.clone();
        seam.positions.push(seam.positions[0]);
        seam.normals.push(seam.normals[0]);
        seam.tangents.push(seam.tangents[0]);
        seam.uvs.push(Vec2::new(0.9, 0.9));
        seam.indices[3] = 4;
        assert_eq!(seam.uv_seam_edges(), vec![vec![(0, 2), (2, 4)]]);

        // The same split with the same UV is a hard edge, not a seam
        let mut hard = seam.clone();
        hard.uvs[4] = hard.uvs[0];
        assert_eq!(hard.uv_seam_edges(), vec![Vec::<(u32, u32)>::new()]);
    }

    #[test]
    fn uv_seams_join_positions_with_opposite_signed_zero() {
        let mut mesh = quad();
        mesh.positions.push(Vec3::new(-0.0, 0.0, 0.0));
        mesh.normals.push(mesh.normals[0]);
        mesh.tangents.push(mesh.tangents[0]);
        mesh.uvs.push(Vec2::new(0.9, 0.9));
        mesh.indices[3] = 4;
        assert_eq!(mesh.positions[0], mesh.positions[4]);
        assert_eq!(mesh.uv_seam_edges(), vec![vec![(0, 2), (2, 4)]]);
    }

    #[test]
    fn uv_seams_between_materials_belong_to_both_materials() {
        let mut mesh = quad();
        mesh.positions.push(mesh.positions[0]);
        mesh.normals.push(mesh.normals[0]);
        mesh.tangents.push(mesh.tangents[0]);
        mesh.uvs.push(Vec2::new(0.9, 0.9));
        mesh.indices[3] = 4;
        mesh.materials.push("second".into());
        mesh.submeshes[0].count = 3;
        mesh.submeshes.push(SubMesh { name: "second".into(), material: 1, start: 3, count: 3 });

        assert_eq!(mesh.uv_seam_edges(), vec![vec![(0, 2)], vec![(2, 4)]]);
        mesh.uvs[4] = mesh.uvs[0];
        assert_eq!(mesh.uv_seam_edges(), vec![Vec::<(u32, u32)>::new(); 2], "a material boundary alone is not a UV seam");
    }

    #[test]
    fn validate_catches_bad_indices() {
        let mut m = quad();
        m.indices[0] = 99;
        assert!(m.validate().is_err());
        let mut m = quad();
        m.uvs.pop();
        assert!(m.validate().is_err());
    }

    #[test]
    fn append_offsets_indices_and_materials() {
        let mut a = quad();
        let b = quad();
        a.append(&b);
        assert_eq!(a.indices[6], 4);
        assert_eq!(a.submeshes[1].material, 1);
        assert_eq!(a.submeshes[1].start, 6);
        a.validate().unwrap();
        // Colors on one side only: the other side's vertices become white.
        let mut red = quad();
        red.colors = vec![Vec4::new(1.0, 0.0, 0.0, 1.0); 4];
        a.append(&red);
        assert_eq!(a.colors.len(), 12);
        assert_eq!((a.colors[0], a.colors[8]), (Vec4::ONE, Vec4::new(1.0, 0.0, 0.0, 1.0)));
        a.append(&quad());
        assert_eq!(a.colors[15], Vec4::ONE);
        a.validate().unwrap();
        a.colors.pop();
        assert!(a.validate().is_err());
    }

    #[test]
    fn material_uvs_remap_and_split_shared_vertices() {
        // quad(): two triangles sharing an edge; put the second in its own material.
        let mut m = quad();
        m.materials = vec!["a".into(), "b".into()];
        m.submeshes = vec![SubMesh { name: "a".into(), material: 0, start: 0, count: 3 }, SubMesh { name: "b".into(), material: 1, start: 3, count: 3 }];
        let before = m.clone();
        m.map_material_uvs(1, |uv| uv + Vec2::X);
        m.validate().unwrap();
        assert_eq!(m.vertex_count(), 6, "the two shared vertices are split");
        assert_eq!(&m.indices[..3], &before.indices[..3]);
        for k in 0..3 {
            let (old, new) = (before.indices[k] as usize, m.indices[k] as usize);
            assert_eq!(m.uvs[new], before.uvs[old], "material a keeps its UVs");
            let (old, new) = (before.indices[3 + k] as usize, m.indices[3 + k] as usize);
            assert_eq!((m.uvs[new], m.positions[new]), (before.uvs[old] + Vec2::X, before.positions[old]));
        }
        assert_eq!(m.uv_tiles(1).into_iter().collect::<Vec<_>>(), vec![(1, 0)]);
        assert_eq!(m.uv_tiles(0).into_iter().collect::<Vec<_>>(), vec![(0, 0)]);
    }
}
