//! A button on a toast that acts on what the toast says

use crate::TextureCraftApp;

/// What a toast's button does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastAction {
    /// "Update": read every outdated resource file of `project` again (the desktop's "changed on disk" notice)
    UpdateResources {
        /// The project the notice is about; the button does nothing once another one is open
        project: u64,
    },
}

impl ToastAction {
    /// The button's text (English; translated where it is drawn)
    pub fn label(self) -> &'static str {
        match self {
            ToastAction::UpdateResources { .. } => "Update",
        }
    }

    /// The button's accessible name, which says what it acts on (English; translated where it is drawn)
    pub fn accessible_name(self) -> &'static str {
        match self {
            ToastAction::UpdateResources { .. } => "Update resources",
        }
    }

    pub(crate) fn run(self, app: &mut TextureCraftApp) {
        match self {
            // Another project's notice is stale
            ToastAction::UpdateResources { project } if project != app.session.project_id => {}
            #[cfg(not(target_arch = "wasm32"))]
            ToastAction::UpdateResources { .. } => crate::resources_updater::update(app, None),

            // The browser has no files to read again
            #[cfg(target_arch = "wasm32")]
            ToastAction::UpdateResources { .. } => {}
        }
    }
}
