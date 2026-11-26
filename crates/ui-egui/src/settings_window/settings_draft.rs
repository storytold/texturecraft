use crate::TextureCraftApp;
use crate::keymap::Keymap;
use crate::state::Prefs;

use super::{SettingsPage, built_in_locations};

/// What the Settings window shows and edits until OK or Cancel.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingsDraft {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) id: u64,
    pub page: SettingsPage,
    pub prefs: Prefs,
    pub keymap: Keymap,
    /// The shortcuts when the window opened: OK applies only what the Shortcuts page changed,
    /// so a shortcut rebound meanwhile in Help > Shortcuts stays.
    pub keymap_at_open: Keymap,
    /// The Libraries page's Name and Path fields.
    pub library_name: String,
    pub library_path: String,
}

impl SettingsDraft {
    pub(super) fn of(app: &TextureCraftApp) -> SettingsDraft {
        #[cfg(not(target_arch = "wasm32"))]
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

        SettingsDraft {
            #[cfg(not(target_arch = "wasm32"))]
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            page: app.ui.windows.settings_page,
            prefs: app.ui.prefs.clone(),
            keymap: app.ui.keymap.clone(),
            keymap_at_open: app.ui.keymap.clone(),
            library_name: String::new(),
            library_path: String::new(),
        }
    }

    /// Whether the Name and Path fields make a new location: both filled, a name not taken yet.
    pub fn can_add_library(&self) -> bool {
        let name = self.library_name.trim();
        !name.is_empty()
            && !self.library_path.trim().is_empty()
            && !built_in_locations().iter().any(|(built_in, _)| built_in == name)
            && !self.prefs.libraries.iter().any(|location| location.name == name)
    }
}
