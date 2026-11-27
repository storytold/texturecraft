//! What each baked map was baked from, and which maps are out of date since

use std::collections::BTreeMap;

use texturecraft_core::{BakeChange, BakeSource, BakedMap, CageType, TextureSet, fingerprint, fingerprint_words};

use crate::{Result, Session};

impl Session {
    /// What a bake of `map` on texture set `texture_set` would read now
    pub fn bake_source(&self, texture_set: usize, map: BakedMap) -> Result<BakeSource> {
        let set = self.doc.project.texture_set(texture_set)?;
        Ok(self.map_bake_source(set, map, self.set_bake_source(set)))
    }

    /// The inputs of the maps selected for one task.
    pub(super) fn task_bake_sources(&self, set: &TextureSet, maps: impl Iterator<Item = BakedMap>) -> BTreeMap<BakedMap, BakeSource> {
        let shared = std::cell::LazyCell::new(|| self.set_bake_source(set));
        maps.map(|map| (map, self.map_bake_source(set, map, *shared))).collect()
    }

    /// The inputs shared by every map in a texture set.
    fn set_bake_source(&self, set: &TextureSet) -> BakeSource {
        let from_high = !set.bake.high_poly.use_low_as_high && !self.doc.bake_meshes.high.is_empty();
        let high_poly = match from_high {
            true => fingerprint_words(self.doc.bake_meshes.high.iter().map(|high| high.fingerprint)),
            false => 0,
        };
        let reads_cage = from_high && set.bake.cage.kind == CageType::CustomFile;
        let cage = self.doc.bake_meshes.cage.as_ref().filter(|_| reads_cage).map_or(0, |cage| cage.fingerprint);

        BakeSource {
            low_poly: self.low_poly_fingerprint(),
            low_material_file: None,
            high_poly,
            cage,
            settings: set.bake_settings_fingerprint(),
            map_settings: 0,
        }
    }

    fn map_bake_source(&self, set: &TextureSet, map: BakedMap, shared: BakeSource) -> BakeSource {
        let reads_low_colors = shared.high_poly == 0 && set.bake.maps.reads_material_colors(map);
        let low_material_file =
            self.mesh_source.as_ref().filter(|_| reads_low_colors).map(|(_, bytes)| *self.low_material_file_fingerprint.get_or_init(|| fingerprint(bytes)));

        BakeSource { low_material_file, map_settings: set.map_settings_fingerprint(map), ..shared }
    }

    /// What changed since `map` of texture set `texture_set` was baked; empty when up to date, not baked or without a record
    pub fn bake_changes(&self, texture_set: usize, map: BakedMap) -> Vec<BakeChange> {
        let Ok(set) = self.doc.project.texture_set(texture_set) else { return Vec::new() };

        let Some(record) = set.bake_sources.get(&map).filter(|_| set.baked.contains(&map)) else { return Vec::new() };

        self.bake_source(texture_set, map).map(|current| record.changes(&current)).unwrap_or_default()
    }

    /// The out-of-date maps of texture set `texture_set`, each with what changed
    pub fn out_of_date_maps(&self, texture_set: usize) -> BTreeMap<BakedMap, Vec<BakeChange>> {
        let Ok(set) = self.doc.project.texture_set(texture_set) else { return BTreeMap::new() };

        let mut records = set.bake_sources.iter().filter(|(map, _)| set.baked.contains(map)).peekable();

        if records.peek().is_none() {
            return BTreeMap::new();
        }

        let shared = self.set_bake_source(set);
        let changes = records.map(|(map, record)| (*map, record.changes(&self.map_bake_source(set, *map, shared))));
        changes.filter(|(_, changes)| !changes.is_empty()).collect()
    }

    /// Fingerprint of the low poly's geometry, tangent frame, vertex colors and part assignments (computed once per loaded mesh)
    fn low_poly_fingerprint(&self) -> u64 {
        *self.low_poly_fingerprint.get_or_init(|| {
            let source = &self.source;
            let floats = source
                .positions
                .iter()
                .chain(&source.normals)
                .flat_map(|vector| vector.to_array())
                .chain(source.tangents.iter().flat_map(|tangent| tangent.to_array()))
                .chain(source.uvs.iter().flat_map(|uv| uv.to_array()))
                .chain(source.colors.iter().flat_map(|color| color.to_array()));
            let indices = source.indices.iter().copied().map(u64::from);
            let parts =
                source.submeshes.iter().flat_map(|part| [fingerprint(part.name.as_bytes()), part.material as u64, part.start.into(), part.count.into()]);

            fingerprint_words(floats.map(|float| u64::from(float.to_bits())).chain(indices).chain(parts))
        })
    }

    /// Keep what the `stored` maps of `texture_set` were baked from and the newest stroke their bake saw; without it they lose their record
    pub(super) fn record_bake_sources(
        &mut self,
        texture_set: usize,
        recorded: Option<(&BTreeMap<BakedMap, BakeSource>, u64)>,
        stored: &[BakedMap],
    ) -> Result<()> {
        let set = self.doc.project.texture_set_mut(texture_set)?;
        match recorded {
            Some((sources, _)) => set.bake_sources.extend(sources),
            None => set.bake_sources.retain(|map, _| !stored.contains(map)),
        }

        let baked = &set.baked;
        set.bake_sources.retain(|map, _| baked.contains(map));
        if let Some((_, painted_stroke)) = recorded {
            self.forget_painted_region(texture_set, stored, painted_stroke);
        }

        Ok(())
    }
}
