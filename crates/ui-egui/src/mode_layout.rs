//! A layout file contains one mode's panels and view arrangement, never preferences or recent files.
use crate::{
    docking::Dock,
    state::{Mode, UiState, ViewLayout},
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(crate) struct ModeLayout {
    pub mode: Mode,
    dock: Dock,
    layout: ViewLayout,
    swap_views: bool,
    split_ratio: f32,
    toolbar_docks: bool,
    toolbar_tools: bool,
}

impl ModeLayout {
    pub fn of(ui: &UiState, mode: Mode) -> Self {
        let mut state = ui.clone();
        state.mode = mode;
        state.sync_mode_dock();
        Self {
            mode,
            dock: state.dock,
            layout: state.layout,
            swap_views: state.swap_views,
            split_ratio: state.split_ratio,
            toolbar_docks: state.toolbar_docks,
            toolbar_tools: state.toolbar_tools,
        }
    }

    pub fn apply(self, ui: &mut UiState) {
        ui.mode = self.mode;
        ui.sync_mode_dock();
        ui.dock = self.dock;
        ui.layout = self.layout;
        ui.swap_views = self.swap_views;
        ui.split_ratio = self.split_ratio;
        ui.toolbar_docks = self.toolbar_docks;
        ui.toolbar_tools = self.toolbar_tools;
        ui.rail_panel = None;
    }

    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        let layout: Self = if value.get("windows").is_some() {
            let state: UiState = serde_json::from_value(value).map_err(|error| error.to_string())?;
            if !crate::ui_state_store::fully_read(&state) {
                return Err("Invalid UI layout".into());
            }

            Self::of(&state, state.mode)
        } else {
            serde_json::from_value(value).map_err(|error| error.to_string())?
        };
        if !layout.split_ratio.is_finite() || !(0.0..=1.0).contains(&layout.split_ratio) || layout.dock.replaced_unreadable() {
            return Err("Invalid UI layout".into());
        }

        Ok(layout)
    }
}
