use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use texturecraft_core::Channel;
use texturecraft_paint::BrushSettings;

/// A brush the user saved: the whole brush (size included) and the channels it paints.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserBrush {
    pub name: String,
    pub brush: BrushSettings,
    /// Channels painted when it was saved (None: leave the channels alone).
    #[serde(default)]
    pub channels: Option<BTreeSet<Channel>>,
    /// The file it was read from or last written to (None: not on disk yet). Deleting and
    /// renaming act on this file, which for an older bare `.tcbrush` is not the name's file.
    #[serde(skip)]
    pub file: Option<std::path::PathBuf>,
}
