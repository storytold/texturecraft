use texturecraft_core::LayerId;

use super::super::CompositeProgram;

/// The program that composites anchor point `anchor`'s height over the whole set at f32, for
/// the micro details that read it as their height: the project with the first reader and every
/// draw after it hidden, under a top layer whose Height reads the anchor
pub(crate) struct MicroCapture {
    pub(super) anchor: LayerId,
    pub(super) program: Box<CompositeProgram>,
}

impl std::fmt::Debug for MicroCapture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("MicroCapture").field("anchor", &self.anchor).finish_non_exhaustive()
    }
}
