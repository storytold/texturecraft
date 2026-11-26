use serde::{Deserialize, Serialize};

/// A folder the user added as a library: its name and path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LibraryLocation {
    pub name: String,
    pub path: String,
}
