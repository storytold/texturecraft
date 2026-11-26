//! The bake panels' settings on the property sheet: labels above full-width controls, as the reference's bake panels lay them out

use crate::property_sheet::PropertySheet;

/// A folding section with `contents` under its caption; its reset glyph resets at once, or opens `reset_menu` when not empty. Returns the reset choice: `Some(0)` for a plain reset, else the chosen item of `reset_menu` (from 0)
pub(super) fn section(sheet: &mut PropertySheet<'_>, title: &str, reset_menu: &[&str], contents: impl FnOnce(&mut PropertySheet<'_>)) -> Option<usize> {
    let section = sheet.fold_section(title, true);
    let mut chosen = None;
    if reset_menu.is_empty() {
        chosen = section.reset.clicked().then_some(0);
    } else {
        // Hanging left from the glyph at the panel's right edge, so it stays inside the panel
        egui::Popup::menu(&section.reset).align(egui::RectAlign::BOTTOM_END).align_alternatives(&[egui::RectAlign::BOTTOM_START]).show(|ui| {
            for (index, item) in reset_menu.iter().enumerate() {
                if ui.button(*item).clicked() {
                    chosen = Some(index);
                }
            }
        });
    }
    if section.open {
        contents(sheet);
    }
    chosen
}

/// A dropdown of `options`, each with its shown name
pub(super) fn choice<T: Copy + PartialEq>(sheet: &mut PropertySheet<'_>, label: &str, value: &mut T, options: &[(T, &str)]) {
    let values: Vec<T> = options.iter().map(|(option, _)| *option).collect();
    let name = |option: T| options.iter().find(|(candidate, _)| *candidate == option).map_or("", |(_, name)| *name).to_owned();
    if let Some(picked) = sheet.dropdown_above(label, *value, &values, name) {
        *value = picked;
    }
}

/// A slider whose dragged values keep `decimals` places
pub(super) fn number(sheet: &mut PropertySheet<'_>, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, decimals: usize) {
    if sheet.slider_above(label, value, range.clone(), decimals).is_some() {
        let scale = 10_f32.powi(decimals as i32);
        *value = ((*value * scale).round() / scale).clamp(*range.start(), *range.end());
    }
}

/// A slider of whole numbers
pub(super) fn integer(sheet: &mut PropertySheet<'_>, label: &str, value: &mut u32, range: std::ops::RangeInclusive<u32>) {
    let mut number_value = *value as f32;
    if sheet.slider_above(label, &mut number_value, *range.start() as f32..=*range.end() as f32, 0).is_some() {
        *value = (number_value.round() as u32).clamp(*range.start(), *range.end());
    }
}

/// A one-line text field
pub(super) fn text_input(sheet: &mut PropertySheet<'_>, label: &str, value: &mut String) {
    sheet.text_field_above(label, value);
}
