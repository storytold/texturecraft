use texturecraft_engine::core::{BakeSyncGroup, BakedMap};

/// What a texture set picker does with the sets chosen in it
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SetPickerPurpose {
    /// Exactly the chosen sets share the group's settings
    Sync(BakeSyncGroup),
    /// Copy from the active set to the chosen ones: `what` as the `bake.copy` command takes it, for one map or for all
    Copy { what: &'static str, map: Option<BakedMap> },
}
