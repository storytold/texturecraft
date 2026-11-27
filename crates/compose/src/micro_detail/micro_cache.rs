use std::sync::{Arc, Mutex};

use super::cache_key::CacheKey;
use super::cached_bytes::CachedBytes;
use super::{CACHE_BYTES, CACHE_ENTRIES};

/// Built maps by what they were built from, most recently used last
pub(super) struct MicroCache<T> {
    entries: Mutex<Vec<(CacheKey, Arc<T>)>>,
}

impl<T: CachedBytes> MicroCache<T> {
    pub(super) const fn new() -> MicroCache<T> {
        MicroCache { entries: Mutex::new(Vec::new()) }
    }

    /// The map built from `key`, built by `build` when missing (outside the lock: the build runs parallel work)
    pub(super) fn get_or_build(&self, key: CacheKey, build: impl FnOnce() -> T) -> Arc<T> {
        let found = {
            let mut entries = self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            entries.retain(|(key, _)| key.map.strong_count() > 0);
            entries.iter().position(|(cached, _)| *cached == key).map(|index| {
                let entry = entries.remove(index);
                let built = entry.1.clone();
                entries.push(entry);
                built
            })
        };
        if let Some(built) = found {
            return built;
        }

        let built = Arc::new(build());
        let mut entries = self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        entries.retain(|(cached, _)| *cached != key);
        entries.push((key, built.clone()));
        let mut bytes = 0usize;
        let keep = entries
            .iter()
            .rev()
            .take_while(|(_, built)| {
                bytes = bytes.saturating_add(built.bytes());
                bytes <= CACHE_BYTES
            })
            .count()
            .clamp(1, CACHE_ENTRIES);
        let excess = entries.len().saturating_sub(keep);
        entries.drain(..excess);
        built
    }
}
