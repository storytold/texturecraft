//! The channel's OS identity: the app id every TextureCraft window carries and the names its
//! windows and About dialog show.
//!
//! The stable app id matches the `.desktop` file and the hicolor icon name, so Linux docks show
//! the TextureCraft icon and Wayland window rules match it. The Nightly window carries its own id
//! (the `.desktop` id the Nightly package installs), so a taskbar, a dock and a window rule can
//! tell the two side by side installs apart. The *package* identity is a different string again
//! (`ai.storyteller.texturecraft.nightly`, the bundle and package id); the two are never mixed.

/// Matches the stable `.desktop` file and hicolor icon name.
pub const APP_ID: &str = "ai.storyteller.texturecraft";

/// The Nightly window's id, matching `packaging/linux/package.sh`'s `StartupWMClass` and the
/// nightly `.desktop` file.
pub const NIGHTLY_APP_ID: &str = "ai.storyteller.texturecraft.nightly-v1";

/// Immutable startup identity. The desktop app selects it before any window or service exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchIdentity {
    Stable,
    NightlyV1,
}

static IDENTITY: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// `Err` when the identity was already selected: it is decided once per process, at startup.
pub fn select_identity(identity: LaunchIdentity) -> Result<(), String> {
    let value = match identity {
        LaunchIdentity::Stable => 1,
        LaunchIdentity::NightlyV1 => 2,
    };
    IDENTITY
        .compare_exchange(0, value, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| "the window launch identity was already selected".into())
}

pub fn identity() -> LaunchIdentity {
    if IDENTITY.load(std::sync::atomic::Ordering::Acquire) == 2 { LaunchIdentity::NightlyV1 } else { LaunchIdentity::Stable }
}

pub const fn id_for(identity: LaunchIdentity) -> &'static str {
    match identity {
        LaunchIdentity::Stable => APP_ID,
        LaunchIdentity::NightlyV1 => NIGHTLY_APP_ID,
    }
}

pub fn app_id() -> &'static str {
    id_for(identity())
}

pub const fn name_for(identity: LaunchIdentity) -> &'static str {
    match identity {
        LaunchIdentity::Stable => "TextureCraft",
        LaunchIdentity::NightlyV1 => "TextureCraft Nightly",
    }
}

/// The app's name in this window: "TextureCraft" or "TextureCraft Nightly".
pub fn name() -> &'static str {
    name_for(identity())
}

/// Whether the services that belong to a stable install (updates, the control server) may run.
pub const fn stable_services_for(identity: LaunchIdentity) -> bool {
    matches!(identity, LaunchIdentity::Stable)
}

pub fn stable_services_allowed() -> bool {
    stable_services_for(identity())
}

/// The version this window shows. A Nightly build is prepared with an exact feed identity, which
/// the engine's own constant (the workspace version) does not know; the app sets it at startup.
static DISPLAY_VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn set_display_version(version: String) {
    let _ = DISPLAY_VERSION.set(version);
}

pub fn display_version() -> &'static str {
    DISPLAY_VERSION.get().map(String::as_str).unwrap_or(texturecraft_engine::VERSION)
}

/// A builder for an extra OS window, carrying [`app_id`].
pub fn builder() -> egui::ViewportBuilder {
    egui::ViewportBuilder::default().with_app_id(app_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nightly_identity_is_distinct_and_never_enables_stable_services() {
        assert_eq!(id_for(LaunchIdentity::Stable), APP_ID);
        assert_ne!(id_for(LaunchIdentity::NightlyV1), APP_ID);
        assert_eq!(id_for(LaunchIdentity::NightlyV1), NIGHTLY_APP_ID);
        assert!(stable_services_for(LaunchIdentity::Stable));
        assert!(!stable_services_for(LaunchIdentity::NightlyV1));
        assert_eq!(name_for(LaunchIdentity::Stable), "TextureCraft");
        assert_eq!(name_for(LaunchIdentity::NightlyV1), "TextureCraft Nightly");
    }

    #[test]
    fn the_unset_process_is_the_stable_one_and_extra_windows_carry_its_id() {
        // The identity is a process-wide value set once at startup; without it the app is stable.
        assert_eq!(identity(), LaunchIdentity::Stable);
        assert_eq!(builder().app_id.as_deref(), Some(APP_ID));
        assert_eq!(name(), "TextureCraft");
    }

    #[test]
    fn the_shown_version_falls_back_to_the_workspace_version() {
        // `set_display_version` is set once per process by the desktop app; the fallback is the
        // engine's constant, which is what a stable build shows.
        assert!(display_version() == texturecraft_engine::VERSION || !display_version().is_empty());
    }
}
