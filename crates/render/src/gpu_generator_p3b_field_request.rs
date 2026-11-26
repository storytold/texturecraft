//! One whole-map generator field a GPU program reads (P3b, `gpu_generator_p3b.rs`).

use texturecraft_core::KindControls;

/// A field's controls, brought into range, and the seed it builds with (0 where the kind has no randomness)
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FieldRequest {
    pub(crate) controls: KindControls,
    pub(crate) seed: u32,
}
