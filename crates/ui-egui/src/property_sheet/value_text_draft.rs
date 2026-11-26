use egui::text_edit::TextEditState;

/// The input, original value, and editor state of a visible Properties field
#[derive(Clone)]
pub(super) struct ValueTextDraft {
    pub(super) text: String,
    pub(super) original: String,
    pub(super) editor: TextEditState,
}
