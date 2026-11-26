use std::cell::RefCell;
use std::path::{Path, PathBuf};

mod found;
mod origin;

pub(in crate::pbr_cmds) use found::Found;
use origin::Origin;

use super::*;

/// Regular files below `dir`, with bounded directory traversal and no silent truncation.
fn walk(root: &Path, dir: &Path, depth: usize, visited: &mut usize, files: &mut FileList) -> std::result::Result<(), String> {
    const MAX_DIRECTORY_DEPTH: usize = 4;
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(format!("directory depth exceeds the limit of {MAX_DIRECTORY_DEPTH}"));
    }

    let entries = std::fs::read_dir(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        *visited += 1;
        if *visited > MAX_FILES {
            return Err(format!("directory entry count exceeds the limit of {MAX_FILES}"));
        }

        let path = entry.map_err(|error| error.to_string())?.path();

        // Links and special files must not become import candidates
        let metadata = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.is_dir() {
            walk(root, &path, depth + 1, visited, files)?;
        } else if metadata.is_file()
            && let Ok(relative) = path.strip_prefix(root)
        {
            let name = relative.components().map(|component| component.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            files.push((name, metadata.len()));
        }
    }

    Ok(())
}

pub(super) fn find(cmd: &str, p: &Value) -> Result<Found> {
    if let Some(selected) = p.get("files").and_then(Value::as_array) {
        let mut paths = Vec::new();
        let mut list = FileList::new();
        if selected.len() > MAX_FILES {
            return Err(bad(cmd, format!("selected file count exceeds the limit of {MAX_FILES}")));
        }

        for v in selected {
            let Some(s) = v.as_str() else { return Err(bad(cmd, "`files` must be a list of paths")) };
            let path = PathBuf::from(s);
            let metadata = std::fs::symlink_metadata(&path).map_err(|e| bad(cmd, format!("cannot read {s}: {e}")))?;
            if !metadata.is_file() {
                continue;
            }

            let leaf = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

            // Names are leaf names; the same leaf twice keeps the first
            if list.iter().any(|(n, _)| *n == leaf) {
                continue;
            }

            list.push((leaf, metadata.len()));
            paths.push(path);
        }
        let name = common_name(&list.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>());
        return Ok(Found { source: Origin::Files(paths), files: list, name });
    }

    let path = strp(p, "path").filter(|s| !s.trim().is_empty()).ok_or_else(|| bad(cmd, "`path` (a folder or .zip) or `files` is required"))?;
    let metadata = std::fs::symlink_metadata(path).map_err(|e| bad(cmd, format!("cannot read {path}: {e}")))?;
    let stem = |s: &str| Path::new(s).file_stem().map(|n| n.to_string_lossy().replace(['_', '-'], " "));
    let dir_name = |s: &str| Path::new(s).file_name().map(|n| n.to_string_lossy().replace(['_', '-'], " "));
    if metadata.is_dir() {
        let root = Path::new(path).canonicalize().map_err(|e| bad(cmd, format!("cannot read {path}: {e}")))?;
        let mut files = FileList::new();
        let mut visited_entries = 0;
        walk(&root, &root, 0, &mut visited_entries, &mut files).map_err(|error| bad(cmd, format!("cannot scan {path}: {error}")))?;
        let group = pick_group(cmd, &files)?;
        let files: FileList = files.into_iter().filter(|(n, _)| n.strip_prefix(group.as_str()).is_some_and(|rest| !rest.contains('/'))).collect();
        let name = if group.is_empty() { dir_name(path) } else { dir_name(group.trim_end_matches('/')) }
            .or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()));
        let root = if group.is_empty() { root } else { root.join(group.trim_end_matches('/')) };
        let files = files.into_iter().map(|(n, s)| (n.rsplit('/').next().unwrap_or(&n).to_string(), s)).collect();
        return Ok(Found { source: Origin::Folder { root }, files, name });
    }

    if metadata.is_file() && path.to_ascii_lowercase().ends_with(".zip") {
        if metadata.len() > MAX_FILE_BYTES * 2 {
            return Err(bad(cmd, format!("{path} is too large")));
        }

        let bytes = found::read_regular(Path::new(path), MAX_FILE_BYTES * 2).map_err(|error| bad(cmd, format!("cannot read {path}: {error}")))?;
        let mut zip = texturecraft_formats::zip_read::ZipFiles::open(bytes).map_err(|e| bad(cmd, format!("{path} is not a readable zip: {e}")))?;
        let files = zip.list(MAX_FILES);
        let group = pick_group(cmd, &files)?;
        let files: FileList = files.into_iter().filter(|(n, _)| n.strip_prefix(group.as_str()).is_some_and(|rest| !rest.contains('/'))).collect();
        let name = if group.is_empty() { stem(path) } else { dir_name(group.trim_end_matches('/')) };
        return Ok(Found { source: Origin::Zip(RefCell::new(zip)), files, name });
    }

    Err(bad(cmd, format!("{path} is not a folder or a .zip file")))
}
