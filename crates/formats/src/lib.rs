//! # texturecraft-formats
//!
//! Everything TextureCraft reads from or writes to disk:
//!
//! - [`mesh_import`]: glTF/GLB, FBX (6.x and 7.x, binary and ASCII), COLLADA, USD (usda, usdc,
//!   usdz), Alembic (Ogawa), OBJ, PLY and STL into a
//!   [`texturecraft_geom::Mesh`] (world space in meters, Y up, glTF UV convention, one submesh
//!   per material), with an automatic unwrap ([`texturecraft_geom::unwrap::auto_unwrap`]) when
//!   the file has no UVs and a [`MeshReport`].
//! - [`mesh_export`]: the working mesh as OBJ, as GLB with its textures embedded, or as USD
//!   (usda / usdc with textures alongside, usdz with them packaged).
//! - [`export`]: texture export from a texture set's composite through [`ExportPreset`]s (file
//!   name templates, channel packing, virtual maps such as the final normal, padding, resizing)
//!   to PNG, JPEG, TGA, TIFF, BMP, WebP, EXR and HDR bytes.
//! - [`materialx`]: a MaterialX (`.mtlx`) OpenPBR document for an exported texture set.
//! - [`mcmat`]: MaterialCraft portable materials (`.mcmat`) and mesh maps as 16-bit PNGs for
//!   MaterialCraft graph inputs.
//! - [`presets`]: the built-in export presets and JSON (de)serialization of user presets.
//! - [`project_file`]: the `.tcraft` project zip (project JSON, sparse lossless pixel tiles,
//!   baked maps, the original mesh and a thumbnail).
//! - `fonts`: the installed fonts' families and styles (native only), for text alphas.
//!
//! Every reader takes untrusted bytes and returns an error, never panics. Byte-based functions
//! work everywhere (including the web build); path-based ones are native only.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod export;
#[cfg(not(target_arch = "wasm32"))]
pub mod fonts;
pub mod icc;
pub mod image_res;
pub mod materialx;
pub mod materialx_read;
pub mod mcmat;
pub mod mesh_export;
pub mod mesh_import;
pub mod presets;
pub mod project_file;

pub use export::{
    Dest, ExportInput, ExportOptions, ExportPreset, ExportedFile, FileFormat, MapSpec, Padding, PlannedFile, SourceRef, VirtualMap, list_outputs, render_maps,
};
pub use image_res::{DecodedImage, coverage_png, decode_image, decode_image_within};
pub use mesh_export::{GlbTextures, NamedFiles, UsdExport, UsdFormat, glb_bytes, glb_export_options, obj_bytes, obj_with_mtl, usd_bytes, usd_export_options};
pub use mesh_import::{HighPolyMesh, ImportedMesh, MeshFile, MeshReport, import_high_poly_bytes, import_mesh_bytes, mesh_report, self_contained_mesh};
#[cfg(not(target_arch = "wasm32"))]
pub use presets::load_preset;
pub use presets::{builtin_preset, builtin_presets, preset_from_json, preset_to_json};
pub use project_file::{
    BakePaintData, DEFAULT_PROJECT_OPEN_LIMIT_GIB, ProjectFile, load_project_bytes, project_open_limit_gib, sanitize_resource_name, save_project_bytes,
    set_project_open_limit_gib,
};

#[cfg(not(target_arch = "wasm32"))]
pub use export::write_files;
#[cfg(not(target_arch = "wasm32"))]
pub use mesh_import::{import_mesh, read_mesh_file};
#[cfg(not(target_arch = "wasm32"))]
pub use project_file::{
    DEFAULT_RAM_PERCENT, DEFAULT_RESERVED_CORES, available_save_resources, load_project, save_project, save_ram_budget, set_save_resources,
};

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("malformed {kind}: {msg}")]
    Malformed { kind: &'static str, msg: String },
    #[error("too large: {0}")]
    TooLarge(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("this file was saved by a newer TextureCraft (format {found}; this version reads up to {supported})")]
    NewerVersion { found: u32, supported: u32 },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Geom(#[from] texturecraft_geom::GeomError),
    #[error(transparent)]
    Core(#[from] texturecraft_core::CoreError),
}

pub type Result<T> = std::result::Result<T, FormatError>;

pub(crate) fn malformed(kind: &'static str, msg: impl std::fmt::Display) -> FormatError {
    FormatError::Malformed { kind, msg: msg.to_string() }
}

/// Last-resort guard around third-party parsers: a panic inside them becomes an error.
pub(crate) fn guard<T>(kind: &'static str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(malformed(kind, "the parser failed on this file")))
}

/// Largest image edge accepted or produced anywhere in this crate.
pub const MAX_IMAGE_SIZE: u32 = 16_384;

/// Read a whole file, refusing files over `cap` bytes.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn read_capped(path: &std::path::Path, cap: u64) -> Result<Vec<u8>> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(FormatError::Invalid(format!("{} is not a regular file", path.display())));
    }
    if meta.len() > cap {
        return Err(FormatError::TooLarge(format!("{} is {} bytes (limit {cap})", path.display(), meta.len())));
    }
    Ok(std::fs::read(path)?)
}

/// A short suffix that is different on every call and hard to guess: process id, a nanosecond clock reading and a counter.
/// Used to name temporary files and folders so another local user cannot plant a link at the name in advance.
#[cfg(not(target_arch = "wasm32"))]
pub fn unique_suffix() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    // Mix in the address of a stack local too (ASLR), so two processes started in the same nanosecond still differ.
    let local = 0u8;
    let aslr = std::ptr::addr_of!(local) as usize;
    format!("{:x}-{:x}-{:x}-{}", std::process::id(), nanos, aslr, COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// Write `bytes` to `path` atomically: a sibling temporary file first, then rename over the target.
///
/// The temporary file has an unpredictable name (`<path>.<suffix>.tmp`) and is created exclusively
/// (`create_new`, which never follows a link planted at that name), so a shared folder cannot be used to make the
/// write land on another file. It is removed again if anything fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{}.tmp", unique_suffix()));
    let tmp = std::path::PathBuf::from(tmp);
    let res = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.flush()))
        .and_then(|()| std::fs::rename(&tmp, path));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(res?)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod atomic_tests {
    use super::{unique_suffix, write_atomic};

    /// A file already sitting at the old, predictable `<path>.tmp` name is never written through, and no temporary file is left.
    #[test]
    fn write_atomic_ignores_a_planted_predictable_tmp_name() {
        let dir = std::env::temp_dir().join(format!("tc-write-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, b"precious").unwrap();
        // Stand-in for a planted link: a hard link to the victim at the predictable temporary name.
        std::fs::hard_link(&victim, dir.join("out.bin.tmp")).unwrap();
        write_atomic(&dir.join("out.bin"), b"new").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"precious", "the planted name was written through");
        assert_eq!(std::fs::read(dir.join("out.bin")).unwrap(), b"new");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp") || n == "out.bin.tmp"), "{names:?}");
        // A failed rename (target is a folder) removes its temporary file.
        std::fs::create_dir(dir.join("dst")).unwrap();
        assert!(write_atomic(&dir.join("dst"), b"x").is_err());
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.iter().filter(|n| n.ends_with(".tmp")).count(), 1, "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Suffixes differ on every call.
    #[test]
    fn unique_suffixes_differ() {
        let all: std::collections::HashSet<_> = (0..1000).map(|_| unique_suffix()).collect();
        assert_eq!(all.len(), 1000);
    }
}
