//! What a resource picker's tile shows

use std::sync::Arc;

use egui::Color32;
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::core::ProceduralKind;
use texturecraft_engine::paint::alphas::LibraryAlpha;

/// A tile's picture source. Previews are requested only when its tile is drawn.
pub(crate) enum TilePicture {
    Material(&'static str),
    Filter(&'static str),
    Alpha(&'static LibraryAlpha),
    Image(String, Arc<ImageRes>),
    Procedural(ProceduralKind),
    Swatch(Color32),
    Glyph(&'static str),
    Import,
}
