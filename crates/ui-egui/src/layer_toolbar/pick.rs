use serde_json::Value;

/// What a click in one of the row's menus asks for, done once the menu has closed
pub(super) enum Pick {
    Run { command: &'static str, params: Value },
    Bitmap,
}
