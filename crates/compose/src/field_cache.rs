//! Whole-set fields kept across the blocks of a composite and across composites.
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

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use texturecraft_core::rules::CompositeRules;
use texturecraft_core::{Layer, LayerId};

use crate::below::{StackKey, effect_ids};
use crate::eval::{Field, FieldKey, Inputs, PaintTarget};

/// Default byte budget of a [`FieldCache`]: AS-sized sets (19 whole-set fields, 0.94 GB at 2048)
/// fit; beyond it the remaining fields are computed per block as they would be without the cache.
pub const FIELD_BUDGET: usize = 2 << 30;

/// One field: which layer and job, and the filter of its chain (counted among the neighbourhood
/// filters, bottom → top).
pub(crate) type Slot = (FieldKey, usize);

/// Everything a cached field reads (see the module docs).
#[derive(Debug, PartialEq)]
struct Key {
    stack: StackKey,
    rules: CompositeRules,
}

impl Key {
    fn of(inp: &Inputs, roots: Vec<LayerId>, tree: &BTreeSet<LayerId>) -> Option<Key> {
        let stack = StackKey::of_tree(inp.project, inp.pixels, inp.baked, inp.images, inp.ts, roots, tree)?;
        Some(Key { stack, rules: inp.project.rules })
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
pub struct FieldCache {
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
}

impl std::fmt::Debug for FieldCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldCache").field("fields", &self.entries.len()).field("bytes", &self.bytes).field("budget", &self.budget).finish()
    }
}

impl Default for FieldCache {
    fn default() -> Self {
        FieldCache {
            entries: HashMap::new(),
            bytes: 0,
            budget: FIELD_BUDGET,
            whole: None,
            pass: 0,
            current: None,
            confirmed: HashSet::new(),
            single: false,
            reused: 0,
        }
    }
}

impl FieldCache {
    /// A cache for one composite call (the inputs cannot change during it).
    pub(crate) fn single() -> FieldCache {
        FieldCache { single: true, ..FieldCache::default() }
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

    /// Start a composite (one call of `composite_tiles_cached`).
    pub(crate) fn begin(&mut self) {
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
    }

    /// Whether every entry is current for `inp` (computed once per composite).
    fn unchanged(&mut self, inp: &Inputs) -> bool {
        if self.single {
            return true;
        }
        if let Some(c) = self.current {
            return c;
        }
        let stack = inp.project.texture_set(inp.ts).map(|s| s.stack.clone()).unwrap_or_default();
        let key = Key::of(inp, stack.clone(), &inp.project.render_tree(&stack));
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

    /// Keep `field` for `slot` (see [`FieldCache::get`]) when it fits the budget, dropping fields no composite used
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
