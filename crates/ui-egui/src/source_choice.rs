//! What a fill slot's source picker offers

use texturecraft_engine::core::{LayerId, ProceduralKind};

/// A source the slot's picker offers
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SourceChoice {
    Uniform,
    Procedural(ProceduralKind),
    Image(String),
    Alpha(&'static str),
    Import,
    Anchor(LayerId),
}
