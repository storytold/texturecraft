/// The selected set and its planned files for the export window.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportSetPlan {
    /// The current index, or none if the set left the project.
    pub index: Option<usize>,
    pub name: String,
    pub files: Result<serde_json::Value, String>,
}
