//! How far a 3D projection reaches past its shape

use serde::{Deserialize, Serialize};

/// Whether a projection reaches past its shape (planar frame, tri-planar box, sphere, cylinder)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ShapeCrop {
    #[default]
    ExtendsOutside,
    Cropped,
}

impl ShapeCrop {
    pub const ALL: [ShapeCrop; 2] = [ShapeCrop::Cropped, ShapeCrop::ExtendsOutside];

    /// Command id
    pub fn id(self) -> &'static str {
        match self {
            ShapeCrop::ExtendsOutside => "extendsOutside",
            ShapeCrop::Cropped => "cropped",
        }
    }

    pub fn from_id(id: &str) -> Option<ShapeCrop> {
        ShapeCrop::ALL.into_iter().find(|crop| crop.id() == id)
    }
}
