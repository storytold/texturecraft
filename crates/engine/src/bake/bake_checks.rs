//! What can spoil a texture set's bake, checked before it runs: overlapping UVs and parts "By Mesh Name" leaves unpaired

use std::collections::{BTreeMap, BTreeSet};

use texturecraft_bake::NamePair;
use texturecraft_core::MeshMatchMode;
use texturecraft_geom::{Mesh, SubMesh, UvGBuffer, glam::Vec2};

use crate::{Result, Session};

/// Side of the UV raster the overlap check counts texels on, per UV tile (as mesh import does)
const OVERLAP_CHECK_RESOLUTION: u32 = 512;

/// The only material of the UV-only mesh the overlap check builds per UV tile
const BLOCK_MATERIAL: usize = 0;

impl Session {
    /// About the share (0..1) of texture set `texture_set`'s UV layout the bake sees covered more than once, over the UV tiles it bakes: a texel under three triangles counts twice
    pub fn bake_uv_overlap(&self, texture_set: usize) -> Result<f32> {
        let set = self.doc.project.texture_set(texture_set)?;
        let mesh = self.bake_mesh(texture_set)?;
        let (overlaps, covered) = match set.tile_grid() {
            Some(grid) => {
                let baked_blocks: BTreeSet<(u32, u32)> =
                    set.uv_tiles.iter().filter(|tile| !set.bake_excluded_tiles.contains(tile)).filter_map(|tile| grid.block(*tile)).collect();
                let blocks = triangles_by_block(&mesh, set.material, (grid.cols, grid.rows), &baked_blocks);
                blocks
                    .values()
                    .map(|block| overlap_counts(block, BLOCK_MATERIAL))
                    .fold((0, 0), |(overlaps, covered), (block_overlaps, block_covered)| (overlaps + block_overlaps, covered + block_covered))
            }
            None => overlap_counts(&mesh, set.material),
        };

        Ok(if covered == 0 { 0.0 } else { (overlaps as f32 / covered as f32).min(1.0) })
    }

    /// How texture set `texture_set`'s low-poly parts pair with the high-poly parts ([`NamePair::list`]); `None` unless it bakes from a high poly "By Mesh Name"
    pub fn bake_name_pairs(&self, texture_set: usize) -> Result<Option<Vec<NamePair>>> {
        let set = self.doc.project.texture_set(texture_set)?;
        let settings = &set.bake.high_poly;
        let high = self.bake_meshes.merged_high().filter(|_| settings.match_mode == MeshMatchMode::ByMeshName && !settings.use_low_as_high);

        Ok(high.map(|high| NamePair::list(&self.mesh, set.material, &high.mesh, settings)))
    }

    /// The high-poly parts none of `texture_sets` bakes from, by name: a set matching "Always" bakes from every part, one matching "By Mesh Name" only from the parts its names pair with
    pub fn bake_unpaired_high_parts(&self, texture_sets: &[usize]) -> Result<Vec<String>> {
        let (mut paired, mut unpaired) = (BTreeSet::new(), BTreeSet::new());
        for &texture_set in texture_sets {
            let settings = &self.doc.project.texture_set(texture_set)?.bake.high_poly;
            if !settings.use_low_as_high && settings.match_mode == MeshMatchMode::Always {
                return Ok(Vec::new());
            }

            for pair in self.bake_name_pairs(texture_set)?.unwrap_or_default() {
                match pair {
                    NamePair { low: Some(_), high: Some(high) } => paired.insert(high),
                    NamePair { low: None, high: Some(high) } => unpaired.insert(high),
                    _ => false,
                };
            }
        }

        Ok(unpaired.difference(&paired).cloned().collect())
    }
}

/// Texels covered more than once and texels covered, rasterizing material `material`'s UVs over 0..1
fn overlap_counts(mesh: &Mesh, material: usize) -> (usize, usize) {
    let gbuffer = UvGBuffer::new(mesh, material, OVERLAP_CHECK_RESOLUTION, OVERLAP_CHECK_RESOLUTION);
    (gbuffer.overlaps as usize, gbuffer.covered_count())
}

/// Material `material`'s triangles sorted into the `baked_blocks` of a `grid` (columns, rows) atlas their UVs reach, one UV-only mesh per block with the block's UVs over 0..1: a single pass, however many blocks
fn triangles_by_block(mesh: &Mesh, material: usize, grid: (u32, u32), baked_blocks: &BTreeSet<(u32, u32)>) -> BTreeMap<(u32, u32), Mesh> {
    let scale = Vec2::new(grid.0 as f32, grid.1 as f32);
    let mut blocks: BTreeMap<(u32, u32), Mesh> = BTreeMap::new();
    for triangle in mesh.triangles_of(material) {
        let Some([first, second, third]) = mesh.tri(triangle) else { continue };
        let (Some(first_uv), Some(second_uv), Some(third_uv)) = (mesh.uvs.get(first), mesh.uvs.get(second), mesh.uvs.get(third)) else { continue };
        let uvs = [*first_uv * scale, *second_uv * scale, *third_uv * scale];

        // Every block the triangle's UV bounds touch, so a triangle across a border counts in both
        let (uv_min, uv_max) = uvs.iter().fold((Vec2::INFINITY, Vec2::NEG_INFINITY), |(uv_min, uv_max), uv| (uv_min.min(*uv), uv_max.max(*uv)));
        // Capped at the grid: a stray huge UV must not walk billions of blocks
        let columns = uv_min.x.floor().max(0.0) as u32..=((uv_max.x.ceil() as u32).saturating_sub(1)).min(grid.0 - 1);
        let rows = uv_min.y.floor().max(0.0) as u32..=((uv_max.y.ceil() as u32).saturating_sub(1)).min(grid.1 - 1);
        for column in columns {
            for row in rows.clone() {
                if !baked_blocks.contains(&(column, row)) {
                    continue;
                }

                let block = blocks.entry((column, row)).or_insert_with(|| Mesh { materials: vec![String::new()], ..Mesh::default() });
                let offset = Vec2::new(column as f32, row as f32);
                block.indices.extend(block.uvs.len() as u32..block.uvs.len() as u32 + 3);
                block.uvs.extend(uvs.iter().map(|uv| *uv - offset));
            }
        }
    }

    for block in blocks.values_mut() {
        block.submeshes = vec![SubMesh { name: String::new(), material: BLOCK_MATERIAL, start: 0, count: block.indices.len() as u32 }];
    }

    blocks
}
