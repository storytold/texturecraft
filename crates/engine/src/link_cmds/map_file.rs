use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct MapFile {
    pub(super) usage: String,
    pub(super) file: String,
    pub(super) size: [u32; 2],
}
