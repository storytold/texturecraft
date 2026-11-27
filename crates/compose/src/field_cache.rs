use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use texturecraft_core::BakedMap;
use texturecraft_geom::NO_TRI;

use crate::eval::Inputs;
use crate::field_cache_key::FieldCacheKey;
use crate::sampled_map::SampledMap;
use crate::working_field::working_block;

/// Most fields cached, and the bytes they may hold (the newest is kept whatever its size)
pub(crate) const CACHE_ENTRIES: usize = 8;
const CACHE_BYTES: usize = 256 << 20;

/// FNV-1a prime for the dense texel scan
const TEXEL_HASH_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const TEXEL_HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Geometry and layout data read by the whole-map generators
pub(crate) fn field_identity(inputs: &Inputs) -> u64 {
    let gbuf = inputs.gbuf;
    let mesh = inputs.mesh;
    let mut hash = DefaultHasher::new();
    (gbuf.width, gbuf.height, gbuf.tri.len(), gbuf.bary.len()).hash(&mut hash);

    // Keep this scan synchronous because Rayon tile plans can share a OnceLock
    let mut texels = TEXEL_HASH_BASIS;
    let mut mix_texel = |value: u32| texels = (texels ^ u64::from(value)).wrapping_mul(TEXEL_HASH_PRIME);
    for (index, &triangle) in gbuf.tri.iter().enumerate() {
        mix_texel(triangle);
        if triangle != NO_TRI
            && let Some(weights) = gbuf.bary.get(index)
        {
            mix_texel(weights[0].to_bits());
            mix_texel(weights[1].to_bits());
        }
    }
    texels.hash(&mut hash);

    mesh.indices.hash(&mut hash);
    mesh.positions.len().hash(&mut hash);
    mesh.positions.iter().for_each(|value| value.to_array().map(f32::to_bits).hash(&mut hash));
    mesh.uvs.len().hash(&mut hash);
    mesh.uvs.iter().for_each(|value| value.to_array().map(f32::to_bits).hash(&mut hash));
    mesh.normals.len().hash(&mut hash);
    mesh.normals.iter().for_each(|value| value.to_array().map(f32::to_bits).hash(&mut hash));

    // Surface interpolation needs tangent slots to exist even when generators do not use the values
    mesh.tangents.len().hash(&mut hash);
    inputs.bounds.0.to_array().map(f32::to_bits).hash(&mut hash);
    inputs.bounds.1.to_array().map(f32::to_bits).hash(&mut hash);
    let set = inputs.project.texture_set(inputs.ts).ok();
    if let Some(set) = set {
        set.material.hash(&mut hash);
        mesh.submeshes.len().hash(&mut hash);
        mesh.submeshes.iter().for_each(|part| (part.material, part.start, part.count).hash(&mut hash));
        set.uv_tiles.hash(&mut hash);
        set.resolution.hash(&mut hash);
        set.tile_resolutions.hash(&mut hash);
    }

    hash.finish()
}

/// Built fields by what they were built from, most recently used last
pub(crate) struct FieldCache {
    entries: Mutex<Vec<(FieldCacheKey, Arc<Vec<f32>>)>>,
}

impl FieldCache {
    pub(crate) const fn new() -> FieldCache {
        FieldCache { entries: Mutex::new(Vec::new()) }
    }

    /// Reuse a field keyed by [`field_identity`], the baked maps it reads and its settings.
    /// Run the missing field's parallel build outside the cache lock.
    pub(crate) fn get_or_build(
        &self,
        inp: &Inputs,
        geometry: u64,
        maps: &[BakedMap],
        settings: Vec<u32>,
        build: impl FnOnce((u32, u32)) -> Vec<f32>,
    ) -> Arc<Vec<f32>> {
        let block = working_block(inp);
        let read = maps.iter().filter_map(|map| {
            inp.baked.maps.get(map).map(|data| SampledMap { data: Arc::downgrade(data), baked_size: (inp.baked.resolution, inp.baked.height(*map)) })
        });
        let key = FieldCacheKey { geometry, block, scales: inp.block_scales.clone(), maps: read.collect(), settings };

        // A hit moves to the most recently used end
        let cached = {
            let mut entries = self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            entries.retain(|(key, _)| key.maps.iter().all(|map| map.data.strong_count() > 0));
            let hit = entries.iter().position(|(cached, _)| *cached == key).map(|index| entries.remove(index));
            hit.map(|(cached, field)| {
                entries.push((cached, field.clone()));
                field
            })
        };
        if let Some(field) = cached {
            return field;
        }

        let field = Arc::new(build(block));
        let mut entries = self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.retain(|(cached, _)| *cached != key);
        entries.push((key, field.clone()));
        let mut bytes = 0usize;
        let keep = entries
            .iter()
            .rev()
            .take_while(|(_, built)| {
                bytes = bytes.saturating_add(built.len().saturating_mul(size_of::<f32>()));
                bytes <= CACHE_BYTES
            })
            .count()
            .clamp(1, CACHE_ENTRIES);
        let excess = entries.len().saturating_sub(keep);
        entries.drain(..excess);
        field
    }
}
