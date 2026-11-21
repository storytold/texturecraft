//! The projection preview's cached picture

use egui::TextureHandle;

/// The preview square's last picture: what it shows (`key`), how many rays per pixel along each axis it was cast with, and the texture holding it
#[derive(Clone)]
pub(super) struct RenderedPreview {
    pub(super) key: String,
    pub(super) supersample: usize,
    pub(super) texture: TextureHandle,
}
