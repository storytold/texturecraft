use std::sync::OnceLock;

use texturecraft_geom::{Mesh, TrianglePartition, UvSeam};

/// Seams and the UV island of each rasterized triangle, borrowing the existing texel-to-triangle grid
pub struct SeamPaddingInput<'a> {
    pub(crate) seams: Vec<UvSeam>,
    triangles: &'a [u32],
    material: Option<(&'a Mesh, usize)>,
    islands: OnceLock<TrianglePartition>,
}

impl<'a> SeamPaddingInput<'a> {
    /// Use seams and a partition from the same mesh and material as the rasterized triangles
    pub fn new(seams: Vec<UvSeam>, triangles: &'a [u32], islands: TrianglePartition) -> Self {
        Self { seams, triangles, material: None, islands: OnceLock::from(islands) }
    }

    /// Find seams now and build island ownership only if a padding texel can cross
    pub fn of_material(mesh: &'a Mesh, material: usize, triangles: &'a [u32]) -> Option<Self> {
        let seams = UvSeam::of_material(mesh, material);
        if seams.is_empty() {
            return None;
        }

        Some(Self { seams, triangles, material: Some((mesh, material)), islands: OnceLock::new() })
    }

    pub(crate) fn triangle_island(&self, triangle: u32) -> Option<usize> {
        let islands = self.islands.get().or_else(|| self.material.map(|(mesh, material)| self.islands.get_or_init(|| UvSeam::islands_of(mesh, material))))?;
        islands.index_of(triangle)
    }

    pub(crate) fn triangle_at(&self, texel: u32) -> Option<u32> {
        self.triangles.get(texel as usize).copied()
    }

    pub(crate) fn island_at(&self, texel: u32) -> Option<usize> {
        self.triangle_island(self.triangle_at(texel)?)
    }
}
