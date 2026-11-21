//! A tab of a resource picker's edge strip

use crate::picker_group::PickerGroup;
use crate::resource_picker_item::ResourcePickerItem;

/// A tab of the picker's edge strip: its icon, name and groups (a picker with one tab draws no strip)
pub(crate) struct PickerTab<K> {
    pub icon: &'static str,
    pub name: &'static str,
    pub groups: Vec<PickerGroup<K>>,
}

impl<K> PickerTab<K> {
    /// Every item of the tab, group after group
    pub fn items(&self) -> impl Iterator<Item = &ResourcePickerItem<K>> {
        self.groups.iter().flat_map(|group| group.items.iter())
    }
}
