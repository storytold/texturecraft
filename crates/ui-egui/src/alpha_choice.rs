//! What the brush Alpha slot's picker offers

use texturecraft_engine::paint::BrushAlpha;

/// One tile of the Alpha slot's picker: a built-in shape, a library alpha, an image resource, or importing one
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AlphaChoice {
    Shape(BrushAlpha),
    Library(&'static str),
    Image(String),
    Import,
}
