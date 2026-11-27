use std::sync::Weak;

use egui::TextureHandle;
use texturecraft_engine::compose::ImageRes;

/// A cached picture
pub(super) struct Picture {
    pub(super) texture: Option<TextureHandle>,
    pub(super) full: bool,
    pub(super) request: u64,
    /// The pass it was last drawn in
    pub(super) drawn: u64,
    /// The images it shows, so no new image reuses their addresses (and so their key) while it is cached
    pub(super) _images: Vec<Weak<ImageRes>>,
}
