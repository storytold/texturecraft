use serde_json::{Value, json};
use texturecraft_engine::core::{LayerId, Projection};

use crate::TextureCraftApp;
use crate::frame_gizmo::selected_fill_projection;

/// The selected fill with a warp projection that the tool bar edits: a fill layer, or a fill effect in its mask
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WarpTarget {
    pub layer: LayerId,
    pub effect: Option<LayerId>,
    /// The image mirrored left to right and top to bottom inside the frame
    pub flip: [bool; 2],
}

impl WarpTarget {
    /// The selected fill when it projects through a warp, in the modes where the frame gizmo shows
    pub(crate) fn of(app: &TextureCraftApp) -> Option<WarpTarget> {
        let (layer, effect, projection) = selected_fill_projection(app)?;
        let Projection::Warp { frame, .. } = projection else { return None };

        Some(WarpTarget { layer, effect, flip: frame.flip })
    }

    /// `fields` addressed to this fill, as every command that edits a fill's projection takes it
    pub(super) fn params(&self, mut params: Value) -> Value {
        params["layer"] = json!(self.layer.0);
        if let Some(effect) = self.effect {
            params["effect"] = json!(effect.0);
        }

        params
    }
}
