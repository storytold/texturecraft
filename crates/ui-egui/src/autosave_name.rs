//! Where a project's autosave copy goes. Each project has a file of its own, `<stem>-copy-<suffix>.tcraft`: a project's
//! name comes from its mesh file, so the name alone would let projects made from one mesh overwrite each other's copy

use std::path::{Path, PathBuf};

use texturecraft_engine::core::PROJECT_EXT;
use texturecraft_engine::formats::export::sanitize_file_stem;

use crate::TextureCraftApp;

/// Hex digits of a copy's suffix
const SUFFIX_DIGITS: usize = 16;

/// Between a copy's stem and its suffix, so a user's own file named like `report-20240115` is not taken for a copy
const COPY_MARK: &str = "-copy-";

/// The longest stem kept, so the suffix and the temporary files of a save still fit a file name
const MAX_STEM_CHARS: usize = 170;

impl TextureCraftApp {
    /// The open project's autosave file in `folder`. A saved project's copy is named after its file and stays the
    /// same across restarts; an unsaved project gets one that no other open project or running instance shares
    pub(crate) fn autosave_path(&mut self, folder: &Path) -> PathBuf {
        let Some(saved) = self.session.path.as_deref() else {
            let project = self.session.project_id;
            let suffix = match self.unsaved_autosave_suffix {
                Some((owner, suffix)) if owner == project => suffix,
                _ => {
                    let suffix = new_suffix(project);
                    self.unsaved_autosave_suffix = Some((project, suffix));
                    suffix
                }
            };
            return folder.join(file_name(&self.session.doc.project.name, suffix));
        };

        let saved = absolute(Path::new(saved));

        // A project opened from its autosave copy keeps writing there, rather than chaining suffixes
        if is_copy_in(&saved, &absolute(folder)) {
            return saved;
        }

        let stem = saved.file_stem().map_or_else(|| self.session.doc.project.name.clone(), |stem| stem.to_string_lossy().into_owned());
        folder.join(file_name(&stem, hash(saved.as_os_str().as_encoded_bytes())))
    }
}

fn file_name(stem: &str, suffix: u64) -> String {
    let stem: String = sanitize_file_stem(stem).chars().take(MAX_STEM_CHARS).collect();
    format!("{stem}{COPY_MARK}{suffix:0SUFFIX_DIGITS$x}.{PROJECT_EXT}")
}

/// `path` made absolute, as given when that fails
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `path` with links and, on Windows, the verbatim prefix resolved so two spellings of one folder compare equal; made absolute when that fails
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| absolute(path))
}

/// Whether `path` is an autosave copy of `folder`: a project file there whose name ends in the copy mark and a suffix
fn is_copy_in(path: &Path, folder: &Path) -> bool {
    let suffix = path.file_stem().and_then(|stem| stem.to_str()).and_then(|stem| stem.rsplit_once(COPY_MARK)).map(|(_, suffix)| suffix);
    path.parent().is_some_and(|parent| resolved(parent) == resolved(folder))
        && path.extension().is_some_and(|extension| extension == PROJECT_EXT)
        && suffix.is_some_and(|suffix| suffix.len() == SUFFIX_DIGITS && suffix.bytes().all(|digit| digit.is_ascii_hexdigit()))
}

/// A suffix for the unsaved project `project`: from this process and the moment, so another instance or a later run
/// takes a different one
fn new_suffix(project: u64) -> u64 {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let mut seed = Vec::new();
    seed.extend_from_slice(&std::process::id().to_le_bytes());
    seed.extend_from_slice(&now.to_le_bytes());
    seed.extend_from_slice(&project.to_le_bytes());
    hash(&seed)
}

/// 64-bit FNV-1a: the same bytes give the same suffix in every release, which the standard hasher does not promise.
/// Wide enough that two paths are very unlikely to share one
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::Session;
    use texturecraft_engine::geom::primitives;

    use super::*;

    fn project(name: &str) -> Session {
        Session::new_project(name, primitives::cube(1.0), "builtin:cube", None, 128).unwrap()
    }

    fn app(session: Session) -> TextureCraftApp {
        TextureCraftApp::new(&eframe::CreationContext::_new_kittest(egui::Context::default()), session)
    }

    fn name_of(path: &Path) -> &str {
        path.file_name().and_then(|name| name.to_str()).unwrap()
    }

    #[test]
    fn unsaved_projects_with_one_name_get_different_copies_that_stay_put() {
        let folder = Path::new("autosave");
        let mut first = app(project("hero"));
        let mut second = app(project("hero"));

        let (one, other) = (first.autosave_path(folder), second.autosave_path(folder));

        assert_ne!(one, other, "another instance never writes the same file");
        assert!(name_of(&one).starts_with("hero-") && name_of(&one).ends_with(".tcraft"), "{one:?}");
        assert_eq!(one.parent(), Some(folder));
        assert_eq!(first.autosave_path(folder), one, "the next autosave replaces this project's copy");

        first.session.switch_to(project("hero"));
        assert_ne!(first.autosave_path(folder), one, "a new project of the same name starts its own copy");
    }

    #[test]
    fn a_saved_project_names_its_copy_after_its_file_and_keeps_it_across_restarts() {
        let folder = Path::new("autosave");
        let copy_of = |saved: &str| {
            let mut app = app(project("hero"));
            app.session.path = Some(saved.to_string());
            app.autosave_path(folder)
        };

        let first = copy_of("work/Hero_v1.tcraft");
        let second = copy_of("work/Hero_v2.tcraft");
        let same_name_elsewhere = copy_of("backup/Hero_v1.tcraft");

        assert!(name_of(&first).starts_with("Hero_v1-"), "{first:?}");
        assert_ne!(first, second);
        assert_ne!(first, same_name_elsewhere, "files of one name in two folders keep two copies");
        assert_eq!(copy_of("work/Hero_v1.tcraft"), first, "a later run finds the same copy");
    }

    #[test]
    fn a_project_opened_from_its_copy_keeps_writing_to_it() {
        let folder = std::env::temp_dir().join("tc-autosave");
        let mut app = app(project("hero"));
        let copy = app.autosave_path(&folder);

        app.session.path = Some(copy.display().to_string());
        assert_eq!(app.autosave_path(&folder), copy);

        // A file there that is not a copy is the user's own: autosave leaves it alone
        let own = folder.join("hero.tcraft");
        app.session.path = Some(own.display().to_string());
        assert_ne!(app.autosave_path(&folder), own);

        let dated = folder.join("hero-20240115.tcraft");
        app.session.path = Some(dated.display().to_string());
        assert_ne!(app.autosave_path(&folder), dated);
    }

    #[test]
    fn suffixes_have_a_fixed_width_and_long_names_are_cut() {
        let narrow = file_name("hero", 1);
        let wide = file_name("hero", u64::MAX);
        assert_eq!(narrow.len(), wide.len(), "every suffix has the same width");
        assert!(file_name(&"a".repeat(500), 1).len() < 200);
    }
}
