use super::{MaterialInterface, map_file::MapFile};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct CookReport {
    pub(super) material: MaterialInterface,
    pub(super) maps: Vec<MapFile>,
}
