//! STL (binary and ASCII), read by the shared bounded mesh importer.

use texturecraft_geom::glam::{Vec3, Vec4};

use super::{RawMesh, check_triangles};
use crate::{FormatError, Result};

const KIND: &str = "STL";

pub(crate) fn read(bytes: &[u8]) -> Result<RawMesh> {
    let scene = craft_mesh::stl::read(bytes, &craft_mesh::Limits::default()).map_err(|e| FormatError::Invalid(e.to_string()))?;
    let mut raw = RawMesh::default();
    raw.warnings = scene.warnings;

    let mut tri_count = 0;
    let mut vertex_colors = Vec::new();
    let has_colors = !scene.materials.is_empty();
    for mesh in scene.meshes {
        // The shared model groups colored facets into parts. Position indices still reflect
        // source order, so merge the parts back into STL order for TextureCraft's raw mesh.
        let mut triangles = Vec::new();
        for part in mesh.parts {
            let material = part.material.and_then(|i| scene.materials.get(i));
            let color = material.map(|m| Vec4::from_array(m.base_color));
            for tri in part.triangles() {
                let first_position = tri.iter().map(|c| c.position).min().unwrap_or(u32::MAX);
                triangles.push((first_position, tri, color));
            }
        }
        triangles.sort_by_key(|(first_position, _, _)| *first_position);
        for (_, tri, color) in triangles {
            check_triangles(tri_count, 1)?;
            tri_count += 1;
            let mut points = [Vec3::ZERO; 3];
            for (i, corner) in tri.into_iter().enumerate() {
                let p = mesh.positions.get(corner.position as usize).ok_or_else(|| FormatError::Invalid(format!("{KIND}: position index out of range")))?;
                points[i] = Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32);
            }
            // Recompute per-facet normals as TextureCraft's old importer did; stored STL
            // normals are often zero or disagree with the vertex winding.
            let normal = (points[1] - points[0]).cross(points[2] - points[0]).try_normalize().unwrap_or(Vec3::Y);
            raw.positions.extend_from_slice(&points);
            raw.normals.get_or_insert_with(Vec::new).extend_from_slice(&[normal; 3]);
            if has_colors {
                // The shared reader normalizes STL's color extensions to linear RGBA.
                // Uncolored facets alongside colored ones receive its neutral gray material.
                let color = color.unwrap_or(Vec4::new(0.6, 0.6, 0.6, 1.0));
                vertex_colors.extend_from_slice(&[color; 3]);
            }
        }
    }
    if raw.positions.is_empty() {
        return Err(crate::malformed(KIND, "no facets"));
    }
    if has_colors {
        raw.colors = Some(vertex_colors);
    }
    raw.groups.push(("Default".into(), String::new(), (0..raw.positions.len() as u32).collect()));
    Ok(raw)
}
