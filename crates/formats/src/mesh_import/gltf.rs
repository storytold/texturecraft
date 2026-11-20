//! glTF 2.0 / GLB: the default scene's node tree, flattened to world space.

use std::collections::HashMap;

use texturecraft_geom::glam::{Mat3, Mat4, Vec2, Vec3, Vec4};

use super::{RawMesh, check_triangles, check_vertices};
use crate::{FormatError, Result, malformed};

const KIND: &str = "glTF";
/// Deepest node hierarchy followed (also stops cyclic node graphs).
const MAX_DEPTH: usize = 64;
/// Node instances visited (bounds exponential instancing in shared node graphs).
const MAX_NODE_VISITS: usize = 1_000_000;

/// A glTF or GLB from its bytes alone: buffers and images in other files must already be folded
/// in as data URIs ([`super::self_contained_mesh`]).
pub(crate) fn read(bytes: &[u8]) -> Result<RawMesh> {
    crate::guard(KIND, || read_inner(bytes))
}

fn limits() -> craft_mesh::Limits {
    craft_mesh::Limits {
        max_vertices: super::MAX_VERTICES,
        max_triangles: texturecraft_geom::MAX_TRIANGLES,
        max_meshes: super::MAX_VERTICES,
        max_parts: super::MAX_VERTICES,
        max_materials: super::MAX_VERTICES,
        max_depth: MAX_DEPTH,
        max_nodes: MAX_NODE_VISITS,
        max_data_bytes: super::MAX_MESH_FILE_BYTES as usize,
        ..craft_mesh::Limits::default()
    }
}

fn convert_error(e: craft_mesh::Error) -> FormatError {
    match e {
        craft_mesh::Error::Missing(_) => FormatError::Unsupported(
            "this glTF's buffers are in files that were not found next to it; keep its .bin files beside the .gltf, or use a .glb".into(),
        ),
        craft_mesh::Error::TooLarge(s) => FormatError::TooLarge(s),
        craft_mesh::Error::Unsupported(s) => FormatError::Unsupported(s),
        craft_mesh::Error::Malformed(s) => malformed(KIND, s),
    }
}

fn read_inner(bytes: &[u8]) -> Result<RawMesh> {
    let scene = craft_mesh::gltf::read(bytes, &limits(), &mut craft_mesh::no_external).map_err(convert_error)?;
    let mut raw = RawMesh { normals: Some(Vec::new()), warnings: scene.warnings.clone(), ..RawMesh::default() };
    let mut attrs = Attrs::default();
    let mut stack: Vec<(usize, Mat4, usize)> = scene.roots.iter().rev().map(|&r| (r, Mat4::IDENTITY, 0)).collect();
    let mut visits = 0usize;
    while let Some((index, parent, depth)) = stack.pop() {
        visits += 1;
        if visits > MAX_NODE_VISITS {
            return Err(FormatError::TooLarge(format!("the glTF scene has more than {MAX_NODE_VISITS} node instances")));
        }
        let Some(node) = scene.nodes.get(index) else {
            return Err(malformed(KIND, format!("node {index} does not exist")));
        };
        let m = parent * Mat4::from_cols_array(&node.transform.map(|v| v as f32));
        if let Some(cam_idx) = node.camera
            && let Some(cam) = scene.cameras.get(cam_idx)
            && raw.cameras.len() < super::MAX_CAMERAS
        {
            let (fov, ortho_half_height) = match cam.projection {
                craft_mesh::Projection::Perspective { yfov, .. } => (yfov.to_degrees() as f32, None),
                craft_mesh::Projection::Orthographic { ymag, .. } => (40.0, Some(ymag as f32)),
            };
            let name = if !cam.name.is_empty() {
                cam.name.clone()
            } else if !node.name.is_empty() {
                node.name.clone()
            } else {
                format!("Camera {}", raw.cameras.len() + 1)
            };
            raw.cameras.push(super::RawCamera::placed(name, m, Vec3::NEG_Z, fov, ortho_half_height));
        }
        if let Some(mesh_idx) = node.mesh
            && let Some(mesh) = scene.meshes.get(mesh_idx)
        {
            let part = if !node.name.is_empty() { node.name.as_str() } else { mesh.name.as_str() };
            for p in &mesh.parts {
                add_part(&mut raw, &mut attrs, &scene, mesh, p, m, part)?;
            }
        }
        if depth >= MAX_DEPTH {
            return Err(malformed(KIND, "the node hierarchy is nested too deeply (or cyclic)"));
        }
        stack.extend(node.children.iter().rev().map(|&c| (c, m, depth + 1)));
    }
    match attrs.uv_seen {
        (_, false) => raw.uvs = Some(attrs.uvs),
        (true, true) => raw.warnings.push("some primitives have no UVs; the whole mesh was unwrapped automatically".into()),
        (false, true) => {}
    }
    raw.colors = attrs.color_seen.then_some(attrs.colors);
    Ok(raw)
}

/// Per-vertex attributes gathered across primitives.
#[derive(Default)]
struct Attrs {
    uvs: Vec<Vec2>,
    /// (some primitive had UVs, some primitive had none)
    uv_seen: (bool, bool),
    /// White where a primitive has no COLOR_0.
    colors: Vec<Vec4>,
    color_seen: bool,
}

fn add_part(
    raw: &mut RawMesh,
    attrs: &mut Attrs,
    scene: &craft_mesh::Scene,
    mesh: &craft_mesh::Mesh,
    part: &craft_mesh::Part,
    transform: Mat4,
    part_name: &str,
) -> Result<()> {
    let tris = part.corners.as_chunks::<3>().0;
    if tris.is_empty() {
        return Ok(());
    }
    check_triangles(raw.triangle_count(), tris.len())?;

    let linear_transform = Mat3::from_mat4(transform);
    let normal_transform = linear_transform.inverse().transpose();
    let flip = linear_transform.determinant() < 0.0;

    let mat_name = match part.material {
        None => "Default".to_string(),
        Some(i) => scene.materials.get(i).filter(|m| !m.name.is_empty()).map(|m| m.name.clone()).unwrap_or_else(|| format!("Material_{i}")),
    };
    if let Some(i) = part.material
        && !raw.base_colors.iter().any(|c| c.material == mat_name)
        && let Some(mat) = scene.materials.get(i)
    {
        base_color(raw, scene, mat, &mat_name);
    }

    let uv_set = mesh.uv_sets.first();
    let has_uvs = part.corners.iter().all(|c| c.uv != craft_mesh::NONE && uv_set.is_some_and(|s| (c.uv as usize) < s.len()));
    if has_uvs {
        attrs.uv_seen.0 = true;
    } else {
        attrs.uv_seen.1 = true;
    }

    let has_normals = part.corners.iter().all(|c| c.normal != craft_mesh::NONE && (c.normal as usize) < mesh.normals.len());
    let has_colors = !mesh.colors.is_empty();
    attrs.color_seen |= has_colors;

    let pos_of = |i: u32| -> Vec3 {
        let p = mesh.positions.get(i as usize).copied().unwrap_or_default();
        transform.transform_point3(Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32))
    };
    let uv_of = |i: u32| -> Vec2 {
        if i == craft_mesh::NONE { Vec2::ZERO } else { uv_set.and_then(|s| s.get(i as usize)).copied().map(Vec2::from).unwrap_or_default() }
    };
    let color_of = |i: u32| -> Vec4 { if has_colors { mesh.colors.get(i as usize).copied().map(Vec4::from).unwrap_or(Vec4::ONE) } else { Vec4::ONE } };

    let raw_normals = raw.normals.get_or_insert_with(Vec::new);
    if has_normals {
        let mut remap: HashMap<(u32, u32, u32), u32> = HashMap::new();
        let mut out_tris = Vec::with_capacity(part.corners.len());
        for t in tris {
            let order = if flip { [t[0], t[2], t[1]] } else { [t[0], t[1], t[2]] };
            for c in order {
                let id = match remap.entry((c.position, c.normal, c.uv)) {
                    std::collections::hash_map::Entry::Occupied(e) => *e.get(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let id = raw.positions.len();
                        check_vertices(id + 1)?;
                        let n = mesh.normals.get(c.normal as usize).copied().unwrap_or([0.0, 1.0, 0.0]);
                        let n = (normal_transform * Vec3::new(n[0] as f32, n[1] as f32, n[2] as f32)).try_normalize().unwrap_or(Vec3::Y);
                        raw.positions.push(pos_of(c.position));
                        raw_normals.push(n);
                        attrs.uvs.push(uv_of(c.uv));
                        attrs.colors.push(color_of(c.position));
                        *e.insert(id as u32)
                    }
                };
                out_tris.push(id);
            }
        }
        raw.group(&mat_name, part_name).extend_from_slice(&out_tris);
    } else {
        // glTF: flat normals when none are given, so every triangle gets its own corners
        check_vertices(raw.positions.len().saturating_add(tris.len() * 3))?;
        let mut out_tris = Vec::with_capacity(part.corners.len());
        for t in tris {
            let order = if flip { [t[0], t[2], t[1]] } else { [t[0], t[1], t[2]] };
            let p = order.map(|c| pos_of(c.position));
            let face = (p[1] - p[0]).cross(p[2] - p[0]).try_normalize().unwrap_or(Vec3::Y);
            for (k, c) in order.iter().enumerate() {
                out_tris.push(raw.positions.len() as u32);
                raw.positions.push(p[k]);
                raw_normals.push(face);
                attrs.uvs.push(uv_of(c.uv));
                attrs.colors.push(color_of(c.position));
            }
        }
        raw.group(&mat_name, part_name).extend_from_slice(&out_tris);
    }

    Ok(())
}

/// Record `material`'s base color factor and embedded texture unless it is plain white.
fn base_color(raw: &mut RawMesh, scene: &craft_mesh::Scene, material: &craft_mesh::Material, name: &str) {
    let factor = Vec4::from(material.base_color);
    let image = material.base_color_texture.and_then(|info| {
        if info.uv_set != 0 {
            raw.warnings.push(format!("the base color texture of {name} uses UV set {}; only the first is read", info.uv_set));
            return None;
        }
        scene.images.get(info.image).and_then(|im| im.data.clone())
    });
    if image.is_some() || factor != Vec4::ONE {
        raw.base_colors.push(super::RawBaseColor { material: name.to_string(), factor, image });
    }
}

pub(crate) fn percent_decode(s: &str) -> Option<String> {
    craft_mesh::gltf::uri_decode_strict(s)
}

#[cfg(test)]
mod uri_decode_tests {
    use super::percent_decode;

    #[test]
    fn strict_shared_uri_decoder_preserves_reference_policy() {
        assert_eq!(percent_decode("texture%20é%2B.png"), Some("texture é+.png".into()));
        assert_eq!(percent_decode("a+b%GG%2"), Some("a+b%GG%2".into()));
        assert_eq!(percent_decode("%252e%252e"), Some("%2e%2e".into()));
        // Decoding does not replace the external-reference confinement checks.
        assert_eq!(percent_decode("%2e%2e%2fsecret"), Some("../secret".into()));
        assert_eq!(percent_decode("%00"), Some("\0".into()));
        for uri in ["%FF", "%C0%AF", "%ED%A0%80", "%E2%82"] {
            assert_eq!(percent_decode(uri), None, "{uri}");
        }
    }
}
