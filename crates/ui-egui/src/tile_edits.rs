//! The edits one frame of the Tile Generator's controls makes

use egui::Ui;
use serde_json::{Value, json};

use crate::angle_dial::angle_dial;
use crate::gray_slider::gray_slider;
use crate::i18n::tr;
use crate::widgets::{combo, slider_edit};

/// The edits one frame of the Tile Generator's controls makes: settings by command name, and whether they continue a drag
#[derive(Default)]
pub(crate) struct TileEdits {
    pub settings: serde_json::Map<String, Value>,
    pub merge: Option<bool>,
}

impl TileEdits {
    pub(crate) fn set(&mut self, key: &str, value: Value, merge: bool) {
        self.settings.insert(key.to_string(), value);
        self.merge = Some(self.merge.unwrap_or(true) && merge);
    }

    pub(crate) fn slider(&mut self, ui: &mut Ui, label: &str, key: &str, mut value: f32, range: std::ops::RangeInclusive<f32>) {
        if let Some(merge) = slider_edit(ui, label, &mut value, range, 2) {
            self.set(key, json!(value), merge);
        }
    }

    pub(crate) fn count(&mut self, ui: &mut Ui, label: &str, key: &str, value: u32, range: std::ops::RangeInclusive<u32>) {
        let mut shown = value as f32;
        if let Some(merge) = slider_edit(ui, label, &mut shown, *range.start() as f32..=*range.end() as f32, 0) {
            self.set(key, json!(shown.round() as u32), merge);
        }
    }

    /// Two sliders for one `[horizontal, vertical]` setting
    pub(crate) fn pair(&mut self, ui: &mut Ui, labels: [&str; 2], key: &str, mut value: [f32; 2], range: std::ops::RangeInclusive<f32>) {
        let mut merge = None;
        for (label, component) in labels.into_iter().zip(value.iter_mut()) {
            merge = merge.or(slider_edit(ui, label, component, range.clone(), 2));
        }
        if let Some(merge) = merge {
            self.set(key, json!(value), merge);
        }
    }

    pub(crate) fn check(&mut self, ui: &mut Ui, label: &str, key: &str, mut value: bool) {
        if ui.checkbox(&mut value, label).changed() {
            self.set(key, json!(value), false);
        }
    }

    pub(crate) fn angle(&mut self, ui: &mut Ui, label: &str, key: &str, mut degrees: f32) {
        if let Some(merge) = angle_dial(ui, label, &mut degrees) {
            self.set(key, json!(degrees), merge);
        }
    }

    /// A gray from black to white with a button that inverts it, as the reference's color rows
    pub(crate) fn gray(&mut self, ui: &mut Ui, label: &str, key: &str, mut value: f32) {
        if let Some(merge) = gray_slider(ui, label, &mut value) {
            self.set(key, json!(value), merge);
        }
    }

    /// A menu of `choices`, sent by their saved names
    pub(crate) fn choice<T: Copy + PartialEq + std::fmt::Debug>(
        &mut self,
        ui: &mut Ui,
        label: &str,
        key: &str,
        current: T,
        choices: &[T],
        name: fn(T) -> &'static str,
    ) {
        combo(ui, label, tr(name(current)), |ui| {
            for choice in choices {
                if ui.selectable_label(*choice == current, tr(name(*choice))).clicked() {
                    self.set(key, json!(format!("{choice:?}")), false);
                }
            }
        });
    }
}
