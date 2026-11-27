//! App-owned nightly profile fences; no stable fallback, migration or process environment changes.
//!
//! A Nightly profile has to hold *all* of its user state: eframe's `app.ron`/`ui.json`, the error
//! log, session logs, autosave copies, smart materials, layouts, templates, brush and export
//! presets. Anything that resolves to a stable folder would read and prune Stable's files, so the
//! root is refused when it overlaps one of them, when an entry inside it is an alias that leaves it,
//! or when it is not writable.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// The folder names the launch checks use for this app in each platform's location.
const BRAND: &str = "TextureCraft";
/// The bundle identifier of the stable package (macOS library folders, the Linux Flatpak).
const STABLE_ID: &str = "ai.storyteller.texturecraft";
/// The package id of the Nightly Flatpak bundle (`packaging/linux/flatpak-bundle.sh`). Inside that
/// sandbox `FLATPAK_ID` is always set, so this exact id is the one a Nightly native launch accepts.
const NIGHTLY_ID: &str = "ai.storyteller.texturecraft.nightly";
/// eframe lowercases its folder name on Linux (`file_storage.rs`), so Stable's real settings folder
/// is `$XDG_DATA_HOME/texturecraft`, not the capitalised `TextureCraft` the app is called.
const LINUX_STORAGE_BRAND: &str = "texturecraft";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Windows,
    Linux,
    Mac,
}

impl Platform {
    fn current() -> Result<Self, String> {
        match std::env::consts::OS {
            "windows" => Ok(Self::Windows),
            "linux" => Ok(Self::Linux),
            "macos" => Ok(Self::Mac),
            _ => Err("nightly-v1 is not defined for this operating system".into()),
        }
    }

    fn ignores_case(self) -> bool {
        self != Self::Linux
    }
}

fn absolute(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(format!("{label} must be absolute and contain no parent traversal"));
    }
    Ok(path)
}

fn required_base(env: &impl Fn(&str) -> Option<OsString>, key: &str) -> Result<PathBuf, String> {
    let value = env(key).filter(|v| !v.is_empty()).ok_or_else(|| format!("cannot establish the stable {key} boundary"))?;
    absolute(PathBuf::from(value), key)
}

fn xdg_base(env: &impl Fn(&str) -> Option<OsString>, key: &str, fallback: &str) -> Result<PathBuf, String> {
    match env(key).filter(|v| !v.is_empty()) {
        Some(value) => absolute(PathBuf::from(value), key),
        None => Ok(required_base(env, "HOME")?.join(fallback)),
    }
}

/// Resolve existing ancestors before creating anything. A dangling link or an inaccessible ancestor
/// is not a missing directory that may be replaced or followed later.
fn canonical_future(path: &Path) -> Result<PathBuf, String> {
    let mut parent = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::symlink_metadata(parent) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(parent.file_name().ok_or("the profile root has no existing ancestor")?.to_os_string());
                parent = parent.parent().ok_or("the profile root has no existing ancestor")?;
            }
            Err(error) => return Err(format!("cannot inspect the profile boundary: {error}")),
        }
    }
    let mut resolved = std::fs::canonicalize(parent).map_err(|e| format!("cannot resolve the profile boundary: {e}"))?;
    for part in missing.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}

/// Whether one path is the other or contains it (in either direction), compared component by
/// component and folded the way the platform's filesystem folds names. `TextureCraft2` is not
/// inside `TextureCraft`.
fn overlaps(a: &Path, b: &Path, platform: Platform) -> bool {
    let parts = |p: &Path| {
        p.components()
            .map(|c| {
                let value = c.as_os_str().to_string_lossy().into_owned();
                if platform.ignores_case() { value.to_lowercase() } else { value }
            })
            .collect::<Vec<_>>()
    };
    let (a, b) = (parts(a), parts(b));
    a.starts_with(&b) || b.starts_with(&a)
}

/// The running install: never let the profile live inside it (it would be replaced or removed by
/// the next update).
fn installation_boundary(platform: Platform, exe: &Path) -> Result<PathBuf, String> {
    let parent = exe.parent().ok_or("cannot establish the installation boundary")?;
    if platform == Platform::Mac {
        // A profile in Resources is still inside the versioned bundle, not beside MacOS.
        if let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("app"))) {
            return Ok(bundle.to_path_buf());
        }
    }
    Ok(parent.to_path_buf())
}

/// The default data folder, `Documents/TextureCraft`, as the app's own resolver builds it
/// (`crate::data_folder("")`): `USERPROFILE` on Windows and `HOME` elsewhere, then `Documents`.
fn default_data_folder(platform: Platform, env: &impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, String> {
    let home = match platform {
        Platform::Windows => required_base(env, "USERPROFILE")?,
        _ => required_base(env, "HOME")?,
    };
    Ok(home.join("Documents").join(BRAND))
}

fn forbidden_roots(platform: Platform, env: &impl Fn(&str) -> Option<OsString>, exe: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = match platform {
        Platform::Windows => {
            // eframe's settings folder (`%APPDATA%\TextureCraft`), its local counterpart, and the MSI's
            // per-machine and any per-user install folders. A custom install folder is covered by the
            // executable's own folder below.
            let mut roots = ["APPDATA", "LOCALAPPDATA"].into_iter().map(|key| required_base(env, key).map(|p| p.join(BRAND))).collect::<Result<Vec<_>, _>>()?;
            roots.push(required_base(env, "LOCALAPPDATA")?.join("Programs").join("ArtCraft").join(BRAND));
            for key in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
                if let Some(base) = env(key).filter(|v| !v.is_empty()) {
                    roots.push(absolute(PathBuf::from(base), key)?.join(BRAND));
                }
            }
            roots
        }
        Platform::Linux => {
            // The four XDG roots carry eframe's settings, caches and state under the app's name.
            let mut roots =
                [("XDG_CONFIG_HOME", ".config"), ("XDG_DATA_HOME", ".local/share"), ("XDG_CACHE_HOME", ".cache"), ("XDG_STATE_HOME", ".local/state")]
                    .into_iter()
                    .map(|(key, fallback)| xdg_base(env, key, fallback).map(|p| p.join(BRAND)))
                    .collect::<Result<Vec<_>, _>>()?;
            // eframe's own folder for this app on Linux, which is what holds Stable's `app.ron`,
            // `ui.json` and `errors.log`: it folds the name, so it is the lowercase one.
            roots.push(xdg_base(env, "XDG_DATA_HOME", ".local/share")?.join(LINUX_STORAGE_BRAND));
            roots.extend(["/usr/bin/texturecraft", "/usr/local/bin/texturecraft", "/opt/texturecraft"].map(PathBuf::from));
            // The stable Flatpak keeps its own per-app data folder.
            roots.push(required_base(env, "HOME")?.join(format!(".var/app/{STABLE_ID}")));
            roots
        }
        Platform::Mac => {
            let home = required_base(env, "HOME")?;
            let library = home.join("Library");
            vec![
                library.join("Application Support").join(BRAND),
                library.join("Caches").join(BRAND),
                library.join("Caches").join(STABLE_ID),
                library.join("Preferences").join(BRAND),
                library.join("Preferences").join(format!("{STABLE_ID}.plist")),
                library.join("Saved Application State").join(format!("{STABLE_ID}.savedState")),
                home.join("Applications").join(format!("{BRAND}.app")),
                PathBuf::from("/Applications").join(format!("{BRAND}.app")),
            ]
        }
    };
    // The user's own documents folder is Stable's default data folder on every platform; a profile
    // there would share autosaves, smart materials and presets with it.
    roots.push(default_data_folder(platform, env)?);
    roots.push(installation_boundary(platform, exe)?);
    roots.into_iter().map(|p| canonical_future(&p)).collect()
}

/// An entry inside the profile must resolve inside it, so a link planted there cannot point
/// Nightly's writes at another app's files.
fn private_subdirectory(root: &Path, name: impl AsRef<Path>, label: &str) -> Result<PathBuf, String> {
    let chosen = canonical_future(&absolute(root.to_path_buf(), "the nightly profile root")?)?;
    let child = canonical_future(&chosen.join(name))?;
    if child == chosen || !child.starts_with(&chosen) {
        return Err(format!("the nightly {label} must stay inside its selected profile"));
    }
    Ok(child)
}

/// Every existing entry directly in the profile (autosave, logs, library and so on) must resolve
/// inside it.
fn entries_stay_inside(root: &Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("cannot inspect the nightly profile: {error}")),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot inspect the nightly profile: {e}"))?;
        private_subdirectory(root, entry.file_name(), "profile entry")?;
    }
    Ok(())
}

/// The app's writable probe: create a file exclusively in the profile and remove it again.
pub fn ensure_writable(dir: &Path) -> std::io::Result<()> {
    let probe = dir.join("exclusive-write-probe");
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe)?;
    drop(file);
    std::fs::remove_file(probe)
}

/// Refuse a stable folder, the running install or an alias, create a missing root with owner-only
/// access where the platform has it, and return the resolved root to use.
pub fn validate(
    root: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    exe: Option<&Path>,
    writable: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    validate_for(Platform::current()?, root, env, exe, writable)
}

fn validate_for(
    platform: Platform,
    root: &Path,
    env: impl Fn(&str) -> Option<OsString>,
    exe: Option<&Path>,
    writable: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PathBuf, String> {
    absolute(root.to_path_buf(), "the nightly profile root")?;
    // A native package must not carry another app's Flatpak namespace, and the Nightly bundle must
    // be able to start inside its own sandbox, where the runtime always sets `FLATPAK_ID`. Its
    // `~/.var/app/<nightly id>` sandbox is separate from stable's, so only other ids are refused.
    if platform != Platform::Windows
        && let Some(id) = env("FLATPAK_ID").filter(|v| !v.is_empty())
        && id != NIGHTLY_ID
    {
        return Err("nightly-v1 only runs inside its own Flatpak; launch through the native Launcher or the Nightly bundle".into());
    }
    let chosen = canonical_future(root)?;
    let exe = absolute(exe.ok_or("cannot establish the installation boundary")?.to_path_buf(), "the executable")?;
    let forbidden = forbidden_roots(platform, &env, &exe)?;
    if forbidden.iter().any(|stable| overlaps(&chosen, stable, platform)) {
        return Err("the nightly root must not overlap stable data or an installation".into());
    }
    // Existing aliases under the profile must not redirect any of its folders elsewhere.
    entries_stay_inside(&chosen)?;
    let mut create = std::fs::DirBuilder::new();
    create.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        create.mode(0o700);
    }
    create.create(&chosen).map_err(|e| format!("cannot create the nightly profile: {e}"))?;
    let confirmed = std::fs::canonicalize(&chosen).map_err(|e| format!("cannot resolve the created nightly profile: {e}"))?;
    if confirmed != chosen || forbidden.iter().any(|stable| overlaps(&confirmed, stable, platform)) {
        return Err("the nightly profile changed while it was being validated".into());
    }
    entries_stay_inside(&confirmed)?;
    writable(&confirmed).map_err(|e| format!("the nightly profile is not writable: {e}"))?;
    Ok(confirmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture {
        root: PathBuf,
        env: BTreeMap<String, OsString>,
        exe: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = loop {
                let candidate =
                    std::env::temp_dir().join(format!("texturecraft-profile-policy-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
                match std::fs::create_dir(&candidate) {
                    Ok(()) => break std::fs::canonicalize(candidate).unwrap(),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create owned fixture: {error}"),
                }
            };
            let home = root.join("home");
            let exe = root.join("install/texturecraft");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(&exe, b"fixture executable, never run").unwrap();
            let env = [("HOME", home.clone()), ("USERPROFILE", home), ("APPDATA", root.join("roaming")), ("LOCALAPPDATA", root.join("local"))]
                .into_iter()
                .map(|(key, value)| (key.into(), value.into_os_string()))
                .collect();
            Self { root, env, exe }
        }

        fn get(&self, key: &str) -> Option<OsString> {
            self.env.get(key).cloned()
        }

        fn check(&self, platform: Platform, root: &Path) -> Result<PathBuf, String> {
            validate_for(platform, root, |key| self.get(key), Some(&self.exe), ensure_writable)
        }

        fn home(&self) -> PathBuf {
            PathBuf::from(self.get("HOME").unwrap())
        }

        fn profile(&self, name: &str) -> PathBuf {
            self.root.join("profiles").join(name)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only the directory this fixture created is owned for removal.
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn a_fresh_private_profile_is_created_and_returned_resolved() {
        let fixture = Fixture::new();
        let profile = fixture.profile("nightly");
        for platform in [Platform::Windows, Platform::Linux, Platform::Mac] {
            assert_eq!(fixture.check(platform, &profile).unwrap(), profile);
            assert!(profile.is_dir());
            std::fs::remove_dir_all(&profile).ok();
        }
    }

    #[test]
    fn every_stable_data_folder_of_each_platform_is_refused_before_any_probe() {
        let fixture = Fixture::new();
        let home = fixture.home();
        let candidates: Vec<(Platform, PathBuf)> = vec![
            (Platform::Windows, PathBuf::from(fixture.get("APPDATA").unwrap()).join(BRAND)),
            (Platform::Windows, PathBuf::from(fixture.get("LOCALAPPDATA").unwrap()).join(BRAND)),
            (Platform::Windows, PathBuf::from(fixture.get("LOCALAPPDATA").unwrap()).join("Programs/ArtCraft").join(BRAND)),
            (Platform::Windows, home.join("Documents").join(BRAND)),
            (Platform::Linux, home.join(".config").join(BRAND)),
            (Platform::Linux, home.join(".local/share").join(BRAND)),
            (Platform::Linux, home.join(".local/share").join(LINUX_STORAGE_BRAND)),
            (Platform::Linux, home.join(".cache").join(BRAND)),
            (Platform::Linux, home.join(".local/state").join(BRAND)),
            (Platform::Linux, home.join("Documents").join(BRAND)),
            (Platform::Linux, PathBuf::from("/usr/bin/texturecraft")),
            (Platform::Linux, PathBuf::from("/opt/texturecraft")),
            (Platform::Linux, home.join(format!(".var/app/{STABLE_ID}"))),
            (Platform::Mac, home.join("Library/Application Support").join(BRAND)),
            (Platform::Mac, home.join("Library/Caches").join(BRAND)),
            (Platform::Mac, home.join("Library/Preferences").join(format!("{STABLE_ID}.plist"))),
            (Platform::Mac, home.join("Library/Saved Application State").join(format!("{STABLE_ID}.savedState"))),
            (Platform::Mac, home.join("Documents").join(BRAND)),
            (Platform::Mac, PathBuf::from(format!("/Applications/{BRAND}.app"))),
        ];
        for (platform, stable) in candidates {
            for candidate in [stable.clone(), stable.join("nested nightly")] {
                let probed = std::cell::Cell::new(false);
                let result = validate_for(
                    platform,
                    &candidate,
                    |key| fixture.get(key),
                    Some(&fixture.exe),
                    |_| {
                        probed.set(true);
                        Ok(())
                    },
                );
                assert!(result.is_err(), "{platform:?} accepted {}", candidate.display());
                assert!(!probed.get(), "a refused root must not be probed: {}", candidate.display());
            }
        }
    }

    #[test]
    fn the_running_install_is_fenced_on_every_platform() {
        let fixture = Fixture::new();
        for platform in [Platform::Windows, Platform::Linux, Platform::Mac] {
            let inside = fixture.exe.parent().unwrap().join("profile");
            assert!(fixture.check(platform, &inside).is_err(), "{platform:?} let the profile inside the install");
        }
    }

    #[test]
    fn lookalike_names_are_not_overlaps_but_case_folding_is() {
        let fixture = Fixture::new();
        let documents = fixture.home().join("Documents");
        assert!(fixture.check(Platform::Linux, &documents.join("TextureCraft2")).is_ok(), "a neighbor with a suffix is a different folder");
        assert!(fixture.check(Platform::Linux, &documents.join("texturecraft")).is_ok(), "Linux folds names exactly");
        for platform in [Platform::Windows, Platform::Mac] {
            assert!(fixture.check(platform, &documents.join("texturecraft")).is_err(), "{platform:?} must fold case");
        }
    }

    #[test]
    fn relative_parent_traversal_and_empty_roots_are_refused() {
        let fixture = Fixture::new();
        for root in [PathBuf::from("relative/profile"), PathBuf::from("/nightly/../stable"), PathBuf::from("")] {
            assert!(fixture.check(Platform::Linux, &root).is_err(), "{}", root.display());
        }
    }

    #[test]
    fn only_the_nightly_flatpak_may_start_with_its_own_id() {
        let fixture = Fixture::new();
        let profile = fixture.profile("flatpak");
        assert!(fixture.check(Platform::Linux, &profile).is_ok());
        std::fs::remove_dir_all(&profile).ok();
        let check_with = |id: &str, candidate: &Path| {
            let id = OsString::from(id);
            validate_for(
                Platform::Linux,
                candidate,
                |key| if key == "FLATPAK_ID" { Some(id.clone()) } else { fixture.get(key) },
                Some(&fixture.exe),
                ensure_writable,
            )
        };
        // Stable's id, its window id and an unrelated app's id are all refusals.
        for id in [STABLE_ID, "ai.storyteller.texturecraft.nightly-v1", "org.example.other"] {
            assert!(check_with(id, &profile).is_err(), "{id} was accepted");
        }
        // The Nightly bundle's own id is the sandbox the Nightly package runs in: accepted, with the
        // profile its wrapper names inside it (`${XDG_DATA_HOME}/TextureCraft-Nightly`).
        let sandbox = fixture.home().join(format!(".var/app/{NIGHTLY_ID}"));
        let sandboxed = sandbox.join(".local/share/TextureCraft-Nightly");
        assert_eq!(check_with(NIGHTLY_ID, &sandboxed).expect("the Nightly Flatpak may start"), sandboxed);
        std::fs::remove_dir_all(&sandbox).ok();
    }

    #[test]
    #[cfg(unix)]
    fn an_entry_that_leaves_the_profile_is_refused() {
        let fixture = Fixture::new();
        let profile = fixture.profile("aliased");
        let outside = fixture.root.join("somewhere else");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, profile.join("autosave")).unwrap();
        assert!(fixture.check(Platform::Linux, &profile).is_err(), "a link out of the profile must be refused");
        // A dangling link is not a missing folder that may be created.
        std::fs::remove_file(profile.join("autosave")).unwrap();
        std::os::unix::fs::symlink(profile.join("missing"), profile.join("logs")).unwrap();
        assert!(fixture.check(Platform::Linux, &profile).is_err());
        std::fs::remove_file(profile.join("logs")).unwrap();
        // A plain folder inside it is fine.
        std::fs::create_dir(profile.join("autosave")).unwrap();
        assert!(fixture.check(Platform::Linux, &profile).is_ok());
    }

    #[test]
    fn a_failed_write_probe_never_falls_back_to_stable() {
        let fixture = Fixture::new();
        let profile = fixture.profile("read only");
        let reached = std::cell::Cell::new(false);
        let result = validate_for(
            Platform::Linux,
            &profile,
            |key| fixture.get(key),
            Some(&fixture.exe),
            |_| {
                reached.set(true);
                Err(std::io::Error::other("read-only filesystem"))
            },
        );
        assert!(result.is_err());
        assert!(reached.get());
        assert!(result.unwrap_err().contains("not writable"));
        // The app must not then use the stable folder: it simply refuses to start.
        assert!(fixture.check(Platform::Linux, &fixture.home().join("Documents").join(BRAND)).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn a_profile_whose_existing_ancestor_is_a_link_resolves_before_the_fence() {
        let fixture = Fixture::new();
        let real = fixture.root.join("real profiles");
        std::fs::create_dir_all(&real).unwrap();
        let link = fixture.root.join("linked");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // A link to a private folder is fine, and the resolved root is what the app uses.
        let resolved = fixture.check(Platform::Linux, &link.join("nightly")).unwrap();
        assert_eq!(resolved, real.join("nightly"));
    }
}
