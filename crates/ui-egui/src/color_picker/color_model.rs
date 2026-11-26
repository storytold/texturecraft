/// What the picker's three sliders edit
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ColorModel {
    #[default]
    Hsv,
    Rgb,
}

impl ColorModel {
    pub(crate) const ALL: [ColorModel; 2] = [ColorModel::Hsv, ColorModel::Rgb];

    pub(crate) fn name(self) -> &'static str {
        match self {
            ColorModel::Hsv => "HSV",
            ColorModel::Rgb => "RGB",
        }
    }

    /// The letters before the three sliders
    pub(crate) fn letters(self) -> [&'static str; 3] {
        match self {
            ColorModel::Hsv => ["H", "S", "V"],
            ColorModel::Rgb => ["R", "G", "B"],
        }
    }
}
