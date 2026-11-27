//! Where skew or cage painting changed what a texture set's transfer rays read since its maps were baked

use texturecraft_bake::{TexelBox, region_keeps_pixels};
use texturecraft_core::{BakeSource, BakedMap};
use texturecraft_geom::glam::Vec2;

use crate::{Result, Session};

/// The UV box painted since the last bake of a texture set's auto-rebake map
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintedRegion {
    pub uv_min: Vec2,
    pub uv_max: Vec2,
    /// Counts up with every stroke, so a rebake can tell a newer stroke from the one it bakes
    pub stroke: u64,
}

/// Texels added around a painted box: bilinear sampling of the paint map and antialiasing samples reach one texel past it
const MARGIN_TEXELS: u32 = 1;

impl Session {
    /// Note that the bake paint of `texture_set` changed inside `uv_min`..`uv_max` (the whole map when `None`)
    pub(super) fn note_bake_paint(&mut self, texture_set: usize, uv_box: Option<(Vec2, Vec2)>) {
        let (uv_min, uv_max) = uv_box.unwrap_or((Vec2::ZERO, Vec2::ONE));
        let (uv_min, uv_max) = match self.doc.painted_regions.get(&texture_set) {
            Some(region) => (region.uv_min.min(uv_min), region.uv_max.max(uv_max)),
            None => (uv_min, uv_max),
        };
        self.painted_strokes += 1;
        self.doc.painted_regions.insert(texture_set, PaintedRegion { uv_min, uv_max, stroke: self.painted_strokes });
    }

    /// Forget the painted box of `texture_set` once its auto-rebake map is baked again, unless a stroke came after `painted_stroke`
    pub(super) fn forget_painted_region(&mut self, texture_set: usize, stored: &[BakedMap], painted_stroke: u64) {
        let auto = self.doc.project.texture_sets.get(texture_set).and_then(|set| set.auto_rebake);
        let painted_since = self.doc.painted_regions.get(&texture_set).is_some_and(|region| region.stroke > painted_stroke);
        if !painted_since && auto.is_none_or(|map| stored.contains(&map)) {
            self.doc.painted_regions.remove(&texture_set);
        }
    }

    /// Baked auto-rebake maps changed only by painting since, as (set, map, what a bake reads now, newest stroke)
    pub fn auto_rebake_painted(&self) -> Vec<(usize, BakedMap, BakeSource, u64)> {
        let regions = self.doc.painted_regions.iter();
        regions
            .filter_map(|(index, region)| {
                let set = self.doc.project.texture_sets.get(*index)?;
                let map = set.auto_rebake.filter(|map| set.baked.contains(map))?;
                let from_high = !set.bake.high_poly.use_low_as_high && !self.doc.bake_meshes.high.is_empty();
                from_high.then_some(())?;
                let current = self.bake_source(*index, map).ok()?;

                // Only painting changed it: a map without a record is never out of date
                let up_to_date = set.bake_sources.get(&map).is_none_or(|record| *record == current);
                up_to_date.then_some((*index, map, current, region.stroke))
            })
            .collect()
    }

    /// A task that bakes `map` again only where painting changed it, keeping the rest; else the whole map
    pub fn bake_painted_task(&self, texture_set: usize, map: BakedMap) -> Result<crate::BakeTask> {
        let mut task = self.bake_task_for(texture_set, &self.auto_rebake_maps(texture_set, map))?;
        let Some(region) = self.doc.painted_regions.get(&texture_set) else { return Ok(task) };

        let (width, height) = task.settings.common.output_size();
        let stored =
            self.doc.baked.get(&texture_set).filter(|stored| stored.resolution == width && stored.height(map) == height && stored.maps.contains_key(&map));
        let Some(stored) = stored.filter(|_| task.tiles == (1, 1) && region_keeps_pixels(map)) else { return Ok(task) };

        let texel = |uv: f32, size: u32, round: fn(f32) -> f32| (round(uv.clamp(0.0, 1.0) * size as f32) as u32).min(size);
        let painted = TexelBox {
            min: (texel(region.uv_min.x, width, f32::floor), texel(region.uv_min.y, height, f32::floor)),
            max: (texel(region.uv_max.x, width, f32::ceil), texel(region.uv_max.y, height, f32::ceil)),
        };
        task.previous_maps = stored.clone();
        task.region = Some(painted.grown(MARGIN_TEXELS, width, height));
        Ok(task)
    }
}
