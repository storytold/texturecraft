//! The import's result cache: graph cooks kept in a folder on disk (`texturecraft-result-cache`) and reused by the
//! next import that renders the same graph with the same values, inputs and side, whichever project it is in. Off
//! unless asked for (`file.importProject` `resultCache`, CLI `import --result-cache <dir>`); native builds only.
//!
//! A cook is found by its cook key (`compose::graphs::GraphStore::cook_key`: graph text, parameter values, seed,
//! outputs, inputs by content, render side and the evaluator versions of the node kinds the graph uses), so a kept
//! cook is the render's own pixels; a damaged or cut entry is a miss and is removed.

use serde_json::{Value, json};

use crate::Result;
use crate::commands::bad;

/// The cache an import is asked to use: its folder and limit.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultCache {
    pub folder: String,
    /// Limit in GB (`None`: the cache's default).
    pub limit_gb: Option<f64>,
}

impl ResultCache {
    /// From command params `resultCache` (a folder) and `resultCacheGb`; `None` when no folder is given.
    pub fn from_params(id: &str, p: &Value) -> Result<Option<ResultCache>> {
        let Some(folder) = p.get("resultCache").filter(|v| !v.is_null()) else {
            if p.get("resultCacheGb").is_some_and(|v| !v.is_null()) {
                return Err(bad(id, "`resultCacheGb` needs `resultCache` (a folder)"));
            }
            return Ok(None);
        };
        let folder = folder.as_str().map(str::trim).filter(|f| !f.is_empty()).ok_or_else(|| bad(id, "`resultCache` must be a folder path"))?;
        if cfg!(target_arch = "wasm32") {
            return Err(bad(id, "`resultCache` is not available in the browser"));
        }
        let limit_gb = match p.get("resultCacheGb").filter(|v| !v.is_null()) {
            None => None,
            Some(v) => Some(v.as_f64().filter(|g| g.is_finite() && *g >= 0.0).ok_or_else(|| bad(id, "`resultCacheGb` must be a number of GB, 0 or more"))?),
        };
        Ok(Some(ResultCache { folder: folder.to_string(), limit_gb }))
    }
}

/// An open cache: what the graph store keeps cooks in, and the folder's own counts.
#[cfg(not(target_arch = "wasm32"))]
pub struct Opened {
    pub kept: std::sync::Arc<dyn texturecraft_compose::graphs::KeptCooks>,
    store: std::sync::Arc<texturecraft_result_cache::DiskStore>,
}

#[cfg(target_arch = "wasm32")]
pub struct Opened;

/// The cooks' space in the folder: a new cook format starts a new one.
#[cfg(not(target_arch = "wasm32"))]
fn space() -> String {
    format!("cook{}", texturecraft_compose::graphs::kept::FORMAT)
}

#[cfg(not(target_arch = "wasm32"))]
struct Folder(std::sync::Arc<texturecraft_result_cache::DiskStore>);

#[cfg(not(target_arch = "wasm32"))]
impl texturecraft_compose::graphs::KeptCooks for Folder {
    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.0.get(&space(), key)
    }

    fn put(&self, key: &str, bytes: &[u8]) {
        self.0.put(&space(), key, bytes);
    }
}

impl ResultCache {
    /// Opens the folder (made when first written).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(&self) -> Opened {
        let config = texturecraft_result_cache::Config::new(&self.folder, self.limit_gb);
        let store = std::sync::Arc::new(texturecraft_result_cache::DiskStore::new(config));
        Opened { kept: std::sync::Arc::new(Folder(store.clone())), store }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn open(&self) -> Opened {
        Opened
    }
}

impl Opened {
    /// What this import did with the cache, for the command's reply.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn summary(&self) -> Value {
        let s = self.store.stats();
        json!({"folder": self.store.folder().display().to_string(), "found": s.hits, "missed": s.misses, "written": s.written, "damaged": s.damaged, "readBytes": s.read_bytes, "writtenBytes": s.written_bytes})
    }

    #[cfg(target_arch = "wasm32")]
    pub fn summary(&self) -> Value {
        json!(null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_are_checked() {
        let id = "file.importProject";
        assert_eq!(ResultCache::from_params(id, &json!({})).unwrap(), None);
        assert_eq!(ResultCache::from_params(id, &json!({"resultCache": null})).unwrap(), None);
        let c = ResultCache::from_params(id, &json!({"resultCache": "/tmp/x", "resultCacheGb": 2})).unwrap().unwrap();
        assert_eq!((c.folder.as_str(), c.limit_gb), ("/tmp/x", Some(2.0)));
        for p in [json!({"resultCache": ""}), json!({"resultCache": 3}), json!({"resultCache": "/x", "resultCacheGb": -1}), json!({"resultCacheGb": 1})] {
            assert!(ResultCache::from_params(id, &p).is_err(), "{p}");
        }
    }
}
