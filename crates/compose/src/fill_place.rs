use texturecraft_core::{Fill, Projection};

use crate::fill_depth::FillDepth;

/// A fill as a texel samples it: the fill, its projector's depth map when it culls by depth, and whether it covers every texel
#[derive(Clone, Copy)]
pub(crate) struct FillPlace<'f> {
    pub fill: &'f Fill,
    pub projection: &'f Projection,
    pub depth: Option<&'f FillDepth>,
    /// Decided once per tile, so a texel never works its coverage out
    pub covers_all: bool,
}
