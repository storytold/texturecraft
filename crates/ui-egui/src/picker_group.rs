//! A group of a resource picker's tiles under one header

use crate::resource_picker_item::ResourcePickerItem;

/// Items under one bright header (none: no header row)
pub(crate) struct PickerGroup<K> {
    pub header: Option<&'static str>,
    pub items: Vec<ResourcePickerItem<K>>,
}
