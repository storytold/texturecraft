//! [`SetFieldCache`]: whole-set fields kept across the blocks of a composite and across composites.
//!
//! A graph filter that reads the stack reads all of it, so its field (see `eval::field`) covers the
//! whole texture set whatever tiles are composited. Computed per block, every block (and in a
//! frame-sliced composite, every frame) would compute the same field again. This cache keeps such
//! fields, each keyed by everything it reads: the layers it is computed from (the layer itself, and
//! every layer drawn before it when it reads the stack below), their effects, masks, pixels, the
//! set, baked maps, images and the blend rules. Any change to those makes the entry stale; it is
//! then computed again and replaced. Reused fields are the same values a fresh computation gives,
//! so results are identical to computing them every time.
//!
//! Checking every entry's key on every composite would cost more than a frame, so the cache keeps
//! the key of the whole stack as well: while that is unchanged every entry is current. When it
//! changed, the composite that notices checks each entry it meets against the entry's own key
//! and drops the entries it did not confirm.
//!
//! The second cache, [`FieldCache`], keeps the whole-map generator fields (UV islands, borders, drips) keyed by the
//! geometry and baked maps they read.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use texturecraft_core::rules::CompositeRules;
use texturecraft_core::{BakedMap, Layer, LayerId};
use texturecraft_geom::NO_TRI;

use crate::below::{StackKey, effect_ids};
use crate::eval::{Field, FieldKey, Inputs, PaintTarget};
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

/// Default byte budget of a [`SetFieldCache`]: AS-sized sets (19 whole-set fields, 0.94 GB at 2048)
/// fit; beyond it the remaining fields are computed per block as they would be without the cache.
pub const FIELD_BUDGET: usize = 2 << 30;

/// One field: which layer and job, and the filter of its chain (counted among the neighbourhood
/// filters, bottom → top).
pub(crate) type Slot = (FieldKey, usize);

/// Everything a cached field reads (see the module docs).
#[derive(Debug, PartialEq)]
struct Key {
    stack: Arc<StackKey>,
    rules: CompositeRules,
}

impl Key {
    fn of(inp: &Inputs, roots: Vec<LayerId>, tree: &BTreeSet<LayerId>) -> Option<Key> {
        let stack = StackKey::of_tree(inp.project, inp.pixels, inp.baked, inp.images, inp.ts, roots, tree)?;
        Some(Key { stack: Arc::new(stack), rules: inp.project.rules })
    }
}

struct Entry {
    /// None in a single composite's cache, which never checks keys.
    key: Option<Key>,
    /// The layers the key covers (a live stroke on one of them changes the field).
    tree: BTreeSet<LayerId>,
    field: Arc<Field>,
    /// The composite that last used it.
    used: u64,
}

/// See the module docs. One per texture set (inside its [`crate::BelowCache`]), or one per
/// composite call.
pub struct SetFieldCache {
    entries: HashMap<Slot, Entry>,
    bytes: usize,
    /// Byte budget: a field that does not fit is not kept (it is computed again per block).
    pub budget: usize,
    /// The key of the whole stack when every entry was last confirmed.
    whole: Option<Key>,
    /// Bumps per composite.
    pass: u64,
    /// This composite's state: `None` until the first lookup, then whether the whole stack's key
    /// was unchanged (entries are current) or not (each entry is checked, unconfirmed ones dropped).
    current: Option<bool>,
    /// Slots confirmed or computed during a composite that found the stack changed.
    confirmed: HashSet<Slot>,
    /// One composite only: nothing can change during it, so nothing is checked.
    single: bool,
    /// Fields taken from the cache instead of computed, since it was made.
    reused: u64,
    /// The whole stack's key for this composite, when the caller built it already (see [`SetFieldCache::begin`]).
    stack_key: Option<Arc<StackKey>>,
}

impl std::fmt::Debug for SetFieldCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetFieldCache").field("fields", &self.entries.len()).field("bytes", &self.bytes).field("budget", &self.budget).finish()
    }
}

impl Default for SetFieldCache {
    fn default() -> Self {
        SetFieldCache {
            entries: HashMap::new(),
            bytes: 0,
            budget: FIELD_BUDGET,
            whole: None,
            pass: 0,
            current: None,
            confirmed: HashSet::new(),
            single: false,
            reused: 0,
            stack_key: None,
        }
    }
}

impl SetFieldCache {
    /// A cache for one composite call (the inputs cannot change during it).
    pub(crate) fn single() -> SetFieldCache {
        SetFieldCache { single: true, ..SetFieldCache::default() }
    }

    /// Fields held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Fields taken from the cache instead of computed, since it was made.
    pub fn reused(&self) -> u64 {
        self.reused
    }

    /// Bytes held.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.whole = None;
        self.confirmed.clear();
        self.current = None;
    }

    /// Start a composite (one call of `composite_tiles_cached`). `stack_key`: the key of the set's whole stack for its
    /// inputs, when the caller built one (the cache then does not build it again).
    pub(crate) fn begin(&mut self, stack_key: Option<Arc<StackKey>>) {
        self.stack_key = stack_key;
        self.pass += 1;
        self.current = None;
        self.confirmed.clear();
    }

    /// End a composite: when it found the stack changed, entries it did not confirm are dropped (their layers moved,
    /// were deleted or lost the filter, or a live stroke kept them from being checked). A composite that looked none
    /// up checks the stack here, so the fields of filters that are gone do not stay.
    pub(crate) fn end(&mut self, inp: &Inputs) {
        if self.current.is_none() && !self.entries.is_empty() {
            self.unchanged(inp);
        }
        if self.current == Some(false) {
            let confirmed = std::mem::take(&mut self.confirmed);
            let mut freed = 0;
            self.entries.retain(|slot, e| {
                let keep = confirmed.contains(slot);
                if !keep {
                    freed += e.field.bytes();
                }
                keep
            });
            self.bytes = self.bytes.saturating_sub(freed);
        }
        self.current = None;
        self.stack_key = None;
    }

    /// Whether every entry is current for `inp` (computed once per composite).
    fn unchanged(&mut self, inp: &Inputs) -> bool {
        if self.single {
            return true;
        }
        if let Some(c) = self.current {
            return c;
        }
        let key = match &self.stack_key {
            Some(stack) => Some(Key { stack: stack.clone(), rules: inp.project.rules }),
            None => {
                let stack = inp.project.texture_set(inp.ts).map(|s| s.stack.clone()).unwrap_or_default();
                Key::of(inp, stack.clone(), &inp.project.render_tree(&stack))
            }
        };
        let same = key.is_some() && key == self.whole;
        if !same {
            self.whole = key;
        }
        self.current = Some(same);
        same
    }

    /// The field kept for `slot`, computed from the layers `tree` (drawn as `roots`), when it is current.
    pub(crate) fn get(&mut self, inp: &Inputs, slot: Slot, roots: &[LayerId], tree: &BTreeSet<LayerId>) -> Option<Arc<Field>> {
        if previewed(inp, tree) {
            return None;
        }
        let unchanged = self.unchanged(inp);
        let pass = self.pass;
        let entry = self.entries.get_mut(&slot)?;
        if !unchanged && (entry.tree != *tree || Key::of(inp, roots.to_vec(), tree) != entry.key || entry.key.is_none()) {
            if let Some(stale) = self.entries.remove(&slot) {
                self.bytes = self.bytes.saturating_sub(stale.field.bytes());
            }
            return None;
        }
        entry.used = pass;
        let field = entry.field.clone();
        self.reused += 1;
        if !unchanged {
            self.confirmed.insert(slot);
        }
        Some(field)
    }

    /// Keep `field` for `slot` (see [`SetFieldCache::get`]) when it fits the budget, dropping fields no composite used
    /// since the last one first.
    pub(crate) fn put(&mut self, inp: &Inputs, slot: Slot, roots: &[LayerId], tree: &BTreeSet<LayerId>, field: &Arc<Field>) {
        if previewed(inp, tree) {
            return;
        }
        let unchanged = self.unchanged(inp);
        let size = field.bytes();
        if size > self.budget {
            return;
        }
        if let Some(old) = self.entries.remove(&slot) {
            self.bytes = self.bytes.saturating_sub(old.field.bytes());
        }
        if self.bytes + size > self.budget {
            let pass = self.pass;
            let mut freed = 0;
            self.entries.retain(|_, e| {
                let keep = e.used + 1 >= pass;
                if !keep {
                    freed += e.field.bytes();
                }
                keep
            });
            self.bytes = self.bytes.saturating_sub(freed);
            if self.bytes + size > self.budget {
                return;
            }
        }
        // a single composite never checks keys: none is built
        let key = if self.single {
            None
        } else {
            let Some(key) = Key::of(inp, roots.to_vec(), tree) else { return };
            Some(key)
        };
        self.entries.insert(slot, Entry { key, tree: tree.clone(), field: field.clone(), used: self.pass });
        self.bytes += size;
        if !unchanged {
            self.confirmed.insert(slot);
        }
    }
}

/// Whether the live stroke paints one of the layers in `tree` (or an effect of one): fields computed from them hold
/// it, so they are neither taken from the cache nor kept.
fn previewed(inp: &Inputs, tree: &BTreeSet<LayerId>) -> bool {
    let Some(pv) = inp.preview else { return false };
    let (PaintTarget::Layer(t) | PaintTarget::Mask(t)) = pv.target;
    tree.contains(&t) || tree.iter().filter_map(|id| inp.project.layers.get(id)).flat_map(|l: &Layer| effect_ids(l)).any(|e| e == t)
}
