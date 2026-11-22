use texturecraft_core::LayerId;

/// Which fill `layer.fill.*` edits: a fill layer's (`layer`) or a content fill effect's (`effect`)
#[derive(Clone, Copy)]
pub(crate) enum FillTarget {
    Layer(LayerId),
    Effect(LayerId),
}
