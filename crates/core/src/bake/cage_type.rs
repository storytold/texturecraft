use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CageType {
    #[default]
    DistanceBased,
    /// Distances fitted to the high poly at every low-poly vertex
    Automatic,
    CustomFile,
}
