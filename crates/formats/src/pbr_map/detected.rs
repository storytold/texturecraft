use super::MapKind;
use texturecraft_core::NormalFormat;

/// What the name of one file says.
#[derive(Clone, Debug, PartialEq)]
pub struct Detected {
    pub kind: MapKind,
    /// The convention named for a normal map, if any.
    pub normal: Option<NormalFormat>,
    /// Resolution in pixels named by a `2k` / `2048` token, 0 if none.
    pub resolution: u32,
    /// Higher is a stronger match (a weak word such as `bump` loses to a strong one).
    pub strength: u8,
}
