use texturecraft_engine::formats::ExportPreset;

/// A template list action
#[derive(Clone, Debug, PartialEq)]
pub enum ListAction {
    Select(String),
    New,
    /// Copy this template, select the copy
    Duplicate(String),
    StartRename(String),
    Rename(String),
    /// Delete this template (after asking)
    Remove(String),
    Import,
    /// A template read from a file, added and selected
    Imported(ExportPreset),
    Export,
}
