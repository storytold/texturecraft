use texturecraft_formats::export::sanitize_file_stem;
use texturecraft_paint::presets::find_preset;

#[cfg(not(target_arch = "wasm32"))]
use super::BRUSH_EXT;
use super::UserBrush;

/// The user's brush presets, mirrored as `<name>.tcbrush` files in a folder on the desktop.
#[derive(Clone, Debug, Default)]
pub struct UserBrushes {
    pub list: Vec<UserBrush>,
    /// The folder mirroring `list`; None keeps the presets in memory only.
    pub dir: Option<std::path::PathBuf>,
}

fn same_name(first_name: &str, second_name: &str) -> bool {
    first_name.trim().eq_ignore_ascii_case(second_name.trim())
}

/// Two names with the same file stem would share a file.
fn file_key(name: &str) -> String {
    sanitize_file_stem(name.trim()).to_ascii_lowercase()
}

impl UserBrushes {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.list.iter().position(|preset| same_name(&preset.name, name))
    }

    /// `name` belongs to a built-in or to a user brush other than `skip_index` (or shares its file).
    pub(super) fn taken(&self, name: &str, skip_index: Option<usize>) -> bool {
        if find_preset(name).is_some() {
            return true;
        }

        let candidate_file_key = file_key(name);
        self.list
            .iter()
            .enumerate()
            .any(|(index, preset)| Some(index) != skip_index && (same_name(&preset.name, name) || file_key(&preset.name) == candidate_file_key))
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file(&self, name: &str) -> Option<std::path::PathBuf> {
        self.dir.as_ref().map(|directory| directory.join(format!("{}.{BRUSH_EXT}", sanitize_file_stem(name.trim()))))
    }

    /// The file a preset named `name` that was never written would have.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn file_of(&self, name: &str) -> Option<std::path::PathBuf> {
        self.file(name)
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn file_of(&self, _: &str) -> Option<std::path::PathBuf> {
        None
    }

    /// Mirror `brush` to the folder under its name's file and return that file; `old_file` is
    /// the file it had before (an overwrite or a rename), which goes once the new one is written.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn write(&self, brush: &UserBrush, old_file: Option<&std::path::Path>) -> std::result::Result<Option<std::path::PathBuf>, String> {
        let (Some(dir), Some(path)) = (&self.dir, self.file(&brush.name)) else {
            return Ok(None);
        };

        let bytes = serde_json::to_vec_pretty(brush).map_err(|error| error.to_string())?;
        std::fs::create_dir_all(dir).map_err(|error| format!("cannot create {}: {error}", dir.display()))?;

        if self.list.iter().any(|other| {
            other.file.as_ref().is_some_and(|source| {
                Some(source.as_path()) != old_file && source.file_name().zip(path.file_name()).is_some_and(|(existing, new)| existing.eq_ignore_ascii_case(new))
            })
        }) {
            return Err(format!("{} belongs to another brush preset", path.display()));
        }

        match std::fs::symlink_metadata(&path) {
            Ok(_)
                if old_file != Some(path.as_path())
                    && old_file.is_none_or(|old| old.canonicalize().ok().zip(path.canonicalize().ok()).is_none_or(|(old, new)| old != new)) =>
            {
                return Err(format!("{} already exists", path.display()));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("cannot check {}: {error}", path.display()));
            }
            _ => {}
        }

        let old_file = old_file.filter(|old| *old != path);

        // Rename first because case-only paths may alias on the same file
        let case_only = old_file.is_some_and(|old| old.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy()));
        if let Some(old_path) = old_file.filter(|_| case_only) {
            std::fs::rename(old_path, &path).map_err(|error| format!("cannot rename {}: {error}", old_path.display()))?;
        }

        if let Err(error) = texturecraft_formats::write_atomic(&path, &bytes) {
            if let Some(old_path) = old_file.filter(|_| case_only) {
                let _ = std::fs::rename(&path, old_path);
            }

            return Err(format!("cannot write {}: {error}", path.display()));
        }

        if let Some(old_path) = old_file.filter(|_| !case_only)
            && let Err(error) = remove_file(old_path)
        {
            let _ = remove_file(&path);
            return Err(error);
        }

        Ok(Some(path))
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn write(&self, _: &UserBrush, _: Option<&std::path::Path>) -> std::result::Result<Option<std::path::PathBuf>, String> {
        Ok(None)
    }

    /// Remove `brush`'s file (the one it was read from, else its name's file).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn delete(&self, brush: &UserBrush) -> std::result::Result<(), String> {
        match brush.file.clone().or_else(|| self.file(&brush.name)) {
            Some(path) => remove_file(&path),
            None => Ok(()),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn delete(&self, _: &UserBrush) -> std::result::Result<(), String> {
        Ok(())
    }
}

/// Remove `path`; a file that is already gone is fine, any other error is reported.
#[cfg(not(target_arch = "wasm32"))]
fn remove_file(path: &std::path::Path) -> std::result::Result<(), String> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(format!("cannot remove {}: {error}", path.display())),
        _ => Ok(()),
    }
}
