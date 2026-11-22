use egui::layers::ShapeIdx;

/// A card whose rows are being laid out; its face is painted behind them once they end
pub(super) struct OpenCard {
    /// Where the card starts
    pub(super) top: f32,

    /// The placeholder the card's face is painted into, behind its rows
    pub(super) face: ShapeIdx,
}
