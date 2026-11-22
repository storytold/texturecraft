//! A mesh too large for one GPU buffer split into parts, each with its own vertices and indices within the device's
//! buffer limit: the triangles of each draw in order, a part closing when its next triangle would pass the limit. The
//! wireframe's edges go to the part holding an end of them (the other end copied in when it lives elsewhere).

use crate::subdivide::{Draw, Vertex};

/// One part: its vertices, triangles and draws (first index, count, material), and its wireframe lines likewise.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct MeshPart {
    pub verts: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub draws: Vec<Draw>,
    pub lines: Vec<u32>,
    pub line_draws: Vec<Draw>,
}

/// No part: a vertex's slot when no part holds it yet.
const NONE: (u32, u32) = (u32::MAX, u32::MAX);

/// `verts`/`indices` drawn by `draws`, with the wireframe edges per material `wire`, split into parts of at most
/// `max_verts` vertices and `max_indices` triangle indices (each at least a triangle's worth). A part may hold a few
/// vertices more than `max_verts` for the wireframe's edges crossing parts; callers leave headroom for them.
pub(crate) fn split(verts: &[Vertex], indices: &[u32], draws: &[Draw], wire: &[Vec<(u32, u32)>], max_verts: usize, max_indices: usize) -> Vec<MeshPart> {
    let (max_verts, max_indices) = (max_verts.max(3), max_indices.max(3));
    let mut parts: Vec<MeshPart> = Vec::new();
    // the last part holding each vertex, and its index there
    let mut slot = vec![NONE; verts.len()];
    let mut part = MeshPart::default();
    let mut local: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    for &(start, count, material) in draws {
        let mut first = part.indices.len() as u32;
        let tris = indices.get(start as usize..(start as usize).saturating_add(count as usize)).unwrap_or(&[]);
        for tri in tris.as_chunks::<3>().0 {
            if tri.iter().any(|&v| v as usize >= verts.len()) {
                continue;
            }
            let new = tri.iter().filter(|v| !local.contains_key(v)).count();
            if part.verts.len() + new > max_verts || part.indices.len() + 3 > max_indices {
                close(&mut part, first, material);
                parts.push(std::mem::take(&mut part));
                local.clear();
                first = 0;
            }
            let index = parts.len() as u32;
            for &v in tri {
                let at = *local.entry(v).or_insert_with(|| {
                    part.verts.push(verts[v as usize]);
                    (part.verts.len() - 1) as u32
                });
                slot[v as usize] = (index, at);
                part.indices.push(at);
            }
        }
        close(&mut part, first, material);
    }
    if !part.indices.is_empty() || parts.is_empty() {
        parts.push(part);
    }
    // the wireframe: each edge in the part holding its first end, the other end copied in when it is elsewhere
    for (material, edges) in wire.iter().enumerate() {
        let firsts: Vec<u32> = parts.iter().map(|p| p.lines.len() as u32).collect();
        for &(a, b) in edges {
            let (Some(&(pa, la)), Some(&(pb, lb))) = (slot.get(a as usize), slot.get(b as usize)) else { continue };
            if (pa, la) == NONE || (pb, lb) == NONE {
                continue;
            }
            let Some(p) = parts.get_mut(pa as usize) else { continue };
            let lb = if pa == pb {
                lb
            } else {
                p.verts.push(verts[b as usize]);
                (p.verts.len() - 1) as u32
            };
            p.lines.extend([la, lb]);
        }
        for (p, first) in parts.iter_mut().zip(firsts) {
            let count = p.lines.len() as u32 - first;
            if count > 0 {
                p.line_draws.push((first, count, material));
            }
        }
    }
    parts
}

/// Ends the part's draw of `material` begun at index `first`.
fn close(part: &mut MeshPart, first: u32, material: usize) {
    let count = part.indices.len() as u32 - first;
    if count > 0 {
        part.draws.push((first, count, material));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vert(i: u32) -> Vertex {
        Vertex { pos: [i as f32, 0.0, 0.0], nrm: [0.0, 0.0, 1.0], tan: [1.0, 0.0, 0.0, 1.0], uv: [0.0, 0.0] }
    }

    /// The triangles of all parts as positions, in order.
    fn triangles(parts: &[MeshPart]) -> Vec<[f32; 3]> {
        parts
            .iter()
            .flat_map(|p| p.draws.iter().flat_map(move |&(s, c, _)| p.indices[s as usize..(s + c) as usize].iter().map(|&i| p.verts[i as usize].pos[0])))
            .collect::<Vec<_>>()
            .chunks(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect()
    }

    #[test]
    fn a_mesh_over_the_limit_splits_into_parts_with_the_same_triangles() {
        // a strip of 10 triangles over 12 vertices, two materials
        let verts: Vec<Vertex> = (0..12).map(vert).collect();
        let indices: Vec<u32> = (0..10u32).flat_map(|i| [i, i + 1, i + 2]).collect();
        let draws = [(0, 18, 0), (18, 12, 1)];
        let wire = vec![vec![(0, 1), (1, 2), (5, 6)], vec![(9, 10), (2, 11)]];
        let whole = split(&verts, &indices, &draws, &wire, 100, 100);
        assert_eq!(whole.len(), 1);
        assert_eq!(whole[0].draws, vec![(0, 18, 0), (18, 12, 1)]);
        let parts = split(&verts, &indices, &draws, &wire, 5, 100);
        assert!(parts.len() > 1);
        assert!(parts.iter().all(|p| p.verts.len() <= 6), "within the limit but for the wireframe's copies");
        assert_eq!(triangles(&parts), triangles(&whole), "the same triangles in the same order");
        // every edge drawn once, between the same positions
        let edges = |ps: &[MeshPart]| {
            let mut e: Vec<(usize, [f32; 2])> = ps
                .iter()
                .flat_map(|p| {
                    p.line_draws.iter().flat_map(move |&(s, c, m)| {
                        p.lines[s as usize..(s + c) as usize].chunks(2).map(move |l| (m, [p.verts[l[0] as usize].pos[0], p.verts[l[1] as usize].pos[0]]))
                    })
                })
                .collect();
            e.sort_by(|a, b| a.partial_cmp(b).unwrap());
            e
        };
        assert_eq!(edges(&parts), edges(&whole));
        let parts = split(&verts, &indices, &draws, &wire, 100, 6);
        assert!(parts.iter().all(|p| p.indices.len() <= 6) && triangles(&parts) == triangles(&whole));
    }

    #[test]
    fn hostile_ranges_and_indices_are_skipped() {
        let verts: Vec<Vertex> = (0..3).map(vert).collect();
        let parts = split(&verts, &[0, 1, 2, 0, 1, 99], &[(0, 6, 0), (3, u32::MAX, 1)], &[vec![(0, 99)]], 0, 0);
        assert_eq!(triangles(&parts), vec![[0.0, 1.0, 2.0]]);
        assert!(parts.iter().all(|p| p.lines.is_empty()));
        assert_eq!(split(&[], &[], &[], &[], 10, 10).len(), 1);
    }
}
