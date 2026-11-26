use std::collections::BTreeMap;

use texturecraft_core::Channel;
use texturecraft_texel::TileImage;

use crate::StampSource;

/// What one symmetry copy of a smudge stroke carries: its own previous stamp and deposits
#[derive(Clone, Debug)]
pub(super) struct Lane {
    pub(super) source: Option<StampSource>,
    pub(super) prev: Option<[f32; 2]>,
    pub(super) alpha: TileImage,
    pub(super) values: BTreeMap<Channel, TileImage>,
}
