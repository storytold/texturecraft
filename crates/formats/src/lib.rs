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
    create_temp_with_mode(path, &std::collections::HashSet::new(), None)
}

#[cfg(not(target_arch = "wasm32"))]
fn create_temp_with_mode(
    path: &std::path::Path,
    excluded: &std::collections::HashSet<std::path::PathBuf>,
    #[cfg_attr(not(unix), allow(unused_variables))] unix_mode: Option<u32>,
) -> Result<(std::path::PathBuf, std::fs::File)> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("out");

    // Bound retries when stale files, batch targets, or concurrent writers occupy candidate names
    const MAX_TEMP_ATTEMPTS: u32 = 256;

    for attempt in 0..MAX_TEMP_ATTEMPTS {
        let temporary_path = parent.join(format!(".{name}.{}.{attempt}.tmp", std::process::id()));
        if excluded.contains(&temporary_path) {
            continue;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if let Some(mode) = unix_mode {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        match options.open(&temporary_path) {
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
    write_batch_atomic([(path, bytes)])
}

/// Write every `(path, bytes)` pair by staging and syncing all temporary files first (excluding
/// all batch targets), then renaming them into place one at a time. Concurrent readers can briefly
/// observe a mixed generation during the rename loop. Single-file batches rename directly without
/// creating a backup. For multi-file batches, if a later rename fails, earlier publications are
/// rolled back (restoring backups for overwritten targets and removing newly created targets), or
/// an explicit error is returned naming any incomplete rollback paths.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_batch_atomic<'a>(entries: impl IntoIterator<Item = (&'a std::path::Path, &'a [u8])>) -> Result<()> {
    write_batch_atomic_with_hook(entries, |_, _| Ok(()))
}

#[cfg(not(target_arch = "wasm32"))]
enum RollbackBackup {
    File(std::path::PathBuf),
    #[cfg(unix)]
    Symlink(std::path::PathBuf),
}

#[cfg(not(target_arch = "wasm32"))]
fn target_matches_bytes(target_path: &std::path::Path, expected_bytes: &[u8]) -> bool {
    use std::io::Read;

    let Ok(meta) = std::fs::symlink_metadata(target_path) else { return false };
    if !meta.file_type().is_file() {
        return false;
    }
    let Ok(file) = std::fs::File::open(target_path) else { return false };
    let Ok(handle_meta) = file.metadata() else { return false };
    if !handle_meta.file_type().is_file() {
        return false;
    }
    let mut buf = Vec::with_capacity(expected_bytes.len().min(4096));
    if file.take(expected_bytes.len() as u64 + 1).read_to_end(&mut buf).is_err() {
        return false;
    }
    buf.as_slice() == expected_bytes
}

#[cfg(not(target_arch = "wasm32"))]
fn write_batch_atomic_with_hook<'a>(
    entries: impl IntoIterator<Item = (&'a std::path::Path, &'a [u8])>,
    mut before_rename: impl FnMut(usize, &std::path::Path) -> std::io::Result<()>,
) -> Result<()> {
    use std::io::Write;

    // Serialize in-process batch publications so concurrent tasks cannot interleave backups/rollbacks.
    static BATCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = BATCH_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    let entries: Vec<(&std::path::Path, &[u8])> = entries.into_iter().collect();
    if entries.is_empty() {
        return Ok(());
    }
    let mut reserved: std::collections::HashSet<std::path::PathBuf> = entries.iter().map(|(path, _)| path.to_path_buf()).collect();
    let mut staged: Vec<(std::path::PathBuf, &std::path::Path, &[u8])> = Vec::with_capacity(entries.len());
    let cleanup_staged = |staged: &[(std::path::PathBuf, &std::path::Path, &[u8])]| {
        for (temp_path, _, _) in staged {
            let _ = std::fs::remove_file(temp_path);
        }
    };

    for (path, bytes) in entries {
        if path.is_dir() {
            cleanup_staged(&staged);
            return Err(FormatError::Io(std::io::Error::other(format!("{} is a directory", path.display()))));
        }
        let (temporary_path, mut file) = match create_temp_with_mode(path, &reserved, None) {
            Ok(pair) => pair,
            Err(error) => {
                cleanup_staged(&staged);
                return Err(error);
            }
        };
        reserved.insert(temporary_path.clone());
        let write_result = file.write_all(bytes).and_then(|_| file.sync_all());
        drop(file);
        if let Err(error) = write_result {
            let _ = std::fs::remove_file(&temporary_path);
            cleanup_staged(&staged);
            return Err(error.into());
        }
        staged.push((temporary_path, path, bytes));
    }

    // Single-file writes have no earlier publications to roll back, so rename directly.
    if staged.len() == 1 {
        let (ref temporary_path, target_path, _) = staged[0];
        if let Err(error) = before_rename(0, target_path).and_then(|()| std::fs::rename(temporary_path, target_path)) {
            let _ = std::fs::remove_file(temporary_path);
            return Err(error.into());
        }
        return Ok(());
    }

    let mut published: Vec<(&std::path::Path, &[u8], Option<RollbackBackup>)> = Vec::with_capacity(staged.len());
    let rollback = |published: &mut Vec<(&std::path::Path, &[u8], Option<RollbackBackup>)>,
                    remaining: &[(std::path::PathBuf, &std::path::Path, &[u8])],
                    cause: std::io::Error|
     -> FormatError {
        let mut incomplete = Vec::new();
        for (target_path, expected_bytes, backup) in published.drain(..).rev() {
            // Do not overwrite if an external writer modified the target after we published it.
            if !target_matches_bytes(target_path, expected_bytes) {
                if let Some(RollbackBackup::File(backup_path)) = backup {
                    incomplete.push(format!("{} (modified by another writer; backup at {})", target_path.display(), backup_path.display()));
                } else {
                    incomplete.push(format!("{} (modified by another writer)", target_path.display()));
                }
                continue;
            }
            match backup {
                Some(RollbackBackup::File(backup_path)) => {
                    if let Err(err) = std::fs::rename(&backup_path, target_path) {
                        incomplete.push(format!("{} (backup at {}): {err}", target_path.display(), backup_path.display()));
                    }
                }
                #[cfg(unix)]
                Some(RollbackBackup::Symlink(link_target)) => {
                    if let Err(err) = std::fs::remove_file(target_path).and_then(|()| std::os::unix::fs::symlink(&link_target, target_path)) {
                        incomplete.push(format!("{} (symlink -> {}): {err}", target_path.display(), link_target.display()));
                    }
                }
                None => {
                    if let Err(err) = std::fs::remove_file(target_path) {
                        incomplete.push(format!("{}: {err}", target_path.display()));
                    }
                }
            }
        }
        cleanup_staged(remaining);
        if incomplete.is_empty() {
            FormatError::Io(cause)
        } else {
            FormatError::Io(std::io::Error::other(format!("{cause}; incomplete rollback: {}", incomplete.join("; "))))
        }
    };

    for index in 0..staged.len() {
        let target_path = staged[index].1;
        let expected_bytes = staged[index].2;
        if let Err(error) = before_rename(index, target_path) {
            return Err(rollback(&mut published, &staged[index..], error));
        }
        let backup = match std::fs::symlink_metadata(target_path) {
            Ok(meta) => {
                #[cfg(unix)]
                if meta.file_type().is_symlink() {
                    match std::fs::read_link(target_path) {
                        Ok(link_target) => Some(RollbackBackup::Symlink(link_target)),
                        Err(error) => return Err(rollback(&mut published, &staged[index..], error)),
                    }
                } else if !meta.file_type().is_file() {
                    return Err(rollback(&mut published, &staged[index..], std::io::Error::other(format!("{} is not a regular file", target_path.display()))));
                } else {
                    match stage_file_backup(target_path, &meta, &mut reserved) {
                        Ok(path) => Some(RollbackBackup::File(path)),
                        Err(error) => return Err(rollback(&mut published, &staged[index..], error)),
                    }
                }
                #[cfg(not(unix))]
                {
                    if !meta.file_type().is_file() {
                        return Err(rollback(
                            &mut published,
                            &staged[index..],
                            std::io::Error::other(format!("{} is not a regular file", target_path.display())),
                        ));
                    }
                    match stage_file_backup(target_path, &meta, &mut reserved) {
                        Ok(path) => Some(RollbackBackup::File(path)),
                        Err(error) => return Err(rollback(&mut published, &staged[index..], error)),
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(rollback(&mut published, &staged[index..], error)),
        };

        if let Err(error) = std::fs::rename(&staged[index].0, target_path) {
            if let Some(RollbackBackup::File(ref backup_path)) = backup {
                let _ = std::fs::remove_file(backup_path);
            }
            return Err(rollback(&mut published, &staged[index..], error));
        }
        published.push((target_path, expected_bytes, backup));
    }

    for (_, _, backup) in published {
        if let Some(RollbackBackup::File(backup_path)) = backup {
            let _ = std::fs::remove_file(backup_path);
        }
    }

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn stage_file_backup(
    target_path: &std::path::Path,
    meta: &std::fs::Metadata,
    reserved: &mut std::collections::HashSet<std::path::PathBuf>,
) -> std::io::Result<std::path::PathBuf> {
    let (backup_path, mut backup_file) = create_temp_with_mode(target_path, reserved, Some(0o600)).map_err(|e| std::io::Error::other(e.to_string()))?;
    reserved.insert(backup_path.clone());
    let copy_result = (|| -> std::io::Result<()> {
        let mut src = std::fs::File::open(target_path)?;
        std::io::copy(&mut src, &mut backup_file)?;
        backup_file.set_permissions(meta.permissions())?;
        backup_file.sync_all()?;
        Ok(())
    })();
    drop(backup_file);
    if let Err(error) = copy_result {
        let _ = std::fs::remove_file(&backup_path);
        return Err(error);
    }
    Ok(backup_path)
}

#[cfg(test)]
mod batch_atomic_tests {
    use super::{write_batch_atomic, write_batch_atomic_with_hook};

    #[test]
    fn write_batch_atomic_publishes_nothing_on_failure() -> std::io::Result<()> {
        let directory = std::env::temp_dir().join(format!("tc-batch-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory)?;
        let existing = directory.join("existing.png");
        std::fs::write(&existing, b"old")?;
        let blocked = directory.join("blocked_dir");
        std::fs::create_dir_all(&blocked)?;

        let result = write_batch_atomic([(existing.as_path(), b"new".as_slice()), (blocked.as_path(), b"data".as_slice())]);
        assert!(result.is_err());
        assert_eq!(std::fs::read(&existing)?, b"old");
        assert!(std::fs::read_dir(&directory)?.all(|entry| !entry.unwrap().file_name().to_string_lossy().ends_with(".tmp")));

        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }

    #[test]
    fn write_batch_atomic_avoids_colliding_with_later_batch_target() -> std::io::Result<()> {
        let directory = std::env::temp_dir().join(format!("tc-batch-reserve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory)?;
        let temp_named_target = directory.join(format!(".a.png.{}.0.tmp", std::process::id()));
        let main_target = directory.join("a.png");

        write_batch_atomic([(main_target.as_path(), b"main-bytes".as_slice()), (temp_named_target.as_path(), b"temp-named-bytes".as_slice())])
            .map_err(std::io::Error::other)?;

        assert_eq!(std::fs::read(&main_target)?, b"main-bytes");
        assert_eq!(std::fs::read(&temp_named_target)?, b"temp-named-bytes");

        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }

    #[test]
    fn write_batch_atomic_rolls_back_earlier_publications_when_later_rename_fails() -> std::io::Result<()> {
        let directory = std::env::temp_dir().join(format!("tc-batch-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory)?;
        let existing = directory.join("existing.png");
        std::fs::write(&existing, b"old")?;
        let fresh = directory.join("fresh.png");
        let late = directory.join("late.png");

        let result = write_batch_atomic_with_hook(
            [(existing.as_path(), b"new".as_slice()), (fresh.as_path(), b"fresh-bytes".as_slice()), (late.as_path(), b"late-bytes".as_slice())],
            |index, target| {
                if index == 2 {
                    std::fs::create_dir_all(target)?;
                }
                Ok(())
            },
        );

        assert!(result.is_err());
        assert_eq!(std::fs::read(&existing)?, b"old");
        assert!(!fresh.exists());
        assert!(std::fs::read_dir(&directory)?.all(|entry| !entry.unwrap().file_name().to_string_lossy().ends_with(".tmp")));

        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod atomic_tests {
    use super::{write_atomic, write_batch_atomic_with_hook};

    #[test]
    fn write_atomic_does_not_follow_a_planted_temp_symlink() -> std::io::Result<()> {
        let directory = std::env::temp_dir().join(format!("tc-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory)?;
        let target = directory.join("out.png");
        let victim = directory.join("victim.txt");
        std::fs::write(&victim, b"safe")?;
        std::os::unix::fs::symlink(&victim, directory.join(format!(".out.png.{}.0.tmp", std::process::id())))?;

        write_atomic(&target, b"new").map_err(std::io::Error::other)?;

        assert_eq!(std::fs::read(&victim)?, b"safe");
        assert_eq!(std::fs::read(&target)?, b"new");

        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }

    #[test]
    fn write_batch_atomic_rolls_back_symlink_target_without_touching_referent() -> std::io::Result<()> {
        let directory = std::env::temp_dir().join(format!("tc-batch-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory)?;
        let referent = directory.join("referent.png");
        std::fs::write(&referent, b"referent-bytes")?;
        let link = directory.join("link.png");
        std::os::unix::fs::symlink(&referent, &link)?;
        let late = directory.join("late.png");

        let result = write_batch_atomic_with_hook([(link.as_path(), b"new".as_slice()), (late.as_path(), b"late".as_slice())], |index, target| {
            if index == 1 {
                std::fs::create_dir_all(target)?;
            }
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(std::fs::read_link(&link)?, referent);
        assert_eq!(std::fs::read(&referent)?, b"referent-bytes");

        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }
}
