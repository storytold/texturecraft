use texturecraft_core::BakedMap;

/// A generator input a command names: a mesh map it reads, its texture or its custom grunge
#[derive(Clone, Copy)]
pub(crate) enum GeneratorInput {
    Map(BakedMap),
    Texture,
    Grunge,
    MicroHeight,
    MicroNormal,
    Image,
}
