use texturecraft_engine::formats::Dest;

/// The channel swatch of an output map that a map was dropped on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChannelTarget {
    /// The single RGB swatch: R, G and B together.
    Rgb,
    /// One destination channel: R, G, B, A or the gray channel.
    One(Dest),
}

impl ChannelTarget {
    /// The destination channels the swatch writes.
    pub fn dests(self) -> &'static [Dest] {
        match self {
            ChannelTarget::Rgb => &[Dest::R, Dest::G, Dest::B],
            ChannelTarget::One(Dest::R) => &[Dest::R],
            ChannelTarget::One(Dest::G) => &[Dest::G],
            ChannelTarget::One(Dest::B) => &[Dest::B],
            ChannelTarget::One(Dest::A) => &[Dest::A],
            ChannelTarget::One(Dest::L) => &[Dest::L],
        }
    }
}
