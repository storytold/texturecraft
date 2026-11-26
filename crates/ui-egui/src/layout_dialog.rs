/// The layout dialog being shown (not saved)
#[derive(Clone, Debug, PartialEq)]
pub enum LayoutDialog {
    /// Asking for the name to save the current layout under
    Save(String),
    /// A layout of this name exists: asking whether to replace it
    Overwrite(String),
    /// The list with rename and delete
    Manage { renaming: Option<(String, String)>, deleting: Option<String> },
}
