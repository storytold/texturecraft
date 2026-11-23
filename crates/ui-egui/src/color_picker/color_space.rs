use texturecraft_engine::texel::{linear_to_srgb, srgb_to_linear};

/// The space the picker shows a colour's values in; the fill always stores linear values
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ColorSpace {
    #[default]
    Srgb,
    Linear,
}

impl ColorSpace {
    pub(crate) const ALL: [ColorSpace; 2] = [ColorSpace::Srgb, ColorSpace::Linear];

    pub(crate) fn name(self) -> &'static str {
        match self {
            ColorSpace::Srgb => "sRGB",
            ColorSpace::Linear => crate::i18n::tr("Linear"),
        }
    }

    /// A linear colour's values in this space
    pub(crate) fn shown(self, linear: [f32; 3]) -> [f32; 3] {
        match self {
            ColorSpace::Srgb => linear.map(linear_to_srgb),
            ColorSpace::Linear => linear,
        }
    }

    /// The linear colour of values shown in this space
    pub(crate) fn linear(self, shown: [f32; 3]) -> [f32; 3] {
        match self {
            ColorSpace::Srgb => shown.map(srgb_to_linear),
            ColorSpace::Linear => shown,
        }
    }

    /// The screen's sRGB values for values shown in this space, so gradients paint the colours they pick
    pub(crate) fn display(self, shown: [f32; 3]) -> [f32; 3] {
        ColorSpace::Srgb.shown(self.linear(shown))
    }
}
