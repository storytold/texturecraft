use serde_json::Value;

use super::ItemIcon;

/// One item of the Add effect or Add mask menu
pub(super) struct AddItem {
    pub(super) icon: ItemIcon,
    /// The toolbar menu's name (the button says "Add effect" / "Add mask") and the layer context menu's
    pub(super) short: &'static str,
    pub(super) long: &'static str,
    pub(super) command: &'static str,
    pub(super) params: Value,
    pub(super) enabled: bool,
}
