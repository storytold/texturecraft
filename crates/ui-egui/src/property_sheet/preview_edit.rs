/// What a preview block's controls changed this frame (`Some(merge)` per edited part), and where its square is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreviewEdit {
    /// Where the preview square sits, for a surface over it
    pub(crate) square: egui::Rect,
    pub(crate) size: Option<bool>,
    pub(crate) lower: Option<bool>,
}
