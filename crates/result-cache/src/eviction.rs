// SPDX-License-Identifier: MIT OR Apache-2.0
//! Keeping the folder under its limit: entries used longest ago go first (a read sets an entry's time). Temporary files a
//! stopped process left behind go too, once they are old enough not to be another process's write in progress.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A temporary file this old belongs to nobody.
const STALE_TEMPORARY: Duration = Duration::from_secs(6 * 3600);
/// Eviction goes this far below the limit, so that it isn't needed again with the next entry.
const LOW_WATER_PERCENT: u64 = 90;

pub const ENTRY_EXTENSION: &str = "tcresult";
pub const TEMPORARY_EXTENSION: &str = "part";

#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    pub bytes: u64,
    pub used: SystemTime,
}

/// What is in the folder.
#[derive(Debug, Default)]
pub struct Contents {
    pub entries: Vec<Found>,
    pub temporaries: Vec<Found>,
}

impl Contents {
    pub fn total(&self) -> u64 {
        self.entries.iter().chain(&self.temporaries).map(|f| f.bytes).sum()
    }
}

/// Everything below `folder`. What can't be read counts as not there.
pub fn scan(folder: &Path) -> Contents {
    let mut out = Contents::default();
    let mut dirs = vec![folder.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(list) = std::fs::read_dir(&dir) else { continue };
        for entry in list.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                dirs.push(path);
                continue;
            }
            let found = Found { bytes: meta.len(), used: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), path: path.clone() };
            match path.extension().and_then(|e| e.to_str()) {
                Some(ENTRY_EXTENSION) => out.entries.push(found),
                Some(TEMPORARY_EXTENSION) => out.temporaries.push(found),
                _ => {}
            }
        }
    }
    out
}

/// What [`trim`] did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Trimmed {
    pub removed: u64,
    pub freed_bytes: u64,
    /// In the folder afterwards.
    pub total_bytes: u64,
}

/// Brings `folder` under `limit` bytes.
pub fn trim(folder: &Path, limit: u64, now: SystemTime) -> Trimmed {
    let mut contents = scan(folder);
    let mut total = contents.total();
    let mut done = Trimmed::default();
    let mut remove = |found: &Found, total: &mut u64| {
        // gone already is fine: another process trims too
        if std::fs::remove_file(&found.path).is_ok() {
            done.removed += 1;
            done.freed_bytes += found.bytes;
        }
        *total = total.saturating_sub(found.bytes);
    };
    for t in contents.temporaries.iter().filter(|t| now.duration_since(t.used).is_ok_and(|age| age > STALE_TEMPORARY)) {
        remove(t, &mut total);
    }
    if total > limit {
        let target = limit / 100 * LOW_WATER_PERCENT;
        contents.entries.sort_by_key(|e| e.used);
        for e in &contents.entries {
            if total <= target {
                break;
            }
            remove(e, &mut total);
        }
    }
    done.total_bytes = total;
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tc-eviction-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("space")).unwrap();
        dir
    }

    fn file(dir: &Path, name: &str, bytes: usize, age_secs: u64) -> PathBuf {
        let path = dir.join("space").join(name);
        std::fs::write(&path, vec![0u8; bytes]).unwrap();
        let time = SystemTime::now() - Duration::from_secs(age_secs);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(time).unwrap();
        path
    }

    #[test]
    fn the_oldest_go_first_and_below_the_limit_nothing_goes() {
        let dir = folder("order");
        let old = file(&dir, "a.tcresult", 400, 300);
        let middle = file(&dir, "b.tcresult", 400, 200);
        let new = file(&dir, "c.tcresult", 400, 100);
        assert_eq!(trim(&dir, 2000, SystemTime::now()).removed, 0);
        let done = trim(&dir, 1000, SystemTime::now());
        assert_eq!(done.removed, 1);
        assert_eq!(done.total_bytes, 800);
        assert!(!old.exists() && middle.exists() && new.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn eviction_leaves_room() {
        let dir = folder("room");
        for (i, age) in (1..=10).zip((1..=10).rev()) {
            file(&dir, &format!("{i}.tcresult"), 100, age * 10);
        }
        let done = trim(&dir, 950, SystemTime::now());
        assert!(done.total_bytes <= 900 / 100 * 90 + 100, "{done:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_old_temporaries_go() {
        let dir = folder("temporaries");
        let fresh = file(&dir, "a.1.part", 10, 60);
        let stale = file(&dir, "b.2.part", 10, 7 * 3600);
        let other = file(&dir, "notes.txt", 10, 7 * 3600);
        trim(&dir, 1000, SystemTime::now());
        assert!(fresh.exists() && !stale.exists() && other.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
