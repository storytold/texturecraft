use std::collections::BTreeSet;

use super::{DescriptionMode, RowEdit};

/// The list's UI state: the row being edited, which descriptions show, folded UV tile sets and the selected tile
#[derive(Clone, Default)]
pub(crate) struct ListState {
    /// Project and mesh ids the rows' set indices belong to
    owner: (u64, u64),
    /// Set, what it edits, the text so far, and whether the field still needs focus
    pub(crate) edit: Option<(usize, RowEdit, String, bool)>,
    pub(crate) descriptions: DescriptionMode,
    /// UV tile sets whose tile rows are folded away
    pub(crate) folded: BTreeSet<usize>,
    /// Set and UDIM of the selected UV tile row
    pub(crate) tile: Option<(usize, u32)>,
    /// The last pass that drew the list with the pointer over it
    pub(crate) hovered: Option<u64>,
    /// F2 or the menu key came while the pointer was over the list: the selected row takes it in this pass
    pub(crate) rename_key: bool,
    pub(crate) menu_key: bool,
}

impl ListState {
    fn id() -> egui::Id {
        egui::Id::unique("texture_set_list_state")
    }

    /// The state for the project and mesh `owner`; another project or mesh starts over, keeping only the descriptions choice
    pub(crate) fn load(ctx: &egui::Context, owner: (u64, u64)) -> Self {
        let state: Self = ctx.data(|data| data.get_temp::<Self>(Self::id())).unwrap_or_default();
        if state.owner == owner {
            return state;
        }

        ListState { owner, descriptions: state.descriptions, hovered: state.hovered, ..ListState::default() }
    }

    /// The stored state, whatever project it belongs to
    pub(crate) fn stored(ctx: &egui::Context) -> Option<Self> {
        ctx.data(|data| data.get_temp::<Self>(Self::id()))
    }

    pub(crate) fn store(self, ctx: &egui::Context) {
        ctx.data_mut(|data| data.insert_temp(Self::id(), self));
    }
}
