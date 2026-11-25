use egui_phosphor::regular as ic;

use crate::i18n::n_;

/// A page of the Settings window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SettingsPage {
    #[default]
    General,
    Shortcuts,
    Libraries,
}

impl SettingsPage {
    pub const ALL: [SettingsPage; 3] = [SettingsPage::General, SettingsPage::Shortcuts, SettingsPage::Libraries];

    pub fn label(self) -> &'static str {
        match self {
            SettingsPage::General => n_("General"),
            SettingsPage::Shortcuts => n_("Shortcuts"),
            SettingsPage::Libraries => n_("Libraries"),
        }
    }

    pub(super) fn icon(self) -> &'static str {
        match self {
            SettingsPage::General => ic::GEAR,
            SettingsPage::Shortcuts => ic::KEYBOARD,
            SettingsPage::Libraries => ic::GRID_FOUR,
        }
    }
}
