//! What the Stencil slot's picker offers

/// One tile of the Stencil slot's picker: an image resource, or importing one
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StencilChoice {
    Image(String),
    Import,
}
