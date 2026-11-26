//! What a fill's Material-mode picker offers

use texturecraft_engine::core::LayerId;

/// An entry in the Material-mode picker: a built-in material or an anchor point below the layer
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaterialChoice {
    Material(&'static str),
    Anchor(LayerId),
}
