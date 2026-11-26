//! Reference controls whose function TextureCraft lacks: shown greyed, with a hint on hover

use egui::{Response, Ui};

use crate::i18n::{n_, tr};

/// The hover hint of a control that is shown for parity but does nothing yet
pub const NOT_AVAILABLE: &str = n_("Not available yet");

/// Add `add`'s controls greyed out, with the hint on hover over any of them
pub(crate) fn greyed<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let shown = ui.add_enabled_ui(false, add);
    shown.response.on_disabled_hover_text(tr(NOT_AVAILABLE));
    shown.inner
}

/// A greyed checkbox showing `checked`
pub(crate) fn checkbox(ui: &mut Ui, checked: bool, label: &str) -> Response {
    let mut shown = checked;
    ui.add_enabled(false, egui::Checkbox::new(&mut shown, label)).on_disabled_hover_text(tr(NOT_AVAILABLE))
}

/// A greyed labelled dropdown showing `value`
pub(crate) fn combo(ui: &mut Ui, label: &str, value: &str) {
    greyed(ui, |ui| crate::widgets::combo(ui, label, value, |_| {}));
}

/// A greyed slider at `value`
pub(crate) fn slider(ui: &mut Ui, label: &str, value: f32, range: std::ops::RangeInclusive<f32>, decimals: usize) {
    let mut shown = value;
    greyed(ui, |ui| crate::widgets::slider(ui, label, &mut shown, range, decimals));
}
