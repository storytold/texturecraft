use std::path::PathBuf;

use super::ResourceUsage;

/// One file waiting in the window
#[derive(Clone, Debug, PartialEq)]
pub struct ImportRow {
    pub id: u64,
    pub path: PathBuf,
    pub prefix: String,
    pub usage: ResourceUsage,
}
