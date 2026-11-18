//! One tile of a resource picker's Resources tab

use std::sync::Arc;

use egui::TextureHandle;
use texturecraft_engine::compose::ImageRes;

use crate::shelf_preview::ShelfPreview;

/// One tile of the Resources tab: the key a pick returns, its label and group, its picture and its hover preview
pub(crate) struct ResourcePickerTile<'a> {
    pub key: String,
    pub label: String,
    pub group: &'static str,
    pub thumbnail: Option<TextureHandle>,
    pub image: Option<Arc<ImageRes>>,
    pub preview: Box<dyn Fn() -> ShelfPreview + 'a>,
}
