/// What a Resources Updater row's buttons asked for, run after the window is drawn
pub(super) enum RowRequest {
    /// Update: read the resource's file again
    Update(String),
    /// Select new resource: ask for another file for the resource
    SelectNew(String),
    /// Show in folder: the folder holding this file
    ShowFolder(String),
}
