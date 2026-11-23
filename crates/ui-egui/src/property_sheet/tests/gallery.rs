//! What the gallery's controls last set

use super::Filtering;

/// Every control of the shared set once, with the values the gallery last set
pub(super) struct Gallery {
    pub(super) scale: usize,
    pub(super) filtering: Filtering,
    pub(super) hardness: f32,
    pub(super) hardness_merges: Vec<bool>,
    pub(super) offset: f32,
    pub(super) rotation: f32,
    pub(super) position: [f32; 3],
    pub(super) position_merges: Vec<bool>,
    pub(super) linked: bool,
    pub(super) wrap: bool,
    pub(super) crop: bool,
    pub(super) depth_culling: bool,
    pub(super) backface_culling: bool,
    pub(super) channel_on: bool,
    pub(super) channels: [bool; 5],
    pub(super) channel_value: f32,
    pub(super) intensity: f32,
    pub(super) divider: &'static str,
    pub(super) seed: u32,
    pub(super) randomized: u32,
    pub(super) long_choice: usize,
}

impl Default for Gallery {
    fn default() -> Self {
        Gallery {
            scale: 0,
            filtering: Filtering::BilinearHq,
            hardness: 0.75,
            hardness_merges: Vec::new(),
            offset: -0.25,
            rotation: 30.0,
            position: [0.0, 1.25, -0.5],
            position_merges: Vec::new(),
            linked: true,
            wrap: true,
            crop: false,
            depth_culling: true,
            backface_culling: false,
            channel_on: true,
            channels: [true, true, true, false, true],
            channel_value: 0.8,
            intensity: 0.5,
            divider: "1",
            seed: 1234,
            randomized: 0,
            long_choice: 0,
        }
    }
}
