//! The Log panel: the session's messages under a General and an Assets tab, one per channel

use egui::{Color32, RichText, Ui};
use texturecraft_engine::log::{LogEntry, LogLevel};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::theme::Tokens;

mod log_channel;

pub use log_channel::LogChannel;

/// The entries a tab lists, as indices into the session log
fn listed(app: &TextureCraftApp, channel: LogChannel) -> Vec<usize> {
    app.session.log.iter().enumerate().filter(|(_, entry)| channel_of(entry) == channel).map(|(index, _)| index).collect()
}

/// The channel a log entry belongs to: entries about assets go to the Assets tab
pub fn channel_of(entry: &LogEntry) -> LogChannel {
    if entry.is_asset() { LogChannel::Assets } else { LogChannel::General }
}

/// The line colour of a level: light grey for info, yellow-orange for warnings, red for errors
fn level_color(tk: &Tokens, level: LogLevel) -> Color32 {
    match level {
        LogLevel::Info => tk.text,
        LogLevel::Warning => tk.warning,
        LogLevel::Error => tk.danger,
    }
}

/// The plain text of the selected lines, one per row
pub(crate) fn selected_text(app: &TextureCraftApp) -> String {
    let lines: Vec<String> = app.ui.log_selection.iter().filter_map(|index| app.session.log.get(*index)).map(LogEntry::line).collect();
    lines.join("\n")
}

/// Apply a click on row `index`: Ctrl toggles it, Shift extends from the last clicked row, a
/// plain click selects only it
fn select(app: &mut TextureCraftApp, shown: &[usize], index: usize, modifiers: egui::Modifiers) {
    let anchor = app.ui.log_anchor.filter(|anchor| shown.contains(anchor));
    if modifiers.shift
        && let Some(anchor) = anchor
    {
        let (from, to) = (shown.iter().position(|i| *i == anchor).unwrap_or(0), shown.iter().position(|i| *i == index).unwrap_or(0));
        let (low, high) = (from.min(to), from.max(to));
        if !modifiers.command {
            app.ui.log_selection.clear();
        }
        app.ui.log_selection.extend(shown[low..=high].iter().copied());
        return;
    }

    if modifiers.command {
        if !app.ui.log_selection.remove(&index) {
            app.ui.log_selection.insert(index);
        }
    } else {
        app.ui.log_selection.clear();
        app.ui.log_selection.insert(index);
    }
    app.ui.log_anchor = Some(index);
}

/// Remove the log entries `remove` says yes to; the selection is dropped, as its indices move
fn remove_entries(app: &mut TextureCraftApp, mut remove: impl FnMut(usize, &LogEntry) -> bool) {
    let mut index = 0;
    app.session.log.retain(|entry| {
        let drop = remove(index, entry);
        index += 1;
        !drop
    });
    app.ui.log_selection.clear();
    app.ui.log_anchor = None;
}

/// The right-click menu: Copy to clipboard, a separator, then Clear (General) or Clear all and
/// Clear selected (Assets)
fn context_menu(app: &mut TextureCraftApp, ui: &mut Ui, shown: LogChannel) {
    let copy = egui::Button::new(tr("Copy to clipboard")).shortcut_text("Ctrl+C");
    if ui.add(copy).clicked() {
        let text = selected_text(app);
        ui.ctx().copy_text(text);
        ui.close();
    }
    ui.separator();
    match shown {
        LogChannel::General => {
            if ui.button(tr("Clear")).clicked() {
                remove_entries(app, |_, entry| channel_of(entry) == LogChannel::General);
                ui.close();
            }
        }
        LogChannel::Assets => {
            if ui.button(tr("Clear all")).clicked() {
                remove_entries(app, |_, entry| channel_of(entry) == LogChannel::Assets);
                ui.close();
            }
            let chosen = app.ui.log_selection.clone();
            if ui.button(tr("Clear selected")).clicked() {
                remove_entries(app, |index, entry| channel_of(entry) == LogChannel::Assets && chosen.contains(&index));
                ui.close();
            }
        }
    }
}

fn shown_assets_filter(app: &TextureCraftApp) -> bool {
    app.ui.log_channel == LogChannel::Assets
}

fn rect_id() -> egui::Id {
    egui::Id::unique("log_panel_rect")
}

/// Whether this frame's Copy event belongs to the Log: lines are selected and the pointer is over
/// the panel. The app's layer copy must not run on the same Ctrl+C.
pub(crate) fn takes_copy(app: &TextureCraftApp, ctx: &egui::Context) -> bool {
    if app.ui.log_id != app.session.log_id || app.ui.log_selection.is_empty() {
        return false;
    }

    let over = ctx
        .data(|data| data.get_temp::<(egui::Rect, egui::LayerId, u64)>(rect_id()))
        .filter(|(_, _, pass)| pass.saturating_add(2) >= ctx.cumulative_pass_nr())
        .is_some_and(|(rect, layer, _)| ctx.rect_contains_pointer(layer, rect));
    over && ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Copy)))
}

/// The Log panel body (a dock tab or rail popup)
pub(crate) fn body(app: &mut TextureCraftApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for channel in LogChannel::ALL {
            if ui.selectable_label(app.ui.log_channel == channel, tr(channel.name())).clicked() {
                app.ui.log_channel = channel;
                app.ui.log_selection.clear();
                app.ui.log_anchor = None;
            }
        }
    });
    ui.separator();

    // A new session has a new log: indices chosen in the old one point at other lines
    if app.ui.log_id != app.session.log_id {
        app.ui.log_id = app.session.log_id;
        app.ui.log_selection.clear();
        app.ui.log_anchor = None;
    }

    if shown_assets_filter(app) {
        crate::unavailable::greyed(ui, |ui| ui.add(egui::Button::new(egui_phosphor::regular::WARNING).selected(false)));
    }

    let tk = Tokens::get(ui.ctx());
    let shown = app.ui.log_channel;
    let rows = listed(app, shown);

    // The empty space under the lines has the same menu as a line; the lines sit on top of it
    let area = ui.available_rect_before_wrap();
    let pass = ui.ctx().cumulative_pass_nr();
    ui.ctx().data_mut(|data| data.insert_temp(rect_id(), (area, ui.layer_id(), pass)));
    let background = ui.interact(area, ui.make_persistent_id(("log-background", shown.name())), egui::Sense::click());
    background.context_menu(|ui| context_menu(app, ui, shown));
    if ui.rect_contains_pointer(area) && ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Copy))) {
        let text = selected_text(app);
        if !text.is_empty() {
            ui.ctx().copy_text(text);
        }
    }

    egui::ScrollArea::vertical().id_salt(("log", shown.name())).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        for index in rows.iter().copied() {
            let Some(entry) = app.session.log.get(index) else { continue };
            let text = RichText::new(entry.line()).color(level_color(&tk, entry.level));
            let band = ui.painter().add(egui::Shape::Noop);
            let response = ui.add(egui::Label::new(text).selectable(false).sense(egui::Sense::click()));
            if app.ui.log_selection.contains(&index) {
                let rect = response.rect.expand2(egui::vec2(ui.spacing().item_spacing.x, 0.0)).intersect(ui.max_rect());
                ui.painter().set(band, egui::Shape::rect_filled(rect, 0.0, tk.accent_dim));
            }
            if response.clicked() || response.secondary_clicked() && !app.ui.log_selection.contains(&index) {
                let modifiers = ui.input(|input| input.modifiers);
                select(app, &rows, index, modifiers);
            }
            response.context_menu(|ui| context_menu(app, ui, shown));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_picks_the_channel() {
        assert_eq!(channel_of(&LogEntry::new(LogLevel::Error, "Library agent", "resource stone.png: unsupported format")), LogChannel::Assets);
        assert_eq!(channel_of(&LogEntry::new(LogLevel::Info, "Project", "Saved /tmp/a.tcraft")), LogChannel::General);
        assert_eq!(channel_of(&LogEntry::new(LogLevel::Info, "Exporter", "Exported 4 textures to /tmp")), LogChannel::General);
    }
}
