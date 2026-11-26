//! What one image input of a generator replaces or holds

use texturecraft_engine::core::{BakedMap, Generator, KindControls, Source};

use crate::i18n::{baked_map_label, tr};

/// An image input of a generator: a mesh map it reads, or one of the images its switches add
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum GeneratorInputSlot {
    Map(BakedMap),
    Texture,
    Grunge,
    MicroHeight,
    MicroNormal,

    /// Grayscale Conversion's image
    Image,
}

impl GeneratorInputSlot {
    /// The input's name in `effect.set`
    pub(crate) fn id(self) -> &'static str {
        match self {
            GeneratorInputSlot::Map(map) => map.id(),
            GeneratorInputSlot::Texture => "texture",
            GeneratorInputSlot::Grunge => "grunge",
            GeneratorInputSlot::MicroHeight => texturecraft_engine::micro_params::MICRO_HEIGHT_INPUT,
            GeneratorInputSlot::MicroNormal => texturecraft_engine::micro_params::MICRO_NORMAL_INPUT,
            GeneratorInputSlot::Image => "image",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            GeneratorInputSlot::Map(map) => tr(baked_map_label(map)),
            GeneratorInputSlot::Texture => tr("Texture"),
            GeneratorInputSlot::Grunge => tr("Custom Grunge"),
            GeneratorInputSlot::MicroHeight => tr("Micro Height"),
            GeneratorInputSlot::MicroNormal => tr("Micro Normal"),
            GeneratorInputSlot::Image => tr("Input Image"),
        }
    }

    /// The value a slot set to a uniform value starts at
    pub(crate) fn neutral(self) -> f32 {
        match self {
            GeneratorInputSlot::Map(map) => map.neutral()[0],
            GeneratorInputSlot::MicroHeight | GeneratorInputSlot::MicroNormal => 0.5,
            GeneratorInputSlot::Texture | GeneratorInputSlot::Grunge | GeneratorInputSlot::Image => 0.0,
        }
    }

    /// Whether it may read an anchor point (the engine computes the micro normal from an image or a value only)
    pub(crate) fn takes_anchor(self) -> bool {
        self != GeneratorInputSlot::MicroNormal
    }

    /// What it holds on `generator`: None reads the mesh map, or leaves an image slot empty
    pub(crate) fn source(self, generator: &Generator) -> Option<&Source> {
        match self {
            GeneratorInputSlot::Map(map) => generator.inputs.get(&map),
            GeneratorInputSlot::Texture => generator.texture.as_ref().and_then(|texture| texture.source.as_ref()),
            GeneratorInputSlot::Grunge => generator.grunge.as_ref(),
            GeneratorInputSlot::MicroHeight => generator.micro.as_ref().and_then(|micro| micro.height_source.as_ref()),
            GeneratorInputSlot::MicroNormal => generator.micro.as_ref().and_then(|micro| micro.normal_source.as_ref()),
            GeneratorInputSlot::Image => match generator.controls.as_deref() {
                Some(KindControls::Grayscale(grayscale)) => grayscale.source.as_ref(),
                _ => None,
            },
        }
    }
}
