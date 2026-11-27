//! The build-time channel, which is distinct from a launch selector: no runtime environment can
//! promote a stable build into the Nightly one.

use crate::launch_profile::Profile;
use std::ffi::OsString;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildFlavor {
    Stable,
    NightlyV1,
}

/// `TEXTURECRAFT_BUILD_CHANNEL=nightly-v1` at compile time (`build.rs`).
pub const COMPILED: BuildFlavor = if cfg!(texturecraft_nightly_v1) { BuildFlavor::NightlyV1 } else { BuildFlavor::Stable };

/// The version this binary was built as: the prepared feed identity in a nightly build, else the
/// workspace version (`build.rs` bakes it so both flavors read it the same way).
pub const BUILT_VERSION: &str = env!("TEXTURECRAFT_VERSION");

/// A nightly build starts only with its profile, and a stable build only without one.
pub fn require_profile(flavor: BuildFlavor, profile: &Profile) -> Result<(), String> {
    match (flavor, profile.is_nightly()) {
        (BuildFlavor::NightlyV1, false) => Err("Open TextureCraft Nightly from ArtCraft Launcher".into()),
        (BuildFlavor::Stable, true) => Err("This is a stable TextureCraft build; nightly-v1 requires the nightly build".into()),
        _ => Ok(()),
    }
}

/// A refused nightly start explains itself in a native dialog: a release Windows build has no
/// console, and a Dock launch on macOS has none either.
pub fn needs_native_error(flavor: BuildFlavor, desktop_session: bool) -> bool {
    desktop_session && flavor == BuildFlavor::NightlyV1
}

/// `--version` or `-V` alone; a file called `--version` after `--` is never a query.
pub fn version_query(args: &[OsString]) -> bool {
    matches!(args, [arg] if arg == "--version" || arg == "-V")
}

/// The line `--version` prints: `<App> <version> (nightly-v1)` in a nightly build, exactly as the
/// Launcher and the release smoke test read it.
pub fn version(flavor: BuildFlavor) -> String {
    match flavor {
        BuildFlavor::Stable => format!("TextureCraft {BUILT_VERSION}"),
        BuildFlavor::NightlyV1 => format!("TextureCraft {BUILT_VERSION} (nightly-v1)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_nightly_desktop_refusal_requests_a_native_dialog() {
        assert!(needs_native_error(BuildFlavor::NightlyV1, true));
        assert!(!needs_native_error(BuildFlavor::NightlyV1, false));
        assert!(!needs_native_error(BuildFlavor::Stable, true));
        assert!(!needs_native_error(BuildFlavor::Stable, false));
    }

    #[test]
    fn nightly_build_without_a_profile_is_refused_before_startup() {
        let started = std::cell::Cell::new(false);
        let result = require_profile(BuildFlavor::NightlyV1, &Profile::Stable).map(|()| started.set(true));
        assert!(result.unwrap_err().contains("ArtCraft Launcher"));
        assert!(!started.get());
    }

    #[test]
    fn stable_and_nightly_flavors_accept_only_their_own_profile() {
        let nightly = Profile::NightlyV1(std::path::PathBuf::from("already validated root"));
        assert!(require_profile(BuildFlavor::Stable, &Profile::Stable).is_ok());
        assert!(require_profile(BuildFlavor::Stable, &nightly).is_err());
        assert!(require_profile(BuildFlavor::NightlyV1, &nightly).is_ok());
    }

    #[test]
    fn the_nightly_version_line_is_exact_and_says_its_channel() {
        let nightly = version(BuildFlavor::NightlyV1);
        assert!(nightly.starts_with("TextureCraft "), "{nightly}");
        assert!(nightly.ends_with(" (nightly-v1)"), "{nightly}");
        assert_eq!(nightly, format!("TextureCraft {} (nightly-v1)", BUILT_VERSION));
        assert_eq!(version(BuildFlavor::Stable), format!("TextureCraft {BUILT_VERSION}"));
    }

    #[test]
    fn only_a_bare_version_flag_is_a_query() {
        assert!(version_query(&["--version".into()]));
        assert!(version_query(&["-V".into()]));
        assert!(!version_query(&["--".into(), "--version".into()]));
        assert!(!version_query(&["--version".into(), "--version".into()]));
        assert!(!version_query(&[]));
    }
}
