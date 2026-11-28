use serde_json::Value;

use super::ItemIcon;

/// One item of the Add effect or Add mask menu
pub(crate) struct AddItem {
    pub(crate) icon: ItemIcon,
    /// The toolbar menu's name (the button says "Add effect" / "Add mask") and the layer context menu's
    pub(crate) short: &'static str,
    pub(crate) long: &'static str,
    pub(crate) command: &'static str,
    pub(crate) params: Value,
    pub(crate) enabled: bool,
}
