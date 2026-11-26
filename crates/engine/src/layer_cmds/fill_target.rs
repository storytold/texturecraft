use texturecraft_core::LayerId;

/// Which fill `layer.fill.*` edits: a fill layer's (`layer`), a content fill effect's (`effect`) or a mask fill effect's (`effect`: one grayscale source, no channels or material)
#[derive(Clone, Copy)]
pub(crate) enum FillTarget {
    Layer(LayerId),
    Effect(LayerId),
    Mask(LayerId),
}
