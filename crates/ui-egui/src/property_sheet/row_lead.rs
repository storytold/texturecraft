use egui::{Painter, Rect};

/// What leads a setting's row, left of its control
#[derive(Clone, Copy)]
pub(crate) enum RowLead<'a> {
    /// A label beside the controls column; it stacks above a full-width control when it doesn't fit, or always with `stack`
    Label { text: &'a str, stack: bool },

    /// A label beside the controls column that never widens it; it stacks only when it can't fit with [`super::LOOSE_LABEL_AIR`] before the column
    Loose(&'a str),

    /// A picture at the left edge with its label after it (culling rows)
    Picture { picture: &'a dyn Fn(&Painter, Rect), text: &'a str },

    /// A setting of the row above, dim, its label on the label column like its parent's
    Child(&'a str),

    /// A label beside the controls column in a color of its own (Color Balance's color names)
    Tinted { text: &'a str, color: egui::Color32 },
}

impl<'a> RowLead<'a> {
    /// A label beside the controls column
    pub(crate) fn label(text: &'a str) -> RowLead<'a> {
        RowLead::Label { text, stack: false }
    }

    /// The row's name, as assistive tech reads it
    pub(crate) fn text(self) -> &'a str {
        match self {
            RowLead::Label { text, .. } | RowLead::Loose(text) | RowLead::Picture { text, .. } | RowLead::Child(text) | RowLead::Tinted { text, .. } => text,
        }
    }
}
