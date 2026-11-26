//! A name typed in place: a field in the controls column, outlined in the accent while it is typed in

use egui::{CornerRadius, Stroke, StrokeKind};

use super::{CONTROL_RADIUS, PropertySheet, TEXT_SIZE};
use crate::theme::semibold;

/// Air before the field's text
const TEXT_INSET: i8 = 8;

impl PropertySheet<'_> {
    /// A field holding `name`; returns the new name once typing ends (Enter or a click elsewhere) with a changed, non-blank name. Escape keeps the old one
    pub(crate) fn name_field(&mut self, label: &str, name: &str) -> Option<String> {
        let (control, _) = self.row(label);
        let typing_id = self.next_id().with("typed");
        let owner_changed = self.ui.data_mut(|data| {
            let owner = data.get_temp_mut_or(typing_id.with("owner"), self.id);
            std::mem::replace(owner, self.id) != self.id
        });
        if owner_changed {
            // A different anchor must not inherit the previous draft, cursor or undo buffer
            self.ui.memory_mut(|memory| {
                memory.surrender_focus(typing_id);
                memory.data.remove::<String>(typing_id);
                memory.data.remove::<egui::text_edit::TextEditState>(typing_id);
            });
        }

        let typing = self.ui.memory(|memory| memory.has_focus(typing_id));

        // Check raw Escape even if another shortcut consumed it
        let escaped =
            self.ui.input(|input| input.raw.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));
        if escaped {
            self.ui.data_mut(|data| data.remove::<String>(typing_id));
        }

        // A focus change early in this frame still needs the draft for the final commit
        let mut text = self.ui.data(|data| data.get_temp::<String>(typing_id)).unwrap_or_else(|| name.to_owned());

        let tokens = self.tokens;
        let outline = Stroke::new(1.0, if typing { tokens.accent } else { tokens.field_border });
        self.ui.painter().rect(control, CornerRadius::same(CONTROL_RADIUS), tokens.field, outline, StrokeKind::Inside);
        let edit = egui::TextEdit::singleline(&mut text)
            .id(typing_id)
            .font(semibold(TEXT_SIZE))
            .text_color(tokens.text)
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(TEXT_INSET, 0)))
            .align(egui::Align2::LEFT_CENTER);
        let response = self.ui.put(control.shrink(1.0), edit);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
        self.drawn(control.bottom());

        if response.has_focus() {
            self.ui.data_mut(|data| data.insert_temp(typing_id, text));
            return None;
        }

        self.ui.data_mut(|data| data.remove::<String>(typing_id));
        let typed = text.trim();
        (response.lost_focus() && !escaped && !typed.is_empty() && typed != name).then(|| typed.to_owned())
    }
}
