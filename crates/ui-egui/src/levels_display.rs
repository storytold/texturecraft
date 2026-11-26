//! Which histogram the levels editor draws for a colour channel

use egui::Color32;
use texturecraft_engine::compose::EffectHistogram;

use crate::i18n::n_;
use crate::theme::{AXIS_COLORS, Tokens};

/// A levels histogram's view: the three colours, all pooled, one colour, or luminance
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LevelsDisplay {
    #[default]
    Colors,
    Rgb,
    Red,
    Green,
    Blue,
    Luminance,
}

impl LevelsDisplay {
    pub(crate) const ALL: [LevelsDisplay; 6] =
        [LevelsDisplay::Colors, LevelsDisplay::Rgb, LevelsDisplay::Red, LevelsDisplay::Green, LevelsDisplay::Blue, LevelsDisplay::Luminance];

    /// Its word on the display switch, untranslated (short words of the reference's switch)
    pub(crate) fn word(self) -> &'static str {
        match self {
            LevelsDisplay::Colors => n_("Colors"),
            LevelsDisplay::Rgb => "RGB",
            LevelsDisplay::Red => "R",
            LevelsDisplay::Green => "G",
            LevelsDisplay::Blue => "B",
            LevelsDisplay::Luminance => n_("Lum"),
        }
    }

    /// The colours its word takes when chosen: red, green and blue split "Colors"
    pub(crate) fn tints(self) -> &'static [Color32] {
        let [red, green, blue] = &AXIS_COLORS;
        match self {
            LevelsDisplay::Colors => &AXIS_COLORS,
            LevelsDisplay::Red => std::slice::from_ref(red),
            LevelsDisplay::Green => std::slice::from_ref(green),
            LevelsDisplay::Blue => std::slice::from_ref(blue),
            LevelsDisplay::Rgb | LevelsDisplay::Luminance => &[],
        }
    }

    /// The histograms it draws, each with its colour, in drawing order
    pub(crate) fn series<'h>(self, histogram: &'h EffectHistogram, tokens: &Tokens) -> Vec<(&'h [u32], Color32)> {
        let component =
            |index: usize| histogram.components.get(index).map(|counts| (counts.as_slice(), AXIS_COLORS.get(index).copied().unwrap_or(tokens.text)));
        match self {
            LevelsDisplay::Colors => (0..AXIS_COLORS.len()).filter_map(component).collect(),
            LevelsDisplay::Red => component(0).into_iter().collect(),
            LevelsDisplay::Green => component(1).into_iter().collect(),
            LevelsDisplay::Blue => component(2).into_iter().collect(),
            LevelsDisplay::Rgb => vec![(histogram.all.as_slice(), tokens.text)],
            LevelsDisplay::Luminance => vec![(histogram.luminance.as_slice(), tokens.text)],
        }
    }

    /// The counts Auto fits the input range to: what the view shows, the three colours pooled
    pub(crate) fn auto_counts(self, histogram: &EffectHistogram) -> &[u32] {
        let component = |index: usize| histogram.components.get(index).map_or(histogram.all.as_slice(), Vec::as_slice);
        match self {
            LevelsDisplay::Colors | LevelsDisplay::Rgb => &histogram.all,
            LevelsDisplay::Red => component(0),
            LevelsDisplay::Green => component(1),
            LevelsDisplay::Blue => component(2),
            LevelsDisplay::Luminance => &histogram.luminance,
        }
    }
}
