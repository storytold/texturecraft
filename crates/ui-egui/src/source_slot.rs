//! What a source slot card edits: one channel of a fill's material, a mask fill's one grayscale source, or a filter's image input

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, ChannelKind, ImageInput};

use crate::i18n::tr;

/// The one source a slot card edits, and how its values read
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SourceSlot {
    Channel(Channel),
    /// A mask fill's source, read as gray 0..1
    Grayscale,
    /// A filter's image input, read as gray 0..1
    Input(ImageInput),
}

impl SourceSlot {
    /// The slot's name on its card
    pub(crate) fn name(self) -> &'static str {
        match self {
            SourceSlot::Channel(channel) => tr(channel.label()),
            SourceSlot::Grayscale => tr("Grayscale"),
            SourceSlot::Input(input) => tr(input.label),
        }
    }

    /// What the slot's values are
    pub(crate) fn kind(self) -> ChannelKind {
        match self {
            SourceSlot::Channel(channel) => channel.kind(),
            SourceSlot::Grayscale | SourceSlot::Input(_) => ChannelKind::Scalar,
        }
    }

    /// The value Uniform and the slot's × give it: a new mask fill's white for a grayscale
    pub(crate) fn default_value(self) -> [f32; 3] {
        match self {
            SourceSlot::Channel(channel) => channel.default_value(),
            SourceSlot::Grayscale => [1.0; 3],
            SourceSlot::Input(input) => [input.default; 3],
        }
    }

    /// The range a uniform value or a procedural's two ends run over
    pub(crate) fn range(self) -> std::ops::RangeInclusive<f32> {
        if self.kind() == ChannelKind::Signed { -1.0..=1.0 } else { 0.0..=1.0 }
    }

    /// A procedural's two ends when it is picked: a gray fill spans the whole range, a channel a softer part of it
    pub(crate) fn procedural_ends(self) -> ([f32; 3], [f32; 3]) {
        /// Base color stays off pure black and white, which read as unlit or blown out
        const BASE_COLOR_ENDS: ([f32; 3], [f32; 3]) = ([0.15; 3], [0.85; 3]);

        /// Height stays a shallow relief around flat, as a full −1..1 swing reads as deep cuts
        const HEIGHT_ENDS: ([f32; 3], [f32; 3]) = ([-0.05; 3], [0.05; 3]);

        /// The other channels keep off their extremes, so the pattern changes the surface without flattening it to one end
        const CHANNEL_ENDS: ([f32; 3], [f32; 3]) = ([0.2; 3], [0.8; 3]);

        match self {
            SourceSlot::Grayscale | SourceSlot::Input(_) => ([0.0; 3], [1.0; 3]),
            SourceSlot::Channel(Channel::BaseColor) => BASE_COLOR_ENDS,
            SourceSlot::Channel(Channel::Height) => HEIGHT_ENDS,
            SourceSlot::Channel(_) => CHANNEL_ENDS,
        }
    }

    /// The fill command's fields naming the slot (a grayscale has no channel)
    pub(crate) fn fields(self) -> Value {
        match self {
            SourceSlot::Channel(channel) => json!({"channel": channel.id()}),
            SourceSlot::Grayscale => json!({}),
            SourceSlot::Input(input) => json!({"input": input.id}),
        }
    }

    /// A key for the slot's own widget state
    pub(crate) fn key(self) -> &'static str {
        match self {
            SourceSlot::Channel(channel) => channel.id(),
            SourceSlot::Grayscale => "grayscale",
            SourceSlot::Input(input) => input.id,
        }
    }
}
