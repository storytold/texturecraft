//! One seed a folder's Randomize seeds changed, kept on the folder so its Contents reset can put it back

use serde::{Deserialize, Serialize};

use crate::{Channel, LayerId};

/// A seed changed by a folder, identified by its layer or effect and optional material channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RandomizedSeed {
    pub layer: LayerId,
    /// Position used by older files that did not identify the source.
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<(LayerId, Option<Channel>)>,
    /// Filter input name when the target owns more than one seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    pub before: u32,
    pub after: u32,
}
