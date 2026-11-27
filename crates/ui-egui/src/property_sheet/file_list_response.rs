/// What a file list answered: its add button clicked, or a file's × clicked
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FileListResponse {
    pub(crate) add: bool,
    pub(crate) removed: Option<usize>,
}
