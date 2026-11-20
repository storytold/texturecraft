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
#[path = "pbr_map/mod.rs"]
pub mod pbr_maps;
pub mod presets;
pub mod project_file;
pub mod zip_read;

pub use export::{
    Dest, ExportInput, ExportOptions, ExportPreset, ExportedFile, FileFormat, MapOverride, MapSpec, Padding, PlannedFile, SourceRef, VirtualMap, list_outputs,
    render_maps, skip_repeated_names,
};
pub use image_res::{DecodedImage, coverage_png, decode_cooked_image, decode_image, decode_image_within, gray16_png, gray16_resource};
pub use mesh_export::{GlbTextures, NamedFiles, UsdExport, UsdFormat, glb_bytes, glb_export_options, obj_bytes, obj_with_mtl, usd_bytes, usd_export_options};
pub use mesh_import::{HighPolyMesh, ImportedMesh, MeshFile, MeshReport, import_high_poly_bytes, import_mesh_bytes, mesh_report, self_contained_mesh};
#[cfg(not(target_arch = "wasm32"))]
pub use presets::load_preset;
pub use presets::{DEFAULT_PRESET, builtin_preset, builtin_presets, preset_from_json, preset_to_json};
pub use project_file::{
    BakePaintData, DEFAULT_PROJECT_OPEN_LIMIT_GIB, MAX_RESOURCE_NAME_LEN, ProjectFile, RamReservation, load_project_bytes, project_open_limit_gib,
    sanitize_resource_name, save_project_bytes, set_project_open_limit_gib,
};

#[cfg(not(target_arch = "wasm32"))]
pub use export::write_files;
#[cfg(not(target_arch = "wasm32"))]
pub use mesh_import::{import_mesh, read_mesh_file};
#[cfg(not(target_arch = "wasm32"))]
pub use project_file::{
    DEFAULT_RAM_PERCENT, DEFAULT_RESERVED_CORES, available_save_resources, configured_ram_budget, load_project, save_project, save_ram_budget,
    set_save_resources,
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

/// Most texels an imported image resource may decode to by default (8192 × 8192).
/// Larger images are refused from their header before any pixels are decoded.
pub const MAX_RESOURCE_TEXELS: u64 = 1 << 26;

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

/// Create a new temporary file beside `path`, refusing to reuse an existing (possibly symlinked) name.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn create_temp(path: &std::path::Path) -> Result<(std::path::PathBuf, std::fs::File)> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("out");

    // Bound retries when stale files or concurrent writers occupy candidate names
    const MAX_TEMP_ATTEMPTS: u32 = 100;

    for attempt in 0..MAX_TEMP_ATTEMPTS {
        let temporary_path = parent.join(format!(".{name}.{}.{attempt}.tmp", std::process::id()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary_path) {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(FormatError::Io(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "could not create a temporary file")))
}

/// Write `bytes` to `path` atomically: a new temporary file first, then rename over the target.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let (temporary_path, mut file) = create_temp(path)?;
    let result = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);

    let result = result.and_then(|_| std::fs::rename(&temporary_path, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }

    Ok(result?)
}

#[cfg(all(test, unix))]
mod atomic_tests {
    use super::write_atomic;

    #[test]
    fn write_atomic_does_not_follow_a_planted_temp_symlink() -> std::io::Result<()> {
        let directory = std::env::temp_dir().join(format!("tc-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory)?;
        let target = directory.join("out.png");
        let victim = directory.join("victim.txt");
        std::fs::write(&victim, b"safe")?;
        std::os::unix::fs::symlink(&victim, directory.join("out.png.tmp"))?;

        write_atomic(&target, b"new").map_err(std::io::Error::other)?;

        assert_eq!(std::fs::read(&victim)?, b"safe");
        assert_eq!(std::fs::read(&target)?, b"new");

        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }
}
