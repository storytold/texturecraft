//! What a resource picker's tile shows

use std::sync::Arc;

use egui::Color32;
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::core::{GeneratorKind, ProceduralKind};
use texturecraft_engine::paint::alphas::LibraryAlpha;

/// A tile's picture source. Previews are requested only when its tile is drawn.
pub(crate) enum TilePicture {
    Material(&'static str),
    Filter(&'static str),
    Alpha(&'static LibraryAlpha),
    Image(String, Arc<ImageRes>),
    Procedural(ProceduralKind),
    /// A mask generator at its defaults on the smart masks' orb, a die in its corner when a seed changes its result
    Generator(GeneratorKind),
    Swatch(Color32),
    Glyph(&'static str),
    Import,
}
