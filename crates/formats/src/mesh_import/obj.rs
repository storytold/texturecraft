//! Wavefront OBJ through the shared bounded mesh reader.
use super::{MAX_VERTICES, RawMesh, check_vertices};
use crate::{FormatError, Result, malformed};
use std::collections::HashMap;
use texturecraft_geom::glam::{Vec2, Vec3, Vec4};

const KIND: &str = "OBJ";

pub(crate) fn read(bytes: &[u8]) -> Result<RawMesh> {
    crate::guard(KIND, || read_inner(bytes))
}

fn read_inner(bytes: &[u8]) -> Result<RawMesh> {
    let limits = craft_mesh::Limits {
        max_vertices: MAX_VERTICES,
        max_triangles: texturecraft_geom::MAX_TRIANGLES,
        max_face_corners: MAX_VERTICES,
        max_meshes: MAX_VERTICES,
        max_parts: MAX_VERTICES,
        max_materials: MAX_VERTICES,
        max_data_bytes: super::MAX_MESH_FILE_BYTES as usize,
        ..Default::default()
    };
    let options = craft_mesh::obj::ReadOptions { strict: true, preserve_runs: true, ignore_material_libraries: true, groups_as_objects: true };
    let scene = craft_mesh::obj::read_with_options(bytes, &limits, &mut craft_mesh::no_external, &options).map_err(|e| match e {
        craft_mesh::Error::TooLarge(s) => FormatError::TooLarge(s),
        e => FormatError::Invalid(e.to_string()),
    })?;
    let any_uvs = scene.meshes.iter().flat_map(|mesh| mesh.parts.iter()).flat_map(|part| part.corners.iter()).any(|c| c.uv != craft_mesh::NONE);
    let any_colors = scene.meshes.iter().any(|mesh| !mesh.colors.is_empty());
    let mut raw = RawMesh::default();
    let (mut normals, mut uvs, mut colors) = (Vec::new(), Vec::new(), Vec::new());
    let (mut missing_normals, mut missing_uvs) = (false, false);
    for (mesh_id, mesh) in scene.meshes.iter().enumerate() {
        // The shared reader preserves declarations as meshes, even when their names match
        let declaration = mesh_id.to_string();
        for part in &mesh.parts {
            let material = part.material.and_then(|i| scene.materials.get(i)).map_or("Default", |m| m.name.as_str());
            // OBJ model/material runs have independent v/vt/vn deduplication.
            let mut ids = HashMap::new();
            for face in part.faces() {
                let mut polygon = Vec::with_capacity(face.len());
                for corner in face {
                    let id = if let Some(id) = ids.get(corner) {
                        *id
                    } else {
                        let position = mesh.positions.get(corner.position as usize).ok_or_else(|| malformed(KIND, "position index out of range"))?;
                        check_vertices(raw.positions.len().saturating_add(1))?;
                        // Bound both the output pools and the per-run corner map before growth.
                        let staging = raw
                            .positions
                            .len()
                            .checked_add(1)
                            .and_then(|n| n.checked_mul(64))
                            .and_then(|n| ids.len().checked_add(1).and_then(|m| m.checked_mul(48)).and_then(|m| n.checked_add(m)));
                        if staging.is_none_or(|n| n > limits.max_data_bytes) {
                            return Err(FormatError::TooLarge("OBJ mesh exceeds the memory limit".into()));
                        }
                        let id = u32::try_from(raw.positions.len()).map_err(|_| FormatError::TooLarge("OBJ vertex count".into()))?;
                        raw.positions.push(Vec3::new(position[0] as f32, position[1] as f32, position[2] as f32));
                        let normal = mesh.normals.get(corner.normal as usize).map(|n| Vec3::new(n[0] as f32, n[1] as f32, n[2] as f32));
                        missing_normals |= normal.is_none();
                        normals.push(normal.unwrap_or_default());
                        if any_uvs {
                            let uv = mesh.uv_sets.first().and_then(|set| set.get(corner.uv as usize)).map(|v| Vec2::new(v[0], v[1]));
                            missing_uvs |= uv.is_none();
                            uvs.push(uv.unwrap_or_default());
                        }
                        if any_colors {
                            colors.push(mesh.colors.get(corner.position as usize).copied().map(Vec4::from_array).unwrap_or(Vec4::ONE));
                        }
                        ids.insert(*corner, id);
                        id
                    };
                    polygon.push(id);
                }
                raw.polygon(material, &declaration, &polygon);
            }
        }
    }
    // Restore display names only after grouping by declaration and material
    for (_, name, _) in &mut raw.groups {
        if let Some(mesh) = name.parse::<usize>().ok().and_then(|id| scene.meshes.get(id)) {
            *name = mesh.name.clone();
        }
    }
    if missing_uvs {
        raw.warnings.push("some objects have no UVs; default UVs were assigned".into());
    }
    raw.normals = (!missing_normals).then_some(normals);
    raw.uvs = any_uvs.then_some(uvs);
    raw.colors = any_colors.then_some(colors);
    Ok(raw)
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "release benchmark for declaration-heavy OBJ input"]
    fn declaration_import_benchmark() {
        for count in [10_000, 50_000] {
            let mut source = String::from("v 0 0 0\nv 1 0 0\nv 0 1 0\n");
            for _ in 0..count {
                source.push_str("o repeated\nf 1 2 3\n");
            }
            let start = std::time::Instant::now();
            let raw = super::read(source.as_bytes()).unwrap();
            println!("{count} declarations: {:?}", start.elapsed());
            assert_eq!(raw.groups.len(), count);
            assert_eq!(raw.triangle_count(), count);
            assert!(raw.groups.iter().all(|(_, name, tris)| name == "repeated" && tris.len() == 3));
        }
    }
}
