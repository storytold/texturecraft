use serde::{Deserialize, Serialize};

/// The directions a warp's move arrows follow: the frame's own, or the world's
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GizmoSpace {
    #[default]
    Object,
    World,
}
