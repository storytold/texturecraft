use std::cell::RefCell;
use std::path::PathBuf;

/// Where the files of an import come from.
pub(super) enum Origin {
    Folder { root: PathBuf },
    Files(Vec<PathBuf>),
    Zip(RefCell<texturecraft_formats::zip_read::ZipFiles>),
}
