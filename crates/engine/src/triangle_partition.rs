//! Triangles of one material grouped into the pieces polygon fill selects and fills.

use std::collections::HashMap;
use std::hash::Hash;

use texturecraft_geom::Mesh;

/// A material's triangles split into groups joined across shared edges
pub struct TrianglePartition {
    /// The triangles of each group
    pub groups: Vec<Vec<u32>>,
    /// The index of each triangle's group
    group_of: HashMap<u32, usize>,
}

impl TrianglePartition {
    /// Group `material`'s triangles of `mesh`: two triangles join when `edges` names a key for both
    pub fn new<K: Eq + Hash + Clone>(mesh: &Mesh, material: usize, edges: impl Fn(&Mesh, usize) -> Vec<K>) -> TrianglePartition {
        // Each triangle's edges once, in mesh order so the groups come out the same every time
        let triangles: Vec<u32> = mesh.triangles_of(material).map(|triangle| triangle as u32).collect();
        let position_of: HashMap<u32, usize> = triangles.iter().enumerate().map(|(position, triangle)| (*triangle, position)).collect();
        let mut edge_lists: Vec<Vec<K>> = triangles.iter().map(|triangle| edges(mesh, *triangle as usize)).collect();
        let mut by_edge: HashMap<K, Vec<u32>> = HashMap::new();
        for (triangle, triangle_edges) in triangles.iter().zip(&edge_lists) {
            for edge in triangle_edges {
                by_edge.entry(edge.clone()).or_default().push(*triangle);
            }
        }

        let mut groups = Vec::new();
        let mut group_of = HashMap::with_capacity(triangles.len());
        for &start in &triangles {
            if group_of.contains_key(&start) {
                continue;
            }

            let mut members = Vec::new();
            let mut stack = vec![start];
            group_of.insert(start, groups.len());
            while let Some(triangle) = stack.pop() {
                members.push(triangle);
                let triangle_edges = position_of.get(&triangle).and_then(|position| edge_lists.get_mut(*position)).map(std::mem::take).unwrap_or_default();
                for neighbour in triangle_edges.iter().flat_map(|edge| by_edge.get(edge).into_iter().flatten()) {
                    if !group_of.contains_key(neighbour) {
                        group_of.insert(*neighbour, groups.len());
                        stack.push(*neighbour);
                    }
                }
            }
            groups.push(members);
        }
        TrianglePartition { groups, group_of }
    }

    /// The index in [`TrianglePartition::groups`] of the group holding `triangle` (None for a triangle of another material)
    pub fn index_of(&self, triangle: u32) -> Option<usize> {
        self.group_of.get(&triangle).copied()
    }

    /// The group holding `triangle` (None for a triangle of another material)
    pub fn group(&self, triangle: u32) -> Option<&[u32]> {
        self.index_of(triangle).and_then(|group| self.groups.get(group)).map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use texturecraft_geom::primitives;

    use super::*;

    #[test]
    fn triangles_sharing_a_key_join_one_group() {
        let mesh = primitives::plane(2.0, 2);
        let singles = TrianglePartition::new(&mesh, 0, |_, _| Vec::<u32>::new());
        let pairs = TrianglePartition::new(&mesh, 0, |_, triangle| vec![triangle / 2]);

        assert_eq!(singles.groups.len(), mesh.triangle_count());
        assert_eq!(pairs.groups.len(), mesh.triangle_count() / 2);
        assert_eq!(pairs.group(3).map(<[u32]>::len), Some(2));
        assert_eq!(pairs.group(99), None);
    }
}
