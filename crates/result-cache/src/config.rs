// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where the cache lives and how large it may get.

use std::path::{Path, PathBuf};

/// Default limit of the folder, in GB.
pub const DEFAULT_LIMIT_GB: f64 = 20.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub folder: PathBuf,
    pub limit_bytes: u64,
}

impl Config {
    /// A cache in `folder` holding at most `limit_gb` GB ([`DEFAULT_LIMIT_GB`] when `None`; a negative, infinite or
    /// NaN limit counts as none given).
    pub fn new(folder: impl Into<PathBuf>, limit_gb: Option<f64>) -> Config {
        let gb = limit_gb.filter(|g| g.is_finite() && *g >= 0.0).unwrap_or(DEFAULT_LIMIT_GB);
        Config { folder: folder.into(), limit_bytes: (gb * (1u64 << 30) as f64) as u64 }
    }

    /// The default folder under the user's cache folder (`texturecraft/results`), from the environment `lookup`:
    /// `XDG_CACHE_HOME`, else `LOCALAPPDATA`, else `HOME/.cache`. `None` when none of them is set.
    pub fn default_folder(lookup: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
        let set = |name: &str| lookup(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let base = set("XDG_CACHE_HOME").or_else(|| set("LOCALAPPDATA")).or_else(|| set("HOME").map(|h| h.join(".cache")))?;
        Some(base.join("texturecraft").join("results"))
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn limits_and_the_default_folder() {
        assert_eq!(Config::new("/x", Some(2.0)).limit_bytes, 2 << 30);
        assert_eq!(Config::new("/x", None).limit_bytes, 20 << 30);
        assert_eq!(Config::new("/x", Some(f64::NAN)).limit_bytes, 20 << 30);
        assert_eq!(Config::new("/x", Some(-1.0)).limit_bytes, 20 << 30);
        let read = |pairs: &'static [(&'static str, &'static str)]| {
            Config::default_folder(&move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| OsString::from(v)))
        };
        assert_eq!(read(&[("HOME", "/h")]), Some(PathBuf::from("/h/.cache/texturecraft/results")));
        assert_eq!(read(&[("XDG_CACHE_HOME", "/c"), ("HOME", "/h")]), Some(PathBuf::from("/c/texturecraft/results")));
        assert_eq!(read(&[]), None);
    }
}
