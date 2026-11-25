/// A shortcut list row with its display label and default binding.
pub(super) struct ShortcutListEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub default: Option<&'static str>,
}

impl ShortcutListEntry {
    pub(super) const fn new(id: &'static str, label: &'static str, default: Option<&'static str>) -> Self {
        Self { id, label, default }
    }
}
