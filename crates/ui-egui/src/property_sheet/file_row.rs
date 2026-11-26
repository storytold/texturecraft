/// One file of a file list: its name, the folder it was read from when known, and whether it is still loading
#[derive(Clone, Copy)]
pub(crate) struct FileRow<'a> {
    pub(crate) name: &'a str,
    pub(crate) folder: Option<&'a str>,
    pub(crate) loading: bool,
}
