//! Cooks moved out of memory into files of their own ([`super::GraphStore::spill_cooks`]): an import writes each
//! texture set's cooks out once the set is finished, so that a project's sets do not all hold their cooks at once.
//! A spilled cook is read back when something needs it (a composite, a save), exactly as it was ([`super::kept`]'s
//! bytes); the files go when the store's cooks go. Nothing spills in the web build.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

use super::Cooked;

/// What a spilled cook holds, without reading it: per output its component count and allocated tiles, and which
/// outputs are linear (what a project file's index needs).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CookOutline {
    pub outputs: BTreeMap<String, (usize, usize)>,
    pub linear: BTreeSet<String>,
}

impl CookOutline {
    pub fn of(cooked: &Cooked) -> CookOutline {
        CookOutline {
            outputs: cooked.outputs.iter().map(|(n, img)| (n.clone(), (img.comps(), img.allocated_count()))).collect(),
            linear: cooked.linear.clone(),
        }
    }
}

/// Where cooks can be read again when they are not held: a project's file keeps the cooks it was saved with, so a
/// document opened from it need not hold them all ([`super::GraphStore::read_cooks_from`]). A cook that cannot be
/// read (the file is gone or was replaced) is `None`: it is then rendered again when it is needed, the same pixels.
pub trait CookSource: Send + Sync {
    fn get(&self, key: &str) -> Option<Cooked>;
}

/// A source and the outlines of the cooks it holds, by key.
type Sourced = (std::sync::Arc<dyn CookSource>, HashMap<String, CookOutline>);

/// The cooks of one store that are not held in memory: the spilled ones (their folder, made at the first spill, and
/// outlines by key) and the ones its project file keeps.
#[derive(Default)]
pub struct Spill {
    #[cfg(not(target_arch = "wasm32"))]
    dir: Mutex<Option<std::path::PathBuf>>,
    outlines: Mutex<HashMap<String, CookOutline>>,
    source: Mutex<Option<Sourced>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Spill {
    /// Writes `cooked` out under `key`; false when it could not be (it then stays in memory).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn put(&self, key: &str, cooked: &Cooked) -> bool {
        if !texturecraft_core::is_key(key) {
            return false;
        }
        let Some(dir) = self.dir() else { return false };
        if std::fs::write(dir.join(key), super::kept::encode(cooked)).is_err() {
            return false;
        }
        lock(&self.outlines).insert(key.to_string(), CookOutline::of(cooked));
        true
    }

    #[cfg(target_arch = "wasm32")]
    pub fn put(&self, _key: &str, _cooked: &Cooked) -> bool {
        false
    }

    /// The cooks `source` holds (their outlines by key) can be read from it from now on.
    pub fn set_source(&self, source: std::sync::Arc<dyn CookSource>, outlines: HashMap<String, CookOutline>) {
        *lock(&self.source) = Some((source, outlines));
    }

    /// The cook spilled under `key` or kept by the source, read back.
    pub fn get(&self, key: &str) -> Option<Cooked> {
        if !lock(&self.outlines).contains_key(key) {
            let source = lock(&self.source).as_ref().filter(|(_, o)| o.contains_key(key)).map(|(s, _)| s.clone());
            return source.and_then(|s| s.get(key));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dir = lock(&self.dir).clone()?;
            std::fs::read(dir.join(key)).ok().and_then(|b| super::kept::decode(&b))
        }
        #[cfg(target_arch = "wasm32")]
        {
            None
        }
    }

    pub fn outline(&self, key: &str) -> Option<CookOutline> {
        let spilled = lock(&self.outlines).get(key).cloned();
        spilled.or_else(|| lock(&self.source).as_ref().and_then(|(_, o)| o.get(key).cloned()))
    }

    pub fn keys(&self) -> Vec<String> {
        let mut keys: BTreeSet<String> = lock(&self.outlines).keys().cloned().collect();
        keys.extend(lock(&self.source).iter().flat_map(|(_, o)| o.keys().cloned()));
        keys.into_iter().collect()
    }

    /// Forgets (and deletes) the cooks `keep` does not keep.
    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
        if let Some((_, o)) = lock(&self.source).as_mut() {
            o.retain(|k, _| keep(k));
        }
        let mut outlines = lock(&self.outlines);
        let gone: Vec<String> = outlines.keys().filter(|k| !keep(k)).cloned().collect();
        for k in gone {
            outlines.remove(&k);
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(dir) = lock(&self.dir).as_ref() {
                let _ = std::fs::remove_file(dir.join(&k));
            }
        }
    }

    /// The folder, made on first use in the system's temporary folder.
    #[cfg(not(target_arch = "wasm32"))]
    fn dir(&self) -> Option<std::path::PathBuf> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let mut dir = lock(&self.dir);
        if dir.is_none() {
            let root = spill_root();
            remove_orphans(&root);
            let d = root.join(format!("texturecraft-cooks-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&d).ok()?;
            *dir = Some(d);
        }
        dir.clone()
    }
}

/// Where spilled cooks go: the user's cache folder on Linux and the BSDs (their temporary folder is often held in RAM,
/// which spilling must not fill), the system's temporary folder elsewhere.
#[cfg(not(target_arch = "wasm32"))]
fn spill_root() -> std::path::PathBuf {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")));
        if let Some(dir) = cache.map(|c| c.join("texturecraft")).filter(|d| std::fs::create_dir_all(d).is_ok()) {
            return dir;
        }
    }
    std::env::temp_dir()
}

/// Removes the spill folders in `root` that processes no longer running left behind (stopped before they could clean
/// up). Only where a process's liveness can be read (Linux).
#[cfg(not(target_arch = "wasm32"))]
fn remove_orphans(root: &std::path::Path) {
    #[cfg(target_os = "linux")]
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid) = name.to_str().and_then(|n| n.strip_prefix("texturecraft-cooks-")).and_then(|r| r.split('-').next()) else { continue };
            if pid.parse::<u32>().is_ok() && !std::path::Path::new("/proc").join(pid).exists() {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = root;
}

impl Drop for Spill {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(dir) = lock(&self.dir).take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
