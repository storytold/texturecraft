use serde::{Deserialize, Serialize};

/// How a color texture becomes one gray value
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GeneratorGrayscale {
    /// Halfway between the brightest and the darkest channel
    Desaturation,
    /// The channels weighted by how bright the eye sees them
    Luma,
    /// The mean of the three channels
    #[default]
    Average,
    Max,
    Min,
    Red,
    Green,
    Blue,
}

impl GeneratorGrayscale {
    pub const ALL: [GeneratorGrayscale; 8] = [
        GeneratorGrayscale::Desaturation,
        GeneratorGrayscale::Luma,
        GeneratorGrayscale::Average,
        GeneratorGrayscale::Max,
        GeneratorGrayscale::Min,
        GeneratorGrayscale::Red,
        GeneratorGrayscale::Green,
        GeneratorGrayscale::Blue,
    ];

    /// The name the panel shows
    pub fn label(self) -> &'static str {
        match self {
            GeneratorGrayscale::Desaturation => "Desaturation",
            GeneratorGrayscale::Luma => "Luma",
            GeneratorGrayscale::Average => "Average",
            GeneratorGrayscale::Max => "Max",
            GeneratorGrayscale::Min => "Min",
            GeneratorGrayscale::Red => "Red channel",
            GeneratorGrayscale::Green => "Green channel",
            GeneratorGrayscale::Blue => "Blue channel",
        }
    }

    /// The name commands and saved projects use
    pub fn id(self) -> &'static str {
        match self {
            GeneratorGrayscale::Desaturation => "desaturation",
            GeneratorGrayscale::Luma => "luma",
            GeneratorGrayscale::Average => "average",
            GeneratorGrayscale::Max => "max",
            GeneratorGrayscale::Min => "min",
            GeneratorGrayscale::Red => "red",
            GeneratorGrayscale::Green => "green",
            GeneratorGrayscale::Blue => "blue",
        }
    }

    /// The conversion named by its id or its label, in any case
    pub fn parse(name: &str) -> Option<GeneratorGrayscale> {
        GeneratorGrayscale::ALL.into_iter().find(|mode| mode.id().eq_ignore_ascii_case(name) || mode.label().eq_ignore_ascii_case(name))
    }

    /// The gray of `rgb`
    pub fn gray(self, [red, green, blue]: [f32; 3]) -> f32 {
        // ITU-R BT.601 weights, the reference's Luma
        const LUMA: [f32; 3] = [0.299, 0.587, 0.114];
        match self {
            GeneratorGrayscale::Desaturation => (red.max(green).max(blue) + red.min(green).min(blue)) * 0.5,
            GeneratorGrayscale::Luma => red * LUMA[0] + green * LUMA[1] + blue * LUMA[2],
            GeneratorGrayscale::Average => (red + green + blue) / 3.0,
            GeneratorGrayscale::Max => red.max(green).max(blue),
            GeneratorGrayscale::Min => red.min(green).min(blue),
            GeneratorGrayscale::Red => red,
            GeneratorGrayscale::Green => green,
            GeneratorGrayscale::Blue => blue,
        }
    }
}
