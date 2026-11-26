use super::BakedMap;

/// A part of the bake settings texture sets can share: Common Settings, or one map type's settings
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BakeSyncGroup {
    Common,
    Map(BakedMap),
}
