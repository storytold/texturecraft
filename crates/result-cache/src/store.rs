// SPDX-License-Identifier: MIT OR Apache-2.0
//! [`DiskStore`]: entries in a folder, `<space>/<first two of the key>/<key>.tcresult`.

use crate::config::Config;
use crate::entry;
use crate::eviction::{self, ENTRY_EXTENSION, TEMPORARY_EXTENSION};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// Written bytes between looks at the folder's size, at most: walking it costs little, but not nothing. A process can go over
/// the limit by that much, so a share of the limit where that is less.
const TRIM_EVERY: u64 = 512 << 20;
/// That share, as a divisor of the limit.
const TRIM_SHARE: u64 = 16;

/// What this process did with the store.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub hits: u64,
    pub misses: u64,
    /// Entries found damaged, removed and counted as misses.
    pub damaged: u64,
    pub written: u64,
    pub written_bytes: u64,
    pub read_bytes: u64,
}

#[derive(Debug)]
pub struct DiskStore {
    folder: PathBuf,
    limit: u64,
    hits: AtomicU64,
    misses: AtomicU64,
    damaged: AtomicU64,
    written: AtomicU64,
    written_bytes: AtomicU64,
    read_bytes: AtomicU64,
    since_trim: AtomicU64,
    temporaries: AtomicU64,
}

impl DiskStore {
    pub fn new(config: Config) -> DiskStore {
        DiskStore {
            folder: config.folder,
            limit: config.limit_bytes,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            damaged: AtomicU64::new(0),
            written: AtomicU64::new(0),
            written_bytes: AtomicU64::new(0),
            read_bytes: AtomicU64::new(0),
            // the first write looks at the folder: another process may have filled it
            since_trim: AtomicU64::new(TRIM_EVERY),
            temporaries: AtomicU64::new(0),
        }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// The entry's file; `None` for a space or key that cannot name one ([`entry::valid_key`]).
    fn path(&self, space: &str, key: &str) -> Option<PathBuf> {
        if !entry::valid_key(space) || !entry::valid_key(key) {
            return None;
        }
        let shard = key.get(..2).unwrap_or("00");
        Some(self.folder.join(space).join(shard).join(format!("{key}.{ENTRY_EXTENSION}")))
    }

    /// The data stored under `key`. A missing, cut or damaged entry is a miss, and a damaged one is removed.
    pub fn get(&self, space: &str, key: &str) -> Option<Vec<u8>> {
        let read = self.path(space, key).and_then(|path| Some((std::fs::read(&path).ok()?, path)));
        let Some((stored, path)) = read else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        // an entry unpacks to no more than the folder may hold
        let max = usize::try_from(self.limit).unwrap_or(usize::MAX);
        let Some(data) = entry::unpack(key, &stored, max) else {
            let _ = std::fs::remove_file(&path);
            self.damaged.fetch_add(1, Ordering::Relaxed);
            self.misses.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        // the entry's time is when it was last used, which is what eviction goes by
        if let Ok(file) = std::fs::File::options().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        self.hits.fetch_add(1, Ordering::Relaxed);
        self.read_bytes.fetch_add(stored.len() as u64, Ordering::Relaxed);
        Some(data)
    }

    /// Stores `data` under `key`, whole or not at all. Failing to store only costs a later computation, so it isn't an
    /// error: a full disk must not stop a run.
    pub fn put(&self, space: &str, key: &str, data: &[u8]) {
        let Some(path) = self.path(space, key) else { return };
        if data.len() as u64 > self.limit {
            return;
        }
        let stored = entry::pack(key, data);
        if stored.len() as u64 > self.limit {
            return;
        }
        let Some(dir) = path.parent() else { return };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let unique = self.temporaries.fetch_add(1, Ordering::Relaxed);
        let temporary = path.with_extension(format!("{}.{unique}.{TEMPORARY_EXTENSION}", std::process::id()));
        if std::fs::write(&temporary, &stored).is_err() || std::fs::rename(&temporary, &path).is_err() {
            let _ = std::fs::remove_file(&temporary);
            return;
        }
        self.written.fetch_add(1, Ordering::Relaxed);
        self.written_bytes.fetch_add(stored.len() as u64, Ordering::Relaxed);
        let since = self.since_trim.fetch_add(stored.len() as u64, Ordering::Relaxed) + stored.len() as u64;
        if since >= TRIM_EVERY.min(self.limit / TRIM_SHARE) {
            self.since_trim.store(0, Ordering::Relaxed);
            eviction::trim(&self.folder, self.limit, SystemTime::now());
        }
    }

    pub fn stats(&self) -> Stats {
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        Stats {
            hits: get(&self.hits),
            misses: get(&self.misses),
            damaged: get(&self.damaged),
            written: get(&self.written),
            written_bytes: get(&self.written_bytes),
            read_bytes: get(&self.read_bytes),
        }
    }

    /// The bytes in the folder and how many entries.
    pub fn usage(&self) -> (u64, usize) {
        let contents = eviction::scan(&self.folder);
        (contents.total(), contents.entries.len())
    }

    /// Removes everything the cache put in the folder.
    pub fn clear(&self) -> usize {
        let contents = eviction::scan(&self.folder);
        let mut removed = 0;
        for found in contents.entries.iter().chain(&contents.temporaries) {
            removed += usize::from(std::fs::remove_file(&found.path).is_ok());
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn store(tag: &str, limit: u64) -> DiskStore {
        let folder = std::env::temp_dir().join(format!("tc-result-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        DiskStore::new(Config { folder, limit_bytes: limit })
    }

    fn key(text: &str) -> String {
        text.bytes().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn what_is_put_is_got() {
        let s = store("round", 1 << 20);
        assert_eq!(s.get("a", &key("k")), None);
        s.put("a", &key("k"), b"data");
        assert_eq!(s.get("a", &key("k")).as_deref(), Some(&b"data"[..]));
        assert_eq!(s.get("b", &key("k")), None, "spaces are apart");
        assert_eq!(s.stats().hits, 1);
        assert_eq!(s.stats().misses, 2);
        assert_eq!(s.stats().written, 1);
        let _ = std::fs::remove_dir_all(s.folder());
    }

    #[test]
    fn a_cut_or_damaged_entry_is_a_miss_and_goes() {
        let s = store("damage", 1 << 20);
        s.put("a", &key("cut"), &[7u8; 1000]);
        s.put("a", &key("flip"), &[7u8; 1000]);
        let cut = s.path("a", &key("cut")).unwrap();
        let whole = std::fs::read(&cut).unwrap();
        std::fs::write(&cut, &whole[..whole.len() / 2]).unwrap();
        assert_eq!(s.get("a", &key("cut")), None);
        assert!(!cut.exists());
        let flip = s.path("a", &key("flip")).unwrap();
        let mut bytes = std::fs::read(&flip).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&flip, &bytes).unwrap();
        assert_eq!(s.get("a", &key("flip")), None);
        assert_eq!(s.stats().damaged, 2);
        // an entry moved under another key's name isn't trusted either
        s.put("a", &key("one"), b"one");
        let two = s.path("a", &key("two")).unwrap();
        std::fs::create_dir_all(two.parent().unwrap()).unwrap();
        std::fs::rename(s.path("a", &key("one")).unwrap(), &two).unwrap();
        assert_eq!(s.get("a", &key("two")), None);
        let _ = std::fs::remove_dir_all(s.folder());
    }

    #[test]
    fn the_folder_stays_under_its_limit_and_used_entries_stay() {
        let s = store("limit", 3 << 20);
        let random = |seed: u64| {
            let mut v = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
            (0..1 << 20)
                .map(|_| {
                    v ^= v << 13;
                    v ^= v >> 7;
                    v ^= v << 17;
                    v as u8
                })
                .collect::<Vec<u8>>()
        };
        s.put("a", &key("kept"), &random(1));
        for i in 2..12u64 {
            // a used entry gets a newer time than those not used
            std::thread::sleep(std::time::Duration::from_millis(15));
            assert!(s.get("a", &key("kept")).is_some() || i > 2);
            s.put("a", &key(&format!("e{i}")), &random(i));
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
        s.put("a", &key("last"), b"x");
        let (bytes, _) = s.usage();
        assert!(bytes <= 3 << 20, "{bytes}");
        assert!(s.get("a", &key("kept")).is_some(), "the entry in use all along stays");
        let _ = std::fs::remove_dir_all(s.folder());
    }

    #[test]
    fn keys_that_name_no_file_are_never_written() {
        let s = store("keys", 1 << 20);
        for (space, key) in [("a", "../x"), ("../a", "x"), ("a", ""), ("a/b", "x"), ("a", "x.y")] {
            s.put(space, key, b"data");
            assert_eq!(s.get(space, key), None, "{space} {key}");
        }
        assert_eq!(s.stats().written, 0);
        assert_eq!(s.usage(), (0, 0));
        let _ = std::fs::remove_dir_all(s.folder());
    }

    #[test]
    fn a_result_larger_than_the_limit_is_not_kept() {
        let s = store("big", 10);
        s.put("a", &key("k"), &[1u8; 100]);
        assert_eq!(s.stats().written, 0);
        let _ = std::fs::remove_dir_all(s.folder());
    }

    #[test]
    fn processes_and_threads_share_a_folder() {
        let s = Arc::new(store("share", 1 << 30));
        let data: Vec<u8> = (0..200_000u32).map(|v| (v >> 3) as u8).collect();
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (s, data) = (s.clone(), data.clone());
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        s.put("a", &key("same"), &data);
                        // a reader sees the whole entry or none, never part of one
                        if let Some(got) = s.get("a", &key("same")) {
                            assert_eq!(got, data);
                        }
                    }
                })
            })
            .collect();
        handles.into_iter().for_each(|h| h.join().unwrap());
        assert_eq!(s.stats().damaged, 0);
        assert_eq!(s.usage().1, 1);
        assert!(eviction::scan(s.folder()).temporaries.is_empty(), "no temporary stays behind");
        let _ = std::fs::remove_dir_all(s.folder());
    }

    #[test]
    fn clearing_leaves_nothing_of_ours() {
        let s = store("clear", 1 << 20);
        s.put("a", &key("k"), b"data");
        std::fs::write(s.folder().join("notes.txt"), b"mine").unwrap();
        assert_eq!(s.clear(), 1);
        assert_eq!(s.usage(), (0, 0));
        assert!(s.folder().join("notes.txt").exists());
        let _ = std::fs::remove_dir_all(s.folder());
    }

    #[test]
    fn an_unwritable_folder_is_only_a_cache_that_finds_nothing() {
        let file = std::env::temp_dir().join(format!("tc-result-store-file-{}", std::process::id()));
        std::fs::write(&file, b"not a folder").unwrap();
        let s = DiskStore::new(Config { folder: file.clone(), limit_bytes: 1 << 20 });
        s.put("a", &key("k"), b"data");
        assert_eq!(s.get("a", &key("k")), None);
        assert_eq!(s.stats().written, 0);
        let _ = std::fs::remove_file(&file);
    }
}
