use egui::ColorImage;

/// What the worker answers
pub(super) enum OrbResult {
    Picture {
        key: u64,
        full: bool,
        image: ColorImage,
    },
    /// A newer job for its place replaced it before it was finished
    Dropped {
        key: u64,
    },
}
