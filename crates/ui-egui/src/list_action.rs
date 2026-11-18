use texturecraft_engine::formats::ExportPreset;

/// A template list action
#[derive(Clone, Debug, PartialEq)]
pub enum ListAction {
    Select(String),
    New,
    Duplicate,
    StartRename(String),
    Rename(String),
    Remove,
    Import,
    /// A template read from a file, added and selected
    Imported(ExportPreset),
    Export,
}
