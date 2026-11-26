use texturecraft_core::{LayerId, NormalFormat, Source};

/// What a micro details slot reads, as compose's `Slot::of` reads it
#[derive(Clone, Debug, PartialEq)]
pub(super) enum MicroSlot {
    Uniform([f32; 3]),
    Image {
        name: String,
        flip_green: bool,
    },
    /// An anchor point's height over the whole set, from its capture program ([`super::MicroCapture`])
    Anchor(LayerId),
}

impl MicroSlot {
    /// The slot holding `source`: an empty slot and a procedural read black, an anchor its height
    /// when read as the height (`height_anchor`), else black too
    pub(super) fn of(source: Option<&Source>, height_anchor: bool) -> MicroSlot {
        match source {
            Some(Source::Uniform(value)) => MicroSlot::Uniform(*value),
            Some(Source::Image { name, normal_format }) => MicroSlot::Image { name: name.clone(), flip_green: *normal_format == NormalFormat::DirectX },
            Some(Source::Anchor { anchor, .. }) if height_anchor => MicroSlot::Anchor(*anchor),
            _ => MicroSlot::Uniform([0.0; 3]),
        }
    }
}
