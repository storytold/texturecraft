//! Displacement for the path tracers: the mesh's dense copy (the one the viewport displaces in its
//! vertex shader) moved along its normals by the height channel, as the viewport moves it, so
//! Rendering mode shows the same silhouettes and the displaced surface casts its own shadows.

use std::sync::Arc;

use texturecraft_core::Channel;
use texturecraft_core::project::DisplaySettings;
use texturecraft_geom::glam::{Vec2, Vec3, Vec4};
use texturecraft_geom::{Bvh, Mesh, SubMesh};

use crate::subdivide::{self, Draw, Vertex};
use crate::trace::TraceSet;

/// The geometry the path tracers trace for `display`: displaced when the display displaces and a
/// set has height to displace by, else `mesh` and `bvh` as they are.
pub fn trace_geometry(mesh: &Arc<Mesh>, bvh: &Arc<Bvh>, sets: &[TraceSet], display: &DisplaySettings) -> (Arc<Mesh>, Arc<Bvh>) {
    match display.displacement.then(|| displaced(mesh, sets)).flatten() {
        Some(m) => {
            let bvh = Arc::new(Bvh::new(&m));
            (Arc::new(m), bvh)
        }
        None => (mesh.clone(), bvh.clone()),
    }
}

/// `mesh` subdivided like the viewport's dense copy and displaced by the height of the set over
/// each material (None when no set has height and a height strength).
pub fn displaced(mesh: &Mesh, sets: &[TraceSet]) -> Option<Mesh> {
    if !sets.iter().any(|s| s.height_strength != 0.0 && s.composite.channels.contains_key(&Channel::Height)) {
        return None;
    }
    let verts = subdivide::vertices(mesh);
    let draws: Vec<Draw> = if mesh.submeshes.is_empty() {
        vec![(0, u32::try_from(mesh.indices.len()).ok()?, 0)]
    } else {
        mesh.submeshes.iter().map(|s| (s.start, s.count, s.material)).collect()
    };
    let (lo, hi) = mesh.bounds();
    let world_per_uv = subdivide::world_per_uv(&verts, &mesh.indices).unwrap_or(((hi - lo).length() * 0.5).max(1e-3));
    let (dense, indices, dense_draws) = subdivide::subdivide(&verts, &mesh.indices, &draws, subdivide::level_for(mesh.triangle_count()));
    // UV distance between neighbouring vertices: the viewport samples the mip that matches it.
    let spacing = 1.0 / ((indices.len() / 3).max(1) as f32 * 0.5).sqrt();
    let heights: Vec<Option<Heights>> = sets.iter().map(|s| Heights::new(s, spacing)).collect();
    // Every dense vertex belongs to one triangle grid, so to one draw.
    let mut material = vec![usize::MAX; dense.len()];
    for &(start, count, m) in &dense_draws {
        for i in indices.get(start as usize..(start as usize).saturating_add(count as usize)).unwrap_or_default() {
            if let Some(slot) = material.get_mut(*i as usize) {
                *slot = m;
            }
        }
    }
    let lift = |v: &Vertex, m: usize| -> Vec3 {
        let p = Vec3::from(v.pos);
        let Some((set, h)) = sets.iter().zip(&heights).find(|(s, _)| s.material == m) else { return p };
        let n = Vec3::from(v.nrm).normalize_or_zero();
        p + n * (h.as_ref().map_or(0.0, |h| h.sample(Vec2::from(v.uv))) * set.height_strength * world_per_uv / 16.0)
    };
    let positions: Vec<Vec3> = dense.iter().zip(&material).map(|(v, m)| lift(v, *m)).collect();
    let names = |m: usize| mesh.submeshes.iter().find(|s| s.material == m).map(|s| s.name.clone()).unwrap_or_default();
    Some(Mesh {
        positions,
        normals: dense.iter().map(|v| Vec3::from(v.nrm)).collect(),
        tangents: dense.iter().map(|v| Vec4::from(v.tan)).collect(),
        uvs: dense.iter().map(|v| Vec2::from(v.uv)).collect(),
        colors: Vec::new(),
        submeshes: if mesh.submeshes.is_empty() {
            Vec::new()
        } else {
            dense_draws.iter().map(|&(start, count, m)| SubMesh { name: names(m), material: m, start, count }).collect()
        },
        indices,
        materials: mesh.materials.clone(),
        diagonals: Vec::new(),
    })
}

/// A set's height, box-filtered by `k` texels (the mip level the viewport samples).
struct Heights {
    w: usize,
    h: usize,
    data: Vec<f32>,
    tiled: bool,
}

impl Heights {
    fn new(s: &TraceSet, spacing: f32) -> Option<Heights> {
        if s.height_strength == 0.0 || !s.composite.channels.contains_key(&Channel::Height) {
            return None;
        }
        let (w0, h0) = (s.composite.resolution.max(1), s.composite.height.max(1));
        let mip = (w0 as f32 * spacing).max(1.0).log2().floor().clamp(0.0, 6.0) as u32;
        let k = 1u32 << mip;
        let (w, h) = ((w0 / k).max(1), (h0 / k).max(1));
        let mut data = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.0;
                for dy in 0..k {
                    for dx in 0..k {
                        sum += s.composite.value(Channel::Height, (x * k + dx).min(w0 - 1), (y * k + dy).min(h0 - 1))[0];
                    }
                }
                data.push(sum / (k * k) as f32);
            }
        }
        Some(Heights { w: w as usize, h: h as usize, data, tiled: s.tiled })
    }

    /// Bilinear at `uv`, clamped for UV-tiled sets and repeated across UV tile edges for classic offset sets (matching the viewport's height sampler).
    fn sample(&self, uv: Vec2) -> f32 {
        let at = |x: isize, y: isize| {
            let (x, y) = if self.tiled {
                (x.clamp(0, self.w as isize - 1) as usize, y.clamp(0, self.h as isize - 1) as usize)
            } else {
                (x.rem_euclid(self.w as isize) as usize, y.rem_euclid(self.h as isize) as usize)
            };
            self.data.get(y * self.w + x).copied().unwrap_or(0.0)
        };
        let (fx, fy) = (uv.x * self.w as f32 - 0.5, uv.y * self.h as f32 - 0.5);
        if !(fx.is_finite() && fy.is_finite()) {
            return 0.0;
        }
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
        let bottom = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
        top * (1.0 - ty) + bottom * ty
    }
}

#[cfg(test)]
mod tests {
    use texturecraft_compose::Composite;
    use texturecraft_geom::primitives;

    use super::*;

    /// A plane's set with a uniform height.
    fn set(height: f32, strength: f32) -> TraceSet {
        let mut composite = Composite::new(&[Channel::BaseColor, Channel::Height], 64);
        if let Some(img) = composite.channels.get_mut(&Channel::Height) {
            for (x, y) in texturecraft_compose::all_tiles(64, 64) {
                img.tile_mut(x, y, &[texturecraft_texel::to_u16_signed(height)]);
            }
        }
        TraceSet { material: 0, composite, height_strength: strength, tiled: false }
    }

    #[test]
    fn uniform_height_lifts_the_dense_plane_along_its_normal() {
        let plane = primitives::plane(2.0, 1);
        assert!(displaced(&plane, &[set(0.5, 0.0)]).is_none(), "no strength, nothing to displace");
        assert!(displaced(&plane, &[]).is_none());
        let m = displaced(&plane, &[set(0.5, 2.0)]).unwrap();
        assert!(m.triangle_count() > plane.triangle_count());
        assert_eq!((m.positions.len(), m.uvs.len(), m.normals.len()), (m.tangents.len(), m.positions.len(), m.positions.len()));
        // world per UV is 2 (a 2 × 2 plane over the UV square): 0.5 × 2 × 2 / 16.
        let n = plane.normals[0];
        let lift = m.positions.iter().map(|p| p.dot(n)).collect::<Vec<f32>>();
        assert!(lift.iter().all(|d| (d - 0.125).abs() < 1e-3), "{:?}", &lift[..4]);
        let off = DisplaySettings::default();
        let (mesh, bvh) = (Arc::new(plane), Arc::new(Bvh::new(&primitives::plane(2.0, 1))));
        assert!(Arc::ptr_eq(&trace_geometry(&mesh, &bvh, &[set(0.5, 2.0)], &off).0, &mesh), "off unless the display displaces");
        let on = DisplaySettings { displacement: true, ..DisplaySettings::default() };
        assert!(!Arc::ptr_eq(&trace_geometry(&mesh, &bvh, &[set(0.5, 2.0)], &on).0, &mesh));
    }

    #[test]
    fn sample_repeats_across_offset_uvs() {
        let heights = Heights { w: 4, h: 4, data: (0..16).map(|v| v as f32 / 15.0).collect(), tiled: false };
        let val_origin = heights.sample(Vec2::new(0.25, 0.25));
        let val_offset = heights.sample(Vec2::new(1.25, 2.25));
        let val_neg_offset = heights.sample(Vec2::new(-0.75, -0.75));
        assert!((val_origin - val_offset).abs() < 1e-6);
        assert!((val_origin - val_neg_offset).abs() < 1e-6);
    }

    #[test]
    fn sample_clamps_tiled_sets_and_repeats_classic_sets() {
        let mut data = vec![0.0f32; 16];
        for y in 0..4 {
            data[y * 4] = 1.0;
        }
        let tiled = Heights { w: 4, h: 4, data: data.clone(), tiled: true };
        let classic = Heights { w: 4, h: 4, data, tiled: false };

        let tiled_edge = tiled.sample(Vec2::new(0.0, 0.5));
        assert!((tiled_edge - 1.0).abs() < 1e-5, "tiled should clamp without bleeding opposite column: {tiled_edge}");

        let classic_edge = classic.sample(Vec2::new(0.0, 0.5));
        assert!((classic_edge - 0.5).abs() < 1e-5, "classic should repeat and blend opposite column: {classic_edge}");

        let offset = classic.sample(Vec2::new(1.0, 0.5));
        assert!((offset - classic_edge).abs() < 1e-5);
    }
}
