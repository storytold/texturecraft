//! What the gallery's controls last set

use super::Filtering;

/// The gallery's controls, with the values they last set
pub(super) struct Gallery {
    pub(super) filtering: Filtering,
    pub(super) position: [f32; 3],
    pub(super) position_merges: Vec<bool>,
    pub(super) linked: bool,
}

impl Default for Gallery {
    fn default() -> Self {
        Gallery { filtering: Filtering::BilinearHq, position: [0.0, 1.25, -0.5], position_merges: Vec::new(), linked: true }
    }
}
