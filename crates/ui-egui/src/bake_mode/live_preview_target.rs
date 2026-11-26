use texturecraft_engine::core::BakedMap;

/// One map of one texture set, as the map preview shows it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LivePreviewTarget {
    pub(crate) set: usize,
    pub(crate) map: BakedMap,
}
