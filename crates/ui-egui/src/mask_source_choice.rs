//! What the mask paint Grayscale slot's picker offers

/// One tile of the Grayscale slot's picker: one gray, an image resource, or importing one
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MaskSourceChoice {
    Uniform,
    Image(String),
    Import,
}
