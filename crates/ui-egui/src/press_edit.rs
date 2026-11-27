//! What [`crate::widgets::press_edit`] keeps of an editor's last change

/// The press an editor's last change belonged to, and the frame it was seen in
#[derive(Clone, Copy)]
pub(crate) struct PressEdit {
    pub(crate) press: u64,
    pub(crate) frame: u64,
}
