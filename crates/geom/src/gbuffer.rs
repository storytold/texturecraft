//! The UV-space G-buffer: which triangle (and where on it) each texel of a texture set maps
//! to. Painting, compositing, baking and padding all work from this one buffer, so they agree
//! on which texel is which surface point.

use glam::{Vec2, Vec3, Vec4};
use rayon::prelude::*;

use crate::Mesh;

/// Texel not covered by any triangle.
pub const NO_TRI: u32 = u32::MAX;

const BAND: u32 = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct UvGBuffer {
    pub width: u32,
    pub height: u32,
    /// Triangle under each texel center, [`NO_TRI`] when uncovered.
    pub tri: Vec<u32>,
    /// Barycentric weights of the triangle's second and third vertices.
    pub bary: Vec<[f32; 2]>,
    /// Covers past the first on texels where any cover has the texel center strictly inside (overlapping UVs; an edge two neighbours share is no overlap)
    pub overlaps: u32,
}

/// Interpolated surface attributes at a texel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    pub pos: Vec3,
    /// Interpolated shading normal (normalized).
    pub nrm: Vec3,
    /// Flat triangle normal (for ray offsets).
    pub geo_nrm: Vec3,
    pub tangent: Vec4,
    pub uv: Vec2,
    pub tri: u32,
}

fn edge(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

/// Twice-areas (in texels²) below this are degenerate triangles, which cover nothing
const MIN_DOUBLE_AREA: f32 = 1e-12;

/// How far outside a triangle (as a share of its double area) a texel center may sit and still count as covered, so shared edges leave no gaps
const EDGE_TOLERANCE: f32 = 1e-6;

impl UvGBuffer {
    /// Rasterize the triangles of material `material` at `width × height`
    pub fn new(mesh: &Mesh, material: usize, width: u32, height: u32) -> UvGBuffer {
        let (width, height) = (width.max(1), height.max(1));
        let size = Vec2::new(width as f32, height as f32);
        let corner_uvs = |triangle: usize| {
            let [first, second, third] = mesh.tri(triangle)?;
            Some([*mesh.uvs.get(first)?, *mesh.uvs.get(second)?, *mesh.uvs.get(third)?])
        };

        // Sort triangles into bands of rows so each band rasterizes on its own thread
        let mut bands: Vec<Vec<u32>> = vec![Vec::new(); height.div_ceil(BAND) as usize];
        for triangle in mesh.triangles_of(material) {
            let Some(uvs) = corner_uvs(triangle) else { continue };
            let top = (uvs.iter().map(|uv| uv.y).fold(f32::INFINITY, f32::min) * size.y).floor().max(0.0) as i64;
            let bottom = (uvs.iter().map(|uv| uv.y).fold(f32::NEG_INFINITY, f32::max) * size.y).ceil().min(size.y) as i64;
            if bottom <= top || !uvs.iter().all(|uv| uv.is_finite()) {
                continue;
            }

            let (first_band, last_band) = ((top / BAND as i64) as usize, ((bottom - 1).max(0) / BAND as i64) as usize);
            for band in bands.iter_mut().take(last_band + 1).skip(first_band) {
                band.push(triangle as u32);
            }
        }

        let texel_count = width as usize * height as usize;
        let mut tri = vec![NO_TRI; texel_count];
        let mut bary = vec![[0.0f32; 2]; texel_count];
        let band_texels = (BAND * width) as usize;
        let overlaps: u32 = tri
            .par_chunks_mut(band_texels)
            .zip(bary.par_chunks_mut(band_texels))
            .enumerate()
            .map(|(band_index, (band_tri, band_bary))| {
                // Per texel: how many triangles cover it, and whether any has its center strictly inside, so the count doesn't depend on triangle order
                let mut covers = vec![0u32; band_tri.len()];
                let mut strict_cover = vec![false; band_tri.len()];
                let first_row = band_index as u32 * BAND;
                let band_rows = (band_tri.len() as u32) / width;
                for &triangle in bands.get(band_index).map(Vec::as_slice).unwrap_or(&[]) {
                    let Some(uvs) = corner_uvs(triangle as usize) else { continue };
                    let [a, b, c] = uvs.map(|uv| uv * size);
                    let double_area = edge(a, b, c);
                    if double_area.abs() < MIN_DOUBLE_AREA {
                        continue;
                    }

                    let tolerance = EDGE_TOLERANCE * double_area.abs();
                    let (sign, inverse_area) = (double_area.signum(), 1.0 / double_area.abs());
                    let left = a.x.min(b.x).min(c.x).floor().max(0.0) as u32;
                    let right = (a.x.max(b.x).max(c.x).ceil().min(width as f32) as u32).min(width);
                    let top = (a.y.min(b.y).min(c.y).floor().max(first_row as f32) as u32).max(first_row);
                    let bottom = (a.y.max(b.y).max(c.y).ceil() as u32).min(first_row + band_rows);
                    for y in top..bottom {
                        for x in left..right {
                            let center = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                            let weights = [edge(b, c, center) * sign, edge(c, a, center) * sign, edge(a, b, center) * sign];
                            if weights.iter().any(|weight| *weight < -tolerance) {
                                continue;
                            }

                            let index = ((y - first_row) * width + x) as usize;
                            let (Some(slot), Some(slot_bary), Some(cover_count), Some(strict)) =
                                (band_tri.get_mut(index), band_bary.get_mut(index), covers.get_mut(index), strict_cover.get_mut(index))
                            else {
                                continue;
                            };

                            *cover_count += 1;
                            *strict |= weights.iter().all(|weight| *weight > tolerance);
                            *slot = triangle;
                            *slot_bary = [weights[1] * inverse_area, weights[2] * inverse_area];
                        }
                    }
                }

                // Every cover past the first overlaps, unless all of them only touch the texel center with an edge (neighbours sharing it)
                covers.iter().zip(&strict_cover).filter(|(_, strict)| **strict).map(|(count, _)| count.saturating_sub(1)).sum::<u32>()
            })
            .sum();

        UvGBuffer { width, height, tri, bary, overlaps }
    }

    pub fn covered(&self, i: usize) -> bool {
        self.tri.get(i).is_some_and(|t| *t != NO_TRI)
    }

    pub fn coverage_mask(&self) -> Vec<bool> {
        self.tri.iter().map(|t| *t != NO_TRI).collect()
    }

    pub fn covered_count(&self) -> usize {
        self.tri.iter().filter(|t| **t != NO_TRI).count()
    }

    /// Surface attributes at texel index `i` (None when uncovered).
    pub fn surface(&self, mesh: &Mesh, i: usize) -> Option<Surface> {
        let t = *self.tri.get(i)?;
        if t == NO_TRI {
            return None;
        }
        let [b1, b2] = *self.bary.get(i)?;
        surface_at(mesh, t, b1, b2)
    }
}

/// Interpolate triangle `t` at barycentrics (weights of vertices 1 and 2).
pub fn surface_at(mesh: &Mesh, t: u32, b1: f32, b2: f32) -> Option<Surface> {
    let [a, b, c] = mesh.tri(t as usize)?;
    let b0 = 1.0 - b1 - b2;
    let p = |v: &[Vec3]| -> Option<Vec3> { Some(*v.get(a)? * b0 + *v.get(b)? * b1 + *v.get(c)? * b2) };
    let pos = p(&mesh.positions)?;
    let nrm = p(&mesh.normals)?.try_normalize().unwrap_or(Vec3::Y);
    let (pa, pb, pc) = (*mesh.positions.get(a)?, *mesh.positions.get(b)?, *mesh.positions.get(c)?);
    let geo_nrm = (pb - pa).cross(pc - pa).try_normalize().unwrap_or(nrm);
    let tan = *mesh.tangents.get(a)? * b0 + *mesh.tangents.get(b)? * b1 + *mesh.tangents.get(c)? * b2;
    let uv = *mesh.uvs.get(a)? * b0 + *mesh.uvs.get(b)? * b1 + *mesh.uvs.get(c)? * b2;
    let tdir = tan.truncate().try_normalize().unwrap_or_else(|| nrm.any_orthonormal_vector());
    let w = mesh.tangents.get(a).map(|t| if t.w < 0.0 { -1.0 } else { 1.0 }).unwrap_or(1.0);
    Some(Surface { pos, nrm, geo_nrm, tangent: tdir.extend(w), uv, tri: t })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn half_triangle_covers_texels_with_centers_inside() {
        // One triangle covering the lower-left half of UV space (v down: y = v·H).
        let m =
            Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y], None, vec![Vec2::new(0.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0)], vec![0, 1, 2], "m")
                .unwrap();
        let g = UvGBuffer::new(&m, 0, 8, 8);
        for y in 0..8u32 {
            for x in 0..8u32 {
                let inside = (x as f32 + 0.5) <= (y as f32 + 0.5);
                assert_eq!(g.covered((y * 8 + x) as usize), inside, "({x},{y})");
            }
        }
        assert_eq!(g.overlaps, 0);
    }

    #[test]
    fn neighbours_sharing_an_edge_through_texel_centers_do_not_overlap() {
        // An 8×8 grid of quads at 512 texels: each quad's diagonal runs through 64 texel centers
        let grid = primitives::plane(1.0, 8);
        let gbuffer = UvGBuffer::new(&grid, 0, 512, 512);
        assert_eq!(gbuffer.covered_count(), 512 * 512);
        assert_eq!(gbuffer.overlaps, 0);
    }

    #[test]
    fn an_edge_through_a_covered_texel_center_counts_both_edge_covers() {
        // The texel center (0.5, 0.5) lies inside the first triangle and on the diagonal the other two share
        let uv = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.2), Vec2::new(0.2, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)];
        let positions = vec![Vec3::ZERO; uv.len()];
        // In either order: the inside cover first, or the two edge covers first
        for indices in [vec![0, 1, 2, 0, 3, 4, 0, 5, 3], vec![0, 3, 4, 0, 5, 3, 0, 1, 2]] {
            let m = Mesh::from_parts(positions.clone(), None, uv.clone(), indices.clone(), "m").unwrap();
            assert_eq!(UvGBuffer::new(&m, 0, 1, 1).overlaps, 2, "{indices:?}");
        }
    }

    #[test]
    fn overlapping_and_degenerate_triangles() {
        let uv = vec![Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(0.5, 0.5)];
        let m = Mesh::from_parts(vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z], None, uv, vec![0, 1, 2, 0, 1, 2, 0, 3, 3], "m").unwrap();
        let g = UvGBuffer::new(&m, 0, 16, 16);
        assert!(g.overlaps > 0);
        assert!(g.covered_count() > 0);
    }

    #[test]
    fn sphere_surface_lies_on_the_sphere() {
        let m = primitives::sphere(2.0, 64, 32);
        let g = UvGBuffer::new(&m, 0, 128, 128);
        assert!(g.covered_count() > 128 * 128 * 9 / 10, "{}", g.covered_count());
        for i in (0..g.tri.len()).step_by(97) {
            if let Some(s) = g.surface(&m, i) {
                assert!((s.pos.length() - 2.0).abs() < 0.02);
                assert!(s.nrm.dot(s.pos.normalize()) > 0.99);
            }
        }
    }
}
