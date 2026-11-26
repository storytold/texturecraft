use std::collections::{BTreeMap, BTreeSet};

use crate::bake_scene::SharedHandle;
use texturecraft_engine::bake::BakePaintMap;
use texturecraft_engine::core::{BakeResolution, BakeSettings};
use texturecraft_engine::geom::Mesh;

/// What a map preview picture is made from, everything its bake reads: when any of it changes, the picture is out of date
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PreviewInput {
    pub(super) settings: BakeSettings,
    pub(super) mesh_id: u64,
    /// The set's UV tiles, the ones left out of the bake, and their resolutions
    pub(super) uv_tiles: Vec<u32>,
    pub(super) excluded_tiles: BTreeSet<u32>,
    pub(super) tile_resolutions: BTreeMap<u32, u32>,
    /// The high polys', the cage's and the set's painted maps' shared buffers: a file loaded again under the same name is another buffer
    pub(super) high: SharedHandle<Mesh>,
    pub(super) cage: SharedHandle<Mesh>,
    pub(super) painted: [SharedHandle<BakePaintMap>; 2],
    pub(super) previous_pixels: SharedHandle<Vec<f32>>,
    pub(super) size: BakeResolution,
}
