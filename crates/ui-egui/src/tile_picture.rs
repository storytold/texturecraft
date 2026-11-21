//! What a resource picker's tile shows

use std::sync::Arc;

use egui::{Color32, TextureHandle};
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::core::ProceduralKind;

/// What a tile shows: a thumbnail (none while it renders), a project image or a pattern (its thumbnail made only once the tile is drawn), a colour, a glyph, or the import square
pub(crate) enum TilePicture {
    Texture(Option<TextureHandle>),
    Image(String, Arc<ImageRes>),
    Procedural(ProceduralKind),
    Swatch(Color32),
    Glyph(&'static str),
    Import,
}
