//! A shelf query the user saved in the Saved searches pane

use serde::{Deserialize, Serialize};

/// The search text, tab and library folder of a saved shelf query, under the name it lists as
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSearch {
    pub name: String,
    pub filter: String,
    pub tab: usize,
    pub path: Vec<String>,
}
