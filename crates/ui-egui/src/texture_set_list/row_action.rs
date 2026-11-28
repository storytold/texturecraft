use super::RowEdit;

/// What a set row asks for once it has drawn
#[derive(Clone, Copy)]
pub(super) enum RowAction {
    Select,
    ToggleVisible,
    Resize(u32),
    Edit(RowEdit),
    ResetName,
    ToggleFold,
}
