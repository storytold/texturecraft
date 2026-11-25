use egui::Color32;

/// How a channel swatch is drawn.
pub(super) struct SwatchLook {
    /// The source map's colour (None: empty or a constant).
    pub(super) fill: Option<Color32>,
    /// One bar per channel of the source the swatch reads.
    pub(super) bars: Vec<Color32>,
    pub(super) inverted: bool,
}
