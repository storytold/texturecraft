//! COLLADA (`.dae`) scene import through the shared bounded reader.

use texturecraft_geom::glam::{Mat3, Mat4, Vec2, Vec3};

use super::{Corner, PolygonSink, RawCamera, RawMesh};
use crate::{FormatError, Result, malformed};

const KIND: &str = "COLLADA";

fn limits() -> craft_mesh::Limits {
    craft_mesh::Limits {
        max_vertices: super::MAX_VERTICES,
        max_triangles: texturecraft_geom::MAX_TRIANGLES,
        max_face_corners: super::MAX_VERTICES,
        max_meshes: super::MAX_VERTICES,
        max_parts: super::MAX_VERTICES,
        max_materials: super::MAX_VERTICES,
        max_nodes: 1_000_000,
        max_depth: 256,
        max_data_bytes: super::MAX_MESH_FILE_BYTES as usize,
        max_extras: super::MAX_CAMERAS,
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
    crate::guard(KIND, || read_inner(bytes))
}

fn read_inner(bytes: &[u8]) -> Result<RawMesh> {
    let lim = limits();
    let cameras_doc = craft_mesh::dae::read_document_with_cameras(bytes, &lim).map_err(convert)?;
    let doc = cameras_doc.document();
    let to_mat = |m: craft_mesh::Mat4| Mat4::from_cols_array(&m.map(|v| v as f32));
    let global = to_mat(doc.global_transform());
    let scene = &doc.scene;

    let mut sink = PolygonSink::default();
    for w in &scene.warnings {
        sink.warn(w.clone());
    }
    let mut cameras = Vec::new();
    let mut stack: Vec<(usize, Mat4, usize)> = scene.roots.iter().rev().map(|&r| (r, global, 0)).collect();
    let mut visits = 0usize;
    while let Some((id, parent, depth)) = stack.pop() {
        visits += 1;
        if visits > lim.max_nodes || depth > lim.max_depth {
            return Err(FormatError::TooLarge("the COLLADA node hierarchy is too large, too deep or cyclic".into()));
        }
        let node = scene.nodes.get(id).ok_or_else(|| malformed(KIND, "node index out of range"))?;
        let world = parent * to_mat(node.transform);

        for cam in cameras_doc.cameras_for_node(id) {
            if cameras.len() >= super::MAX_CAMERAS {
                break;
            }
            let (fov, ortho) = match cam.projection {
                craft_mesh::Projection::Perspective { yfov, .. } => (yfov.to_degrees() as f32, None),
                craft_mesh::Projection::Orthographic { ymag, .. } => (40.0, Some(ymag as f32)),
            };
            cameras.push(RawCamera::placed(cam.name.clone(), world, Vec3::NEG_Z, fov, ortho));
        }

        if let Some(mesh_idx) = node.mesh {
            let mesh = scene.meshes.get(mesh_idx).ok_or_else(|| malformed(KIND, "mesh index out of range"))?;
            let normal_transform = Mat3::from_mat4(world).inverse().transpose();
            let flip = world.determinant() < 0.0;
            sink.part = node.name.clone();
            let uvs = mesh.uv_sets.first();
            for part in &mesh.parts {
                // An explicit empty name is a binding; only an absent material uses the default.
                let material = part.material.and_then(|m| scene.materials.get(m)).map_or("Default", |m| m.name.as_str());
                for face in part.faces() {
                    let mut corners = Vec::with_capacity(face.len());
                    for c in face {
                        let p = mesh.positions.get(c.position as usize).ok_or_else(|| malformed(KIND, "position index out of range"))?;
                        let p = world.transform_point3(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32));
                        let n = if c.normal == craft_mesh::NONE {
                            None
                        } else {
                            let n = mesh.normals.get(c.normal as usize).ok_or_else(|| malformed(KIND, "normal index out of range"))?;
                            Some(normal_transform * Vec3::new(n[0] as f32, n[1] as f32, n[2] as f32))
                        };
                        let uv = if c.uv == craft_mesh::NONE { None } else { uvs.and_then(|s| s.get(c.uv as usize)).map(|v| Vec2::new(v[0], v[1])) };
                        corners.push(Corner { p, n, uv, color: None });
                    }
                    if flip {
                        corners.reverse();
                    }
                    sink.polygon(material, &corners)?;
                }
            }
        }

        stack.extend(node.children.iter().rev().map(|&c| (c, world, depth + 1)));
    }

    let mut raw = sink.finish();
    raw.cameras = cameras;
    Ok(raw)
}
