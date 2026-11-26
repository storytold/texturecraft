use texturecraft_core::{NormalFormat, Source};

/// What a micro details slot reads, as compose's `Slot::of` reads it
#[derive(Clone, Debug, PartialEq)]
pub(super) enum MicroSlot {
    Uniform([f32; 3]),
    Image { name: String, flip_green: bool },
}

impl MicroSlot {
    /// The slot holding `source`: an empty slot, a procedural and an anchor read black. None for
    /// an anchor read as the height (`height_anchor`), whose height the GPU does not gather.
    pub(super) fn of(source: Option<&Source>, height_anchor: bool) -> Option<MicroSlot> {
        match source {
            Some(Source::Uniform(value)) => Some(MicroSlot::Uniform(*value)),
            Some(Source::Image { name, normal_format }) => Some(MicroSlot::Image { name: name.clone(), flip_green: *normal_format == NormalFormat::DirectX }),
            Some(Source::Anchor { .. }) if height_anchor => None,
            _ => Some(MicroSlot::Uniform([0.0; 3])),
        }
    }
}
