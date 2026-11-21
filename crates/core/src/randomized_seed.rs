//! One seed a folder's Randomize seeds changed, kept on the folder so its Contents reset can put it back

use serde::{Deserialize, Serialize};

use crate::layer::LayerId;

/// A seed inside a folder that the folder's Randomize seeds changed: the `index`-th seed of layer `layer` (its fill's, then its content and mask effects', in stack order) went from `before` to `after`
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomizedSeed {
    pub layer: LayerId,
    pub index: u32,
    pub before: u32,
    pub after: u32,
}
