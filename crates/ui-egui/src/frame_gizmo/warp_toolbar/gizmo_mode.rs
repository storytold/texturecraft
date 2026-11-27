use serde::{Deserialize, Serialize};

/// What a warp's frame gizmo does: Move shows the arrows and centre square, Rotate the turn ring, Scale the edge dots that size the frame; Place on surface puts the frame where a drag over the model touches it
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GizmoMode {
    #[default]
    Move,
    Rotate,
    Scale,
    PlaceOnSurface,
}
