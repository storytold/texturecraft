use serde::{Deserialize, Serialize};

use crate::docking::Dock;

/// One saved panel arrangement
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedLayout {
    pub name: String,
    /// The painting mode's dock, which the other modes' docks start from
    pub painting: Dock,
    pub baking: Dock,
}
