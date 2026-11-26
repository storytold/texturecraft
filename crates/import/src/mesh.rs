//! The file's scene geometry ([`SceneMesh`], decoded by the reader) as TextureCraft's [`Mesh`]: one submesh per placed
//! part, materials in slot order, UV set 0 (the one the file paints on), glTF UV convention on both sides. Parts
//! without normals or tangents get computed ones.

use std::collections::BTreeSet;

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

/// Match the stored glTF reader: used materials appear in triangle order, with distinct nonempty names.
/// Only the mesh's internal names change; texture set labels come from the source records.
pub(crate) fn prepare_stored_materials<'a>(mesh: &mut Mesh, set_materials: impl IntoIterator<Item = &'a mut usize>) -> Result<(), String> {
    mesh.submeshes.retain(|part| part.count >= 3);
    let mut new_slots = vec![None; mesh.materials.len()];
    let mut order = Vec::new();
    for part in &mut mesh.submeshes {
        let original = part.material;
        let slot = new_slots.get_mut(original).ok_or_else(|| format!("mesh part {:?} has no material", part.name))?;
        part.material = *slot.get_or_insert_with(|| {
            let next = order.len();
            order.push(original);
            next
        });
    }
    for slot in set_materials {
        *slot = new_slots.get(*slot).copied().flatten().ok_or_else(|| format!("texture set refers to unused material slot {}", *slot + 1))?;
    }

    // Reserve every source name so a generated suffix cannot change another material's name.
    let mut taken: BTreeSet<String> = order.iter().map(|&slot| mesh.materials[slot].clone()).collect();
    let mut seen = BTreeSet::new();
    let names = std::mem::take(&mut mesh.materials);
    mesh.materials = order
        .into_iter()
        .map(|slot| {
            let name = &names[slot];
            if !name.is_empty() && seen.insert(name) {
                return name.clone();
            }
            let base = if name.is_empty() { "Material" } else { name };
            let mut unique = format!("{base} [{}]", slot + 1);
            while !taken.insert(unique.clone()) {
                unique.push('_');
            }
            unique
        })
        .collect();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materials_are_numbered_as_a_reopened_project_numbers_them() {
        let scene = SceneMesh { materials: vec!["a".into(), "b".into(), "c".into()], parts: vec![part(2, true), part(0, true)] };
        let mut mesh = mesh_of(&scene).unwrap();
        let mut sets = [0usize, 2];
        prepare_stored_materials(&mut mesh, sets.iter_mut()).unwrap();
        assert_eq!(mesh.materials, ["c", "a"]);
        assert_eq!(mesh.submeshes.iter().map(|s| s.material).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(sets, [1, 0]);
    }

    #[test]
    fn stored_material_names_keep_distinct_slots_and_are_stable() {
        let scene = SceneMesh {
            materials: ["Skin", "Skin", "Skin [2]", ""].into_iter().map(str::to_string).collect(),
            parts: (0..4).map(|slot| part(slot, true)).collect(),
        };
        let mut mesh = mesh_of(&scene).unwrap();
        let mut sets = [0, 1, 2, 3];
        let indices = mesh.indices.clone();
        prepare_stored_materials(&mut mesh, sets.iter_mut()).unwrap();

        assert_eq!(sets, [0, 1, 2, 3]);
        assert_eq!(mesh.indices, indices);
        assert_eq!(mesh.materials[0], "Skin");
        assert_eq!(mesh.materials[2], "Skin [2]");
        assert!(mesh.materials.iter().all(|name| !name.is_empty()));
        assert_eq!(mesh.materials.iter().collect::<BTreeSet<_>>().len(), sets.len());
        let names = mesh.materials.clone();
        prepare_stored_materials(&mut mesh, sets.iter_mut()).unwrap();
        assert_eq!(mesh.materials, names);
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
