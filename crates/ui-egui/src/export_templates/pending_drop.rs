use texturecraft_engine::formats::SourceRef;

use super::ChannelTarget;

/// A map dropped on an output channel, waiting for the user to pick which of its channels to use.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingDrop {
    /// The output map, by index into the template's maps.
    pub map: usize,
    pub target: ChannelTarget,
    pub source: SourceRef,
    /// Where the popup opens.
    pub position: egui::Pos2,
}
