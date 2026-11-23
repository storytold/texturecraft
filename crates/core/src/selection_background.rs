//! What a color selection gives the parts it does not select

use serde::{Deserialize, Serialize};

/// The value a color selection gives outside its colors; transparent leaves the mask below as it is
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionBackground {
    Transparent,
    /// What color selections did before the choice existed
    #[default]
    Black,
    White,
    Gray,
}

impl SelectionBackground {
    /// Menu order
    pub const ALL: [SelectionBackground; 4] =
        [SelectionBackground::Transparent, SelectionBackground::Black, SelectionBackground::White, SelectionBackground::Gray];

    /// Whether it is what selections saved before the choice had (not saved then)
    pub fn is_default(&self) -> bool {
        *self == SelectionBackground::default()
    }

    /// Command id
    pub fn id(self) -> &'static str {
        match self {
            SelectionBackground::Transparent => "transparent",
            SelectionBackground::Black => "black",
            SelectionBackground::White => "white",
            SelectionBackground::Gray => "gray",
        }
    }

    pub fn from_id(id: &str) -> Option<SelectionBackground> {
        SelectionBackground::ALL.into_iter().find(|background| background.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            SelectionBackground::Transparent => "Transparent",
            SelectionBackground::Black => "Black",
            SelectionBackground::White => "White",
            SelectionBackground::Gray => "Gray",
        }
    }

    /// The gray it gives, none for transparent
    pub fn value(self) -> Option<f32> {
        match self {
            SelectionBackground::Transparent => None,
            SelectionBackground::Black => Some(0.0),
            SelectionBackground::White => Some(1.0),
            SelectionBackground::Gray => Some(0.5),
        }
    }
}
