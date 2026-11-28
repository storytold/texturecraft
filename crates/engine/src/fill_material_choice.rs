//! What `layer.fill.material` applies to a fill: a whole built-in material, an anchor point, or clearing back to defaults

use texturecraft_core::LayerId;

use crate::materials::SmartMaterial;

/// Whole-fill target choice for `layer.fill.material`
#[derive(Clone, Copy)]
pub(crate) enum FillMaterialChoice {
    Clear,
    Material(&'static SmartMaterial),
    Anchor(LayerId),
}
