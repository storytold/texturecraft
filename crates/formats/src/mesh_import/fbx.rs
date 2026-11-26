//! FBX scene import through the shared bounded reader.
use super::{Corner, MAX_MESH_FILE_BYTES, MAX_TEXTURE_BYTES, MAX_ZIP_ENTRIES, PolygonSink, RawBaseColor, RawCamera, RawMesh};
use crate::{FormatError, Result, malformed};
use texturecraft_geom::glam::{Mat3, Mat4, Vec2, Vec3, Vec4};
const KIND: &str = "FBX";
type Files<'a> = dyn FnMut(&[&str]) -> Option<Vec<u8>> + 'a;
fn limits() -> craft_mesh::Limits {
    craft_mesh::Limits {
        max_vertices: super::MAX_VERTICES,
        max_triangles: texturecraft_geom::MAX_TRIANGLES,
        max_face_corners: super::MAX_VERTICES,
        max_meshes: super::MAX_VERTICES,
        max_parts: super::MAX_VERTICES,
        max_materials: super::MAX_VERTICES,
        max_nodes: super::MAX_VERTICES,
        max_data_bytes: 1 << 31,
        max_extras: 100_000,
        ..Default::default()
    }
}

fn package_limits() -> craft_mesh::fbx::package::PackageLimits {
    craft_mesh::fbx::package::PackageLimits {
        max_entries: MAX_ZIP_ENTRIES as u64,
        max_model_bytes: MAX_MESH_FILE_BYTES,
        max_texture_bytes: MAX_TEXTURE_BYTES,
        max_total_bytes: MAX_MESH_FILE_BYTES,
    }
}

fn convert(e: craft_mesh::Error) -> FormatError {
    match e {
        craft_mesh::Error::TooLarge(s) => FormatError::TooLarge(s),
        craft_mesh::Error::Unsupported(s) => FormatError::Unsupported(s),
        e => FormatError::Invalid(e.to_string()),
    }
}

pub(crate) fn read(bytes: &[u8]) -> Result<RawMesh> {
    crate::guard(KIND, || {
        if !craft_mesh::fbx::package::is_package(bytes) {
            return read_inner(bytes, &mut |_| None);
        }
        let mut package = craft_mesh::fbx::package::Package::open(bytes, package_limits()).map_err(convert)?;
        let fbx = package.model().map_err(convert)?;
        read_inner(&fbx, &mut |paths| package.texture(paths))
    })
}

pub(crate) fn package(file_name: &str, bytes: &[u8], found: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Result<(Option<Vec<u8>>, Vec<String>)> {
    if craft_mesh::fbx::package::is_package(bytes) {
        return Ok((None, Vec::new()));
    }
    crate::guard(KIND, || {
        let limits = limits();
        let doc = craft_mesh::fbx::read_document_strict(bytes, &limits).map_err(convert)?;
        if doc.version == 0 {
            return Err(malformed(KIND, "not an FBX file (no FBXHeaderExtension / FBXVersion)"));
        }
        doc.scene_with_options(
            &limits,
            &mut craft_mesh::no_external,
            &craft_mesh::fbx::SceneOptions { apply_inactive_rotation: true, strict_geometry: true, prefer_last_parent: true, only_object_connections: true },
        )
        .map_err(convert)?;
        craft_mesh::fbx::package::write(&doc, file_name, bytes, &limits, package_limits(), found).map_err(convert)
    })
}

fn read_inner(bytes: &[u8], files: &mut Files) -> Result<RawMesh> {
    let limits = limits();
    let doc = craft_mesh::fbx::read_document_strict(bytes, &limits).map_err(convert)?;
    if doc.version == 0 {
        return Err(malformed(KIND, "not an FBX file (no FBXHeaderExtension / FBXVersion)"));
    }
    let colors = doc.diffuse_materials(&limits, files).map_err(convert)?;
    let (scene, placements) = doc
        .scene_with_placements(
            &limits,
            &mut craft_mesh::no_external,
            &craft_mesh::fbx::SceneOptions { apply_inactive_rotation: true, strict_geometry: true, prefer_last_parent: true, only_object_connections: true },
        )
        .map_err(convert)?;
    let to_mat = |m: craft_mesh::Mat4| Mat4::from_cols_array(&m.map(|v| v as f32));
    let global = to_mat(doc.axes_to_y_up()) * Mat4::from_scale(Vec3::splat((doc.unit_scale().unwrap_or(1.0) / 100.0) as f32));
    let mut sink = PolygonSink::default();
    let mut cameras = Vec::new();
    cameras.resize_with(scene.cameras.len().min(super::MAX_CAMERAS), || None);
    // Bound the stored world matrices and pending node indices together: neither depends on
    // copying model names, even when a hierarchy fans out widely.
    let placement_bytes =
        scene.nodes.len().checked_mul(std::mem::size_of::<Option<Mat4>>()).ok_or_else(|| FormatError::TooLarge("FBX hierarchy size overflow".into()))?;
    let remaining = limits.max_data_bytes.checked_sub(placement_bytes).ok_or_else(|| FormatError::TooLarge("FBX hierarchy exceeds its memory limit".into()))?;
    let stack_limit = remaining / std::mem::size_of::<(usize, Mat4, usize)>();
    if scene.roots.len() > stack_limit {
        return Err(FormatError::TooLarge("FBX hierarchy exceeds its memory limit".into()));
    }
    let mut stack: Vec<_> = scene.roots.iter().rev().map(|i| (*i, global, 0usize)).collect();
    let mut worlds = vec![None; scene.nodes.len()];
    let mut visited = 0usize;
    let mut points = 0usize;
    while let Some((id, parent, depth)) = stack.pop() {
        visited += 1;
        if visited > limits.max_nodes || depth > limits.max_depth {
            return Err(FormatError::TooLarge("FBX hierarchy exceeds its traversal limit".into()));
        }
        let node = scene.nodes.get(id).ok_or_else(|| malformed(KIND, "node index out of range"))?;
        let world = parent * to_mat(node.transform);
        if let Some(slot) = worlds.get_mut(id) {
            *slot = Some(world);
        }
        for child in node.children.iter().rev() {
            if stack.len() >= stack_limit {
                return Err(FormatError::TooLarge("FBX hierarchy exceeds its memory limit".into()));
            }
            stack.push((*child, world, depth + 1));
        }
        if let Some(index) = node.camera
            && let (Some(camera), Some(slot)) = (scene.cameras.get(index), cameras.get_mut(index))
        {
            let (fov, ortho) = match camera.projection {
                craft_mesh::Projection::Perspective { yfov, .. } => (yfov.to_degrees() as f32, None),
                craft_mesh::Projection::Orthographic { ymag, .. } => (40.0, Some(ymag as f32)),
            };
            *slot = Some(RawCamera::placed(camera.name.clone(), world, Vec3::NEG_Z, fov, ortho));
        }
    }
    for placement in placements {
        let id = placement.node;
        let node = scene.nodes.get(id).ok_or_else(|| malformed(KIND, "placement index out of range"))?;
        let world = worlds.get(id).copied().flatten().ok_or_else(|| malformed(KIND, "cycle in model hierarchy"))?;
        let Some(index) = node.mesh else { continue };
        let mesh = scene.meshes.get(index).ok_or_else(|| malformed(KIND, "mesh index out of range"))?;
        points = points.checked_add(mesh.positions.len()).ok_or_else(|| FormatError::TooLarge("FBX instance work overflow".into()))?;
        if points > 200_000_000 {
            return Err(FormatError::TooLarge("FBX instances exceed the 200000000 control-point work limit".into()));
        }
        let normal_transform = Mat3::from_mat4(world).inverse().transpose();
        let flip = world.determinant() < 0.0;
        sink.part = placement.model.and_then(|i| scene.nodes.get(i)).map_or_else(String::new, |n| n.name.clone());
        let (mut bad_normals, mut bad_uvs) = (0usize, 0usize);
        for group in &mesh.parts {
            let material = group.material.and_then(|m| scene.materials.get(m)).map_or("Default", |m| m.name.as_str());
            for face in group.faces() {
                let mut corners = Vec::with_capacity(face.len());
                for c in face {
                    let p = mesh.positions.get(c.position as usize).ok_or_else(|| malformed(KIND, "position index out of range"))?;
                    let p = world.transform_point3(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32));
                    let n = mesh.normals.get(c.normal as usize).map(|n| normal_transform * Vec3::new(n[0] as f32, n[1] as f32, n[2] as f32));
                    if !mesh.normals.is_empty() && n.is_none() {
                        bad_normals += 1;
                    }
                    let uv = mesh.uv_sets.first().map(|set| match set.get(c.uv as usize) {
                        Some(v) => Vec2::new(v[0], v[1]),
                        None => {
                            bad_uvs += 1;
                            Vec2::ZERO
                        }
                    });
                    let color = mesh.colors.get(c.position as usize).copied().map(Vec4::from_array);
                    corners.push(Corner { p, n, uv, color });
                }
                if flip {
                    corners.reverse();
                }
                sink.polygon(material, &corners)?;
            }
        }
        if bad_normals > 0 {
            sink.warn(format!("{bad_normals} polygon corners have no normal in the FBX normal layer; normals were recomputed"));
        }
        if bad_uvs > 0 {
            sink.warn(format!("{bad_uvs} polygon corners have no UV in the FBX UV layer (set to 0, 0)"));
        }
    }
    for warning in scene.warnings {
        sink.warn(warning);
    }
    let mut raw = sink.finish();
    raw.cameras = cameras.into_iter().flatten().collect();
    raw.base_colors = colors.into_iter().map(|c| RawBaseColor { material: c.name, factor: Vec4::from_array(c.factor), image: c.image }).collect();
    Ok(raw)
}

#[cfg(test)]
mod tests {
    #[test]
    fn incomplete_uv_layer_keeps_other_uvs() {
        let bytes = br#"FBXHeaderExtension: { FBXVersion: 7400 }
Objects: {
 Geometry: 1, "Geometry::G", "Mesh" {
  Vertices: 0,0,0,1,0,0,0,1,0
  PolygonVertexIndex: 0,1,-3
  LayerElementUV: 0 {
   MappingInformationType: "ByPolygonVertex"
   ReferenceInformationType: "IndexToDirect"
   UV: 0.25,0.5, 0.75,0.5
   UVIndex: 0,1
  }
 }
 Model: 2, "Model::M", "Mesh" {}
}
Connections: { C: "OO",1,2 }
"#;
        let raw = super::read(bytes).unwrap();
        let uvs = raw.uvs.unwrap();
        assert!(uvs.contains(&texturecraft_geom::glam::Vec2::new(0.25, 0.5)));
        assert!(uvs.contains(&texturecraft_geom::glam::Vec2::ZERO));
        assert!(raw.warnings.iter().any(|w| w.contains("1 polygon corners have no UV")));
    }
}
