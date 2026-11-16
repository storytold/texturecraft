//! Which tools can act on the current selection: the tool shelf and its shortcuts grey the rest

use texturecraft_core::{EffectKind, LayerKind};

use crate::{Session, StackTarget, Tool};

impl Session {
    /// Whether `tool` can act on the selection (or on the quick mask while it is being edited)
    pub fn tool_usable(&self, tool: Tool) -> bool {
        match tool {
            // The picker reads the composite under the cursor, whatever is selected
            Tool::ColorPicker => return true,
            Tool::GeometryMask => return self.selected_layer().is_some(),
            _ => {}
        }

        // Brushes paint the quick mask whatever is selected; paths can't edit it
        if self.quick_mask.editing {
            return tool != Tool::Path;
        }

        let project = &self.doc.project;
        let Some(layer) = self.selected_layer().and_then(|id| project.layers.get(&id)) else { return false };

        // Smudge and Clone read layer content, so they can't yet paint mask values
        let reads_content = matches!(tool, Tool::Smudge | Tool::Clone);
        match self.target {
            StackTarget::Content => matches!(layer.kind, LayerKind::Paint),
            StackTarget::Mask => !reads_content,
            StackTarget::Effect(effect) => {
                let Some((owner, found)) = project.effect(effect) else { return false };
                let Some(owner_layer) = project.layers.get(&owner) else { return false };
                let in_content = owner_layer.content_effects.iter().any(|candidate| candidate.id == effect);
                let in_fill_content = in_content && matches!(owner_layer.kind, LayerKind::Fill(_));

                match found.kind {
                    // Paths saved in a fill layer before paths there were removed stay editable
                    EffectKind::Path(_) => tool == Tool::Path,
                    EffectKind::Paint if tool == Tool::Path => !in_fill_content,
                    EffectKind::Paint => in_content || !reads_content,
                    _ => false,
                }
            }
        }
    }
}
