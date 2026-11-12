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

/// The spilled cooks of one store: their folder (made at the first spill) and outlines by key.
#[derive(Default)]
pub struct Spill {
    #[cfg(not(target_arch = "wasm32"))]
    dir: Mutex<Option<std::path::PathBuf>>,
    outlines: Mutex<HashMap<String, CookOutline>>,
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

    /// The cook spilled under `key`, read back.
    pub fn get(&self, key: &str) -> Option<Cooked> {
        if !lock(&self.outlines).contains_key(key) {
            return None;
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
        lock(&self.outlines).get(key).cloned()
    }

    pub fn keys(&self) -> Vec<String> {
        lock(&self.outlines).keys().cloned().collect()
    }

    /// Forgets (and deletes) the cooks `keep` does not keep.
    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
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
            let d = std::env::temp_dir().join(format!("texturecraft-cooks-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&d).ok()?;
            *dir = Some(d);
        }
        dir.clone()
    }
}

impl Drop for Spill {
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(dir) = lock(&self.dir).take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
