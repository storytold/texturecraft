//! A texture set's UV padding: from island edges, and across seams for the texels past one

use std::collections::{BTreeMap, BTreeSet};

use texturecraft_core::{Channel, ChannelKind, UvPaddingMode};
use texturecraft_texel::TILE;
use texturecraft_texel::dilate::DilationMap;

use crate::SeamPaddingInput;
use crate::seam_padding::{SEAM_REACH, cross_seams};

/// A texture set's padding: `flat` for every texel, with the few texels past a seam listed apart in `crossed` by row
#[derive(Clone, Debug, PartialEq)]
pub struct PaddingMaps {
    /// Padding from island edges (2D), which tangent-space normals always take: a normal turns with its island, so it can't continue across a seam
    pub flat: DilationMap,
    /// Across seams (3D): (texel, source texel) indices of the padding texels that continue from a seam's other side, sorted by texel
    pub crossed: Vec<(u32, u32)>,
    /// Where each row's texels start in `crossed`, one more than the rows (empty without any), so a lookup searches one row
    pub(crate) row_starts: Vec<u32>,
    /// For each tile holding a seam's far side, the tiles whose padding continues from it, so a change there repads them however far they are
    pub(crate) crossed_into: BTreeMap<(u32, u32), BTreeSet<(u32, u32)>>,
}

impl PaddingMaps {
    /// The padding for `mode`: `covered` texels of a `width × height` set padded up to `max_dist` texels inside each `block`
    pub fn new(
        mode: UvPaddingMode,
        covered: &[bool],
        (width, height): (u32, u32),
        max_dist: Option<u32>,
        block: (u32, u32),
        seams: Option<&SeamPaddingInput<'_>>,
    ) -> PaddingMaps {
        let flat = DilationMap::in_rectangular_blocks(covered, width, height, max_dist, block);
        let crossed = seams
            .filter(|_| mode == UvPaddingMode::Space3d)
            .map_or_else(Vec::new, |seams| cross_seams(&flat, max_dist.map_or(SEAM_REACH, |d| d.min(SEAM_REACH)), block, seams));
        let row_starts =
            if crossed.is_empty() { Vec::new() } else { (0..=height).map(|y| crossed.partition_point(|(texel, _)| *texel < y * width) as u32).collect() };
        let tile_of = |texel: u32| ((texel % width) / TILE, (texel / width) / TILE);
        let mut crossed_into: BTreeMap<(u32, u32), BTreeSet<(u32, u32)>> = BTreeMap::new();
        for &(texel, source) in &crossed {
            crossed_into.entry(tile_of(source)).or_default().insert(tile_of(texel));
        }
        PaddingMaps { flat, crossed, row_starts, crossed_into }
    }

    /// The tiles whose padding continues across a seam from texels in `tile`
    pub fn tiles_crossed_from(&self, tile: (u32, u32)) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.crossed_into.get(&tile).into_iter().flatten().copied()
    }

    /// Texels across and down
    pub fn size(&self) -> (u32, u32) {
        (self.flat.width, self.flat.height)
    }

    /// Whether `channel` pads across seams; tangent-space normals pad from island edges
    pub fn crosses_seams(channel: Channel) -> bool {
        channel.kind() != ChannelKind::NormalMap
    }

    /// The covered texel padding (x, y), itself when covered; `across_seams` takes the seams' other sides
    pub fn source(&self, across_seams: bool, x: u32, y: u32) -> Option<(u32, u32)> {
        let source = self.flat.source(x, y)?;
        if !across_seams || source == (x, y) {
            return Some(source);
        }

        let (texel, width) = (y * self.flat.width + x, self.flat.width);
        let row = match (self.row_starts.get(y as usize), self.row_starts.get(y as usize + 1)) {
            (Some(&start), Some(&end)) => self.crossed.get(start as usize..end as usize).unwrap_or_default(),
            _ => &[],
        };
        match row.binary_search_by_key(&texel, |(crossed_texel, _)| *crossed_texel) {
            Ok(found) => row.get(found).map(|(_, crossed_source)| (crossed_source % width, crossed_source / width)),
            Err(_) => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use texturecraft_geom::glam::Vec2;
    use texturecraft_geom::{NO_TRI, TrianglePartition, UvSeam, UvSeamSide, primitives};

    use super::*;

    /// Two islands of one 3D edge: the left one's right side is the right one's left side
    fn two_islands() -> (Vec<bool>, Vec<UvSeam>) {
        let (width, height) = (32u32, 8u32);
        let covered = (0..width * height).map(|i| matches!(i % width, 4..=11 | 20..=27)).collect();
        let at = |x: f32, y: f32| Vec2::new(x / width as f32, y / height as f32);
        let left = UvSeamSide { triangle: 0, start: at(12.0, 0.0), end: at(12.0, 8.0), interior: at(4.0, 0.0) };
        let right = UvSeamSide { triangle: 1, start: at(20.0, 0.0), end: at(20.0, 8.0), interior: at(28.0, 0.0) };
        (covered, vec![UvSeam { sides: [left, right] }])
    }

    fn islands(seams: Vec<UvSeam>, triangles: &[u32]) -> SeamPaddingInput<'_> {
        // A synthetic raster supplies its island ids directly
        let mesh = primitives::plane(1.0, 2);
        let partition = TrianglePartition::new(&mesh, 0, |_, _| Vec::<u32>::new());
        SeamPaddingInput::new(seams, triangles, partition)
    }

    #[test]
    fn padding_stops_at_the_far_islands_edge() {
        let (_, seams) = two_islands();
        let (width, height) = (32, 8);
        for (gap, far_strip) in [(false, 2), (true, 2), (true, 1)] {
            let triangles: Vec<_> = (0..width * height)
                .map(|pixel| match pixel % width {
                    4..=11 => 0,
                    20..=21 => 1,
                    22 if gap => NO_TRI,
                    22..=27 => far_strip,
                    _ => NO_TRI,
                })
                .collect();
            let covered: Vec<_> = triangles.iter().map(|id| *id != NO_TRI).collect();
            let input = islands(seams.clone(), &triangles);

            let padding = PaddingMaps::new(UvPaddingMode::Space3d, &covered, (width, height), Some(4), (0, 0), Some(&input));

            assert_eq!(padding.source(true, 13, 3), Some((21, 3)), "a texel inside the far island continues the seam");
            assert_eq!(padding.source(true, 15, 3), padding.source(false, 15, 3), "another island is out of reach even with no texel gap");
        }
    }

    #[test]
    fn padding_past_a_seam_continues_from_its_other_side_and_normals_stay_flat() {
        let (covered, seams) = two_islands();
        let triangles: Vec<_> = (0..32 * 8)
            .map(|pixel| match pixel % 32 {
                4..=11 => 0,
                20..=27 => 1,
                _ => NO_TRI,
            })
            .collect();
        let input = islands(seams, &triangles);

        let space_3d = PaddingMaps::new(UvPaddingMode::Space3d, &covered, (32, 8), Some(4), (0, 0), Some(&input));
        let space_2d = PaddingMaps::new(UvPaddingMode::Space2d, &covered, (32, 8), Some(4), (0, 0), Some(&input));
        let island = |padding: &PaddingMaps, across_seams: bool, x: u32| {
            padding.source(across_seams, x, 3).map(|(source_x, _)| if source_x < 16 { "left" } else { "right" })
        };

        // Just right of the left island, then just left of the right island
        assert_eq!((island(&space_3d, true, 13), island(&space_3d, true, 18)), (Some("right"), Some("left")));
        assert_eq!((island(&space_2d, true, 13), island(&space_2d, true, 18)), (Some("left"), Some("right")));
        assert_eq!((island(&space_3d, false, 13), island(&space_3d, false, 18)), (Some("left"), Some("right")));
        assert!(PaddingMaps::crosses_seams(Channel::BaseColor) && !PaddingMaps::crosses_seams(Channel::Normal));

        // Away from the seam, both modes pad from the near island, and only texels between the islands are kept apart
        assert_eq!(island(&space_3d, true, 2), Some("left"));
        assert!(!space_3d.crossed.is_empty() && space_2d.crossed.is_empty());
        assert!(space_3d.crossed.iter().all(|(texel, _)| (12..20).contains(&(texel % 32))));
    }
}
