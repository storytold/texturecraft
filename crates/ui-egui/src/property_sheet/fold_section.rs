use egui::Response;

/// What a folding section's caption answered: whether its rows show, and its reset glyph
pub(crate) struct FoldSection {
    pub(crate) open: bool,
    pub(crate) reset: Response,
}
