//! One reader of an anchor point, as the anchor's References band lists it

use texturecraft_engine::core::LayerId;

/// One reader of an anchor point: the layer, its name, what reads (kind name and its icon) and the effect (None: the fill layer itself)
pub(crate) struct Reference {
    pub(crate) layer: LayerId,
    pub(crate) name: String,
    pub(crate) kind: &'static str,
    pub(crate) glyph: &'static str,
    pub(crate) effect: Option<LayerId>,
}
