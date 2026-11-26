/// A file list's add button: its glyph, its label (also its tooltip) and whether it can add now
pub(crate) struct FileListAdd<'a> {
    pub(crate) glyph: &'a str,
    pub(crate) label: &'a str,
    pub(crate) enabled: bool,
}
