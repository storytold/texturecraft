//! A canceled bake keeps the maps it finished: the Baking Log ticked them and the 3D view may have shown them

use std::collections::BTreeMap;

use texturecraft_compose::BakedMaps;
use texturecraft_core::{BakeSource, BakedMap};

use crate::{Result, Session};

impl Session {
    /// Store the maps the canceled bake of `texture_set` `finished` as one undo step and return them; none at a new output size, which would drop the set's other maps
    pub fn keep_canceled_bake(
        &mut self,
        texture_set: usize,
        finished: BakedMaps,
        sources: &BTreeMap<BakedMap, BakeSource>,
        painted_stroke: u64,
    ) -> Result<Vec<BakedMap>> {
        let Some(height) = finished.maps.keys().next().map(|map| finished.height(*map)) else { return Ok(Vec::new()) };

        let resized = self.doc.baked.get(&texture_set).is_some_and(|stored| {
            !stored.maps.is_empty() && (stored.resolution != finished.resolution || stored.maps.keys().any(|map| stored.height(*map) != height))
        });
        if resized {
            return Ok(Vec::new());
        }

        // A map the bake did not finish keeps the record of what its stored pixels were baked from
        let kept_sources: BTreeMap<BakedMap, BakeSource> =
            sources.iter().filter(|(map, _)| finished.maps.contains_key(*map)).map(|(map, source)| (*map, *source)).collect();
        let kept: Vec<BakedMap> = finished.maps.keys().copied().collect();
        self.edit("Bake mesh maps", |session| {
            session.store_bake_maps(texture_set, finished.resolution, height, finished.maps, Some((&kept_sources, painted_stroke)))
        })?;

        // Named like the Texture Set List and the progress footer, which count sets from 1
        let name = self.doc.project.texture_sets.get(texture_set).map_or_else(|| format!("{}", texture_set + 1), |set| set.name.clone());
        self.log.info("Baker", format!("Kept {} baked maps of texture set {name} from the canceled bake", kept.len()));
        Ok(kept)
    }
}
