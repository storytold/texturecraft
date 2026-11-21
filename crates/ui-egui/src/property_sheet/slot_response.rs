/// What a slot card's top row did this frame
#[derive(Clone, Debug)]
pub(crate) struct SlotResponse {
    /// The slot's row: a click opens the source list hanging from it (see [`super::PropertySheet::hanging_list`])
    pub(crate) pick: egui::Response,

    /// The × was clicked: back to a uniform source
    pub(crate) clear: bool,

    /// The ⟲ was clicked: restore the slot's defaults
    pub(crate) reset: bool,

    /// The slot's toggle was flipped
    pub(crate) toggled: bool,
}
