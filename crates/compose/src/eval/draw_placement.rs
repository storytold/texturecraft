//! Where a layer is drawn inside the current texture set.

use texturecraft_core::{ContentsOffset, Layer, LayerId};

use super::EvaluationInputs;

#[derive(Clone, Copy, Default)]
pub(super) struct DrawPlacement {
    pub offset: ContentsOffset,
    /// Zero denotes the original folder tree
    pub instance: usize,
}

impl From<ContentsOffset> for DrawPlacement {
    fn from(offset: ContentsOffset) -> Self {
        Self { offset, instance: 0 }
    }
}

impl DrawPlacement {
    pub(super) fn children(self, inputs: &EvaluationInputs, id: LayerId, source: &Layer) -> Self {
        let instance = if id == source.id { self.instance } else { inputs.instance_draw(self.instance, id) };
        Self { offset: self.offset.then(inputs.project.contents_offset(source.id)), instance }
    }
}
