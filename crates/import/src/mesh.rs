//! The file's scene geometry ([`SceneMesh`], decoded by the reader) as TextureCraft's [`Mesh`]: one submesh per placed
//! part, materials in slot order, UV set 0 (the one the file paints on), glTF UV convention on both sides. Parts
//! without normals or tangents get computed ones.

use glam::{Vec2, Vec3, Vec4};
use texturecraft_geom::{MAX_TRIANGLES, Mesh, SubMesh};
use texturecraft_import_reader::decode::assbin::SceneMesh;

/// `scene` as TextureCraft's mesh; an error when it holds no triangles or too many.
pub fn mesh_of(scene: &SceneMesh) -> Result<Mesh, String> {
    scene.validate()?;
    let triangles: usize = scene.parts.iter().map(|p| p.triangles.len()).sum();
    if triangles == 0 {
        return Err("the mesh has no triangles".into());
    }
    if triangles > MAX_TRIANGLES {
        return Err(format!("the mesh has {triangles} triangles (limit {MAX_TRIANGLES})"));
    }
    let mut mesh = Mesh { materials: scene.materials.clone(), ..Mesh::default() };
    let (mut need_normals, mut need_tangents) = (false, false);
    for part in &scene.parts {
        let base = u32::try_from(mesh.positions.len()).map_err(|_| "the mesh has too many vertices".to_string())?;
        let n = part.positions.len();
        mesh.positions.extend(part.positions.iter().map(|p| Vec3::from_array(*p)));
        match part.normals.len() == n {
            true => mesh.normals.extend(part.normals.iter().map(|v| Vec3::from_array(*v))),
            false => {
                need_normals = true;
                mesh.normals.extend(std::iter::repeat_n(Vec3::Y, n));
            }
        }
        match part.tangents.len() == n {
            true => mesh.tangents.extend(part.tangents.iter().map(|t| Vec4::from_array(*t))),
            false => {
                need_tangents = true;
                mesh.tangents.extend(std::iter::repeat_n(Vec4::new(1.0, 0.0, 0.0, 1.0), n));
            }
        }
        match part.uv_sets.first() {
            Some(uv) => mesh.uvs.extend(uv.iter().map(|v| Vec2::from_array(*v))),
            None => mesh.uvs.extend(std::iter::repeat_n(Vec2::ZERO, n)),
        }
        let start = u32::try_from(mesh.indices.len()).map_err(|_| "the mesh has too many triangles".to_string())?;
        for t in &part.triangles {
            mesh.indices.extend(t.iter().map(|i| base.saturating_add(*i)));
        }
        let count = u32::try_from(part.triangles.len() * 3).map_err(|_| "a part has too many triangles".to_string())?;
        mesh.submeshes.push(SubMesh { name: part.name.clone(), material: part.material, start, count });
    }
    if need_normals {
        mesh.compute_normals();
    }
    if need_tangents || need_normals {
        mesh.compute_tangents();
    }
    mesh.validate().map_err(|e| e.to_string())?;
    Ok(mesh)
}

/// Points the submeshes of slots the file collapsed into one texture set (`merged`) at that set's `slot`.
pub fn merge_slots(mesh: &mut Mesh, slot: usize, merged: &[usize]) {
    for s in mesh.submeshes.iter_mut().filter(|s| merged.contains(&s.material)) {
        s.material = slot;
    }
}

/// Numbers the mesh's materials in the order their triangles first appear, and points `set_materials` (each texture
/// set's material) along. A saved project stores the mesh as glTF, whose reader numbers materials that way (and leaves
/// out materials without triangles); numbered otherwise, a reopened project would show each set's layers on another
/// set's part. Triangle order is kept.
pub fn number_materials_by_appearance<'a>(mesh: &mut Mesh, set_materials: impl IntoIterator<Item = &'a mut usize>) {
    let mut order: Vec<usize> = Vec::new();
    for s in mesh.submeshes.iter().filter(|s| s.count >= 3) {
        if !order.contains(&s.material) {
            order.push(s.material);
        }
    }
    for m in 0..mesh.materials.len() {
        if !order.contains(&m) {
            order.push(m);
        }
    }
    let new_index = |old: usize| order.iter().position(|m| *m == old).unwrap_or(old);
    if order.iter().enumerate().all(|(i, m)| i == *m) {
        return;
    }
    for s in &mut mesh.submeshes {
        s.material = new_index(s.material);
    }
    for m in set_materials {
        *m = new_index(*m);
    }
    let names = std::mem::take(&mut mesh.materials);
    mesh.materials = order.iter().filter_map(|m| names.get(*m).cloned()).collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materials_are_numbered_as_a_reopened_project_numbers_them() {
        let scene = SceneMesh { materials: vec!["a".into(), "b".into(), "c".into()], parts: vec![part(2, true), part(0, true)] };
        let mut mesh = mesh_of(&scene).unwrap();
        let mut sets = [0usize, 1, 2];
        number_materials_by_appearance(&mut mesh, sets.iter_mut());
        // c's triangles come first, then a's; b has none
        assert_eq!(mesh.materials, vec!["c".to_string(), "a".to_string(), "b".to_string()]);
        assert_eq!(mesh.submeshes.iter().map(|s| s.material).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(sets, [1, 2, 0]);
        // already in order: unchanged
        let mut again = sets;
        number_materials_by_appearance(&mut mesh, again.iter_mut());
        assert_eq!(again, sets);
    }
    use texturecraft_import_reader::decode::assbin::ScenePart;

    fn part(material: usize, normals: bool) -> ScenePart {
        ScenePart {
            name: format!("part{material}"),
            material,
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: if normals { vec![[0.0, 0.0, 1.0]; 3] } else { Vec::new() },
            uv_sets: vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]],
            triangles: vec![[0, 1, 2]],
            ..ScenePart::default()
        }
    }

    #[test]
    fn parts_become_submeshes_and_missing_attributes_are_computed() {
        let scene = SceneMesh { materials: vec!["a".into(), "b".into()], parts: vec![part(0, true), part(1, false)] };
        let mut mesh = mesh_of(&scene).unwrap();
        assert_eq!((mesh.positions.len(), mesh.triangle_count(), mesh.submeshes.len()), (6, 2, 2));
        assert_eq!(mesh.indices[3..], [3, 4, 5]);
        assert_eq!(mesh.submeshes[1].name, "part1");
        assert!((mesh.normals[3].z.abs() - 1.0).abs() < 1e-5, "computed normal of a flat triangle");
        merge_slots(&mut mesh, 0, &[1]);
        assert!(mesh.submeshes.iter().all(|s| s.material == 0));
        assert_eq!(mesh.triangles_of(0).count(), 2);
    }

    #[test]
    fn broken_scenes_are_errors() {
        assert!(mesh_of(&SceneMesh::default()).is_err(), "no triangles");
        let bad = SceneMesh { materials: vec![], parts: vec![part(0, true)] };
        assert!(mesh_of(&bad).is_err(), "missing material slot");
        let mut p = part(0, true);
        p.triangles = vec![[0, 1, 9]];
        assert!(mesh_of(&SceneMesh { materials: vec!["a".into()], parts: vec![p] }).is_err());
    }
}
