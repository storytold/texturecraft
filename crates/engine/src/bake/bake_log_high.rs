use super::BakeHighState;

/// One row of the Baking Log's high poly list: a high poly file a texture set bakes from in the latest bake
#[derive(Clone, Debug, PartialEq)]
pub struct BakeLogHigh {
    pub texture_set: usize,
    pub set_name: String,
    /// The file's name as the project keeps it (the project carries the file itself, not where it came from)
    pub file: String,
    pub state: BakeHighState,
}
