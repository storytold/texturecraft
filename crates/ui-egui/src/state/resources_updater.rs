use texturecraft_engine::{ResourceState, ResourceStatus};

/// The Resources Updater window: the project's image resources as last checked, and the filters
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ResourcesUpdater {
    /// The resources and how they compare with their files, outdated first (checked on opening,
    /// Refresh, twice a second while open and when the app gets the focus back)
    pub rows: Vec<ResourceState>,
    /// The status the list is narrowed to (None: all)
    pub status_filter: Option<ResourceStatus>,
    /// Shows only resources whose name contains this, ignoring case
    pub name_filter: String,
    /// Top or Bottom asked the list to scroll to that end in the next frame
    pub jump: Option<egui::Align>,
    /// The project that was open when this window opened
    pub project: u64,
    /// When the rows were checked, in app seconds
    pub(crate) checked_at: f64,
    /// Whether a control of the window had the keyboard focus when it was last drawn (Escape then closes it)
    pub(crate) had_keyboard: bool,
}
