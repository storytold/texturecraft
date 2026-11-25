//! Where a slider's value sits and how it reads

use egui::Id;
use egui::text_edit::TextEditState;

use super::value_text_draft::ValueTextDraft;
use crate::theme::semibold;

type ValueTextDrafts = egui::cache::FramePublisher<Id, Option<ValueTextDraft>>;

/// A value drawn right-aligned at `right` in a `width` wide box centred on `center_y`, reading `shown`; typing may end in `suffix`; it answers as `label`'s value
pub(super) struct ValueText<'a> {
    pub(super) label: &'a dyn std::fmt::Display,
    pub(super) right: f32,
    pub(super) width: f32,
    pub(super) center_y: f32,
    pub(super) shown: &'a str,
    pub(super) suffix: &'a str,
}

/// How far below a value's centre its typing line runs
const LINE_DROP: f32 = 9.0;

impl super::PropertySheet<'_> {
    /// Starts typing `text` into the field `field_id`, all of it selected so the first key replaces it
    pub(super) fn start_typing(&mut self, field_id: Id, text: String) {
        let mut editor = TextEditState::default();
        editor.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(&text)));
        self.keep_typing(field_id, ValueTextDraft { original: text.clone(), text, editor });
        self.ui.memory_mut(|memory| memory.request_focus(field_id));
    }

    pub(super) fn typing_draft(&self, field_id: Id) -> Option<ValueTextDraft> {
        self.ui.memory_mut(|memory| memory.caches.cache::<ValueTextDrafts>().get(&field_id).cloned().flatten())
    }

    /// Keep the draft only while its field is drawn
    pub(super) fn keep_typing(&self, field_id: Id, draft: ValueTextDraft) {
        self.ui.memory_mut(|memory| memory.caches.cache::<ValueTextDrafts>().set(field_id, Some(draft)));
    }

    /// Ends typing and reports whether the text still matches what the field opened with
    pub(super) fn end_typing(&self, field_id: Id, draft: &ValueTextDraft) -> bool {
        self.ui.memory_mut(|memory| memory.caches.cache::<ValueTextDrafts>().set(field_id, None));
        draft.original.trim() == draft.text.trim()
    }

    /// Whether Escape was pressed this frame, even where a shortcut took it before the field lost its focus to it
    pub(super) fn escape_pressed(&self) -> bool {
        self.ui.input(|input| input.raw.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })))
    }

    /// Drops the draft of field `field_id`, if it is being typed in, without applying it
    pub(super) fn drop_typing(&self, field_id: Id) {
        if self.typing_draft(field_id).is_some() {
            self.ui.memory_mut(|memory| {
                memory.caches.cache::<ValueTextDrafts>().set(field_id, None);
                memory.surrender_focus(field_id);
            });
        }
    }

    /// Shows an edit in place of the displayed value, without shifting the sheet's rows
    pub(super) fn typing_edit(&mut self, left: f32, right: f32, center_y: f32, field_id: Id, draft: &mut ValueTextDraft) -> egui::Response {
        let font = semibold(super::TEXT_SIZE);
        let line = self.ui.painter().layout_no_wrap("0".to_owned(), font.clone(), self.tokens.text).size().y;
        let height = line.max(self.ui.spacing().interact_size.y);
        let rect = egui::Rect::from_min_max(egui::pos2(left, center_y - height / 2.0), egui::pos2(right, center_y + height / 2.0));
        let layout = egui::Layout::centered_and_justified(egui::Direction::TopDown);
        let mut child = self.ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));

        draft.editor.clone().store(child.ctx(), field_id);
        let output = egui::TextEdit::singleline(&mut draft.text)
            .id(field_id)
            .font(font)
            .align(egui::Align2::RIGHT_CENTER)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .show(&mut child);

        // Keep selection and undo history with the draft instead of egui's persistent widget data
        child.data_mut(|data| data.remove::<TextEditState>(field_id));
        draft.editor = output.state;
        output.response.response
    }

    /// Takes the one auto id [`Self::typing_edit`]'s child takes, for a value shown instead of typed: the controls after it keep their ids when typing starts or ends, so a click from one field to the next types into the one clicked
    pub(super) fn skip_typing_edit(&mut self) {
        self.ui.skip_ahead_auto_ids(1);
    }

    /// The 1 px line under a typeable value centred on `center_y`, from `left` to `right`: grey on hover, accent while typing
    pub(super) fn value_line(&self, left: f32, right: f32, center_y: f32, color: egui::Color32) {
        let y = (center_y + LINE_DROP).round() + 0.5;
        self.ui.painter().line_segment([egui::pos2(left, y), egui::pos2(right, y)], egui::Stroke::new(1.0, color));
    }
}
