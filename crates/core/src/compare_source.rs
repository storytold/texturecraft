//! What one side of a compare mask reads

use serde::{Deserialize, Serialize};

/// One side of a compare mask's comparison
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareSource {
    /// The compared channel of the stack below the layer
    LayersBelow,
    /// The compared channel of the layer's own content, laid over the stack below by its coverage
    ThisLayer,
    /// The mask so far (the effects under the compare)
    ThisMask,
    /// The compare's constant
    Constant,
}

impl CompareSource {
    /// Menu order
    pub const ALL: [CompareSource; 4] = [CompareSource::LayersBelow, CompareSource::ThisLayer, CompareSource::ThisMask, CompareSource::Constant];

    /// Command id
    pub fn id(self) -> &'static str {
        match self {
            CompareSource::LayersBelow => "layersBelow",
            CompareSource::ThisLayer => "thisLayer",
            CompareSource::ThisMask => "thisMask",
            CompareSource::Constant => "constant",
        }
    }

    pub fn from_id(id: &str) -> Option<CompareSource> {
        CompareSource::ALL.into_iter().find(|source| source.id() == id)
    }

    /// Reads a material channel of the stack (the layer or the layers below)
    pub fn reads_stack(self) -> bool {
        matches!(self, CompareSource::LayersBelow | CompareSource::ThisLayer)
    }

    /// The left side of compare masks saved before sources existed
    pub(crate) fn saved_left() -> CompareSource {
        CompareSource::ThisMask
    }

    /// The right side of compare masks saved before sources existed
    pub(crate) fn saved_right() -> CompareSource {
        CompareSource::Constant
    }

    /// Whether a left side reads what saved compare masks did (not saved then)
    pub(crate) fn is_saved_left(&self) -> bool {
        *self == CompareSource::saved_left()
    }

    /// Whether a right side reads what saved compare masks did (not saved then)
    pub(crate) fn is_saved_right(&self) -> bool {
        *self == CompareSource::saved_right()
    }
}
