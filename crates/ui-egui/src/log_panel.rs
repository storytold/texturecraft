//! The Log panel: the session's messages under a General and an Assets tab, one per channel

use std::hash::{Hash, Hasher};

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use texturecraft_engine::log::{LogEntry, LogLevel};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::theme::Tokens;

mod log_channel;
mod log_width_cache;

pub use log_channel::LogChannel;
pub use log_width_cache::LogWidthCache;

/// The width of the History and Log lists' scrollbars
const LIST_SCROLLBAR_WIDTH: f32 = 8.0;
const LIST_SCROLLBAR_RADIUS: u8 = 3;
/// The air between a list's scrollbar and the list's edge, and between the rows and the scrollbar
const LIST_SCROLLBAR_OUTER_MARGIN: f32 = 2.0;
const LIST_SCROLLBAR_INNER_MARGIN: f32 = 1.0;

/// A newly focused row of a list scrolls in with a neighbour on each side: only shown rows are laid out, so Tab needs the next one there
pub(crate) fn reveal_focused_row(ui: &Ui, response: &egui::Response, row: egui::Rect) {
    if response.gained_focus() {
        // The shown width, so the list never moves sideways
        let around = egui::Rect::from_x_y_ranges(ui.clip_rect().x_range(), row.y_range()).expand2(egui::vec2(0.0, row.height()));
        ui.scroll_to_rect_animation(around, None, egui::style::ScrollAnimation::none());
    }
}

/// Show `total_rows` rows of `row_height` in `area` under a solid, always shown, themed scrollbar
pub(crate) fn panel_rows<R>(
    ui: &mut Ui,
    area: egui::ScrollArea,
    row_height: f32,
    total_rows: usize,
    add_rows: impl FnOnce(&mut Ui, std::ops::Range<usize>) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    let tokens = Tokens::get(ui.ctx());
    let row_visuals = ui.visuals().clone();
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.spacing_mut().scroll = egui::style::ScrollStyle {
            bar_width: LIST_SCROLLBAR_WIDTH,
            bar_outer_margin: LIST_SCROLLBAR_OUTER_MARGIN,
            bar_inner_margin: LIST_SCROLLBAR_INNER_MARGIN,
            ..egui::style::ScrollStyle::solid()
        };

        let visuals = ui.visuals_mut();
        visuals.extreme_bg_color = tokens.field;
        // The thumb stands 3:1 or more off its track in every theme, brighter under the pointer
        visuals.widgets.inactive.bg_fill = tokens.text_dim;
        visuals.widgets.hovered.bg_fill = tokens.text;
        visuals.widgets.active.bg_fill = tokens.text;
        for widget in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active] {
            widget.corner_radius = egui::CornerRadius::same(LIST_SCROLLBAR_RADIUS);
        }

        area.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible).show_rows(ui, row_height, total_rows, |ui, rows| {
            *ui.visuals_mut() = row_visuals;
            add_rows(ui, rows)
        })
    })
    .inner
}

/// The tab strip's and the list's inset from the panel's edges
const PANEL_INSET: f32 = 6.0;
const TAB_HEIGHT: f32 = 24.0;
const TAB_NAME_SIZE: f32 = 11.0;
/// The Assets tab's strip with its filter button, above the list
const FILTER_STRIP_HEIGHT: f32 = 30.0;
const LINE_HEIGHT: f32 = 19.0;
const LINE_TEXT_SIZE: f32 = 12.0;
/// Where a line's text starts in the list, and the air after the widest line
const LINE_TEXT_LEFT: f32 = 6.0;

/// The General and Assets tabs, splitting the panel's width; returns the open tab's rectangle
fn channel_tabs(app: &mut TextureCraftApp, ui: &mut Ui, tokens: &Tokens) -> Rect {
    // The panel's own margin is above the tabs; the strip adds the rest of the inset
    ui.add_space(PANEL_INSET - f32::from(crate::docking::LIST_PANEL_MARGIN.top));
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), TAB_HEIGHT), Sense::hover());
    let strip = strip.shrink2(vec2(PANEL_INSET, 0.0));
    let tab_width = strip.width() / LogChannel::ALL.len() as f32;
    let outline = Stroke::new(1.0, tokens.field_border);
    let mut open_tab = strip;
    for (slot, channel) in LogChannel::ALL.into_iter().enumerate() {
        let rect = Rect::from_min_size(strip.min + vec2(slot as f32 * tab_width, 0.0), vec2(tab_width, TAB_HEIGHT));
        let response = ui.interact(rect, ui.make_persistent_id(("log-tab", channel.name())), Sense::click());
        let open = app.ui.log_channel == channel;
        let name = tr(channel.name());
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, open, name));

        let painter = ui.painter();
        painter.rect(rect, 0.0, if open { tokens.field } else { tokens.row }, outline, StrokeKind::Inside);
        if open {
            open_tab = rect;
        }
        let color = if open || response.hovered() { tokens.text } else { tokens.text_dim };
        painter.text(rect.center(), Align2::CENTER_CENTER, name, crate::theme::semibold(TAB_NAME_SIZE), color);
        if response.has_focus() {
            crate::property_sheet::paint_focus_outline(painter, rect, tokens.text_strong);
        }

        if response.clicked() {
            app.ui.log_channel = channel;
            app.ui.log_selection.clear();
            app.ui.log_anchor = None;
        }
    }
    open_tab
}

/// A line as its row shows it: a message's line breaks become spaces, so it stays on its one row
fn row_text(entry: &LogEntry) -> String {
    entry.line().replace(['\r', '\n'], " ")
}

/// One Log line across the list: one row, never wrapped, banded while selected
fn log_line(ui: &mut Ui, tokens: &Tokens, index: usize, entry: &LogEntry, selected: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), LINE_HEIGHT), Sense::hover());

    // A row is as wide as the widest line: only its shown part takes clicks, so its middle is on screen
    let response = ui.interact(rect.intersect(ui.clip_rect()), ui.make_persistent_id(("log-line", index)), Sense::click());
    let line = entry.line();
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListItem, true, selected, &line));
    reveal_focused_row(ui, &response, rect);
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, tokens.accent_dim);
    }
    let text_at = egui::pos2(rect.left() + LINE_TEXT_LEFT, rect.center().y);
    painter.text(text_at, Align2::LEFT_CENTER, row_text(entry), egui::FontId::proportional(LINE_TEXT_SIZE), level_color(tokens, entry.level));
    if response.has_focus() {
        crate::property_sheet::paint_focus_outline(painter, rect, tokens.text_strong);
    }
    response
}

/// Changes when the installed fonts do: fallback fonts arrive while the app runs, and widths change with them
fn font_key(ui: &Ui) -> u64 {
    ui.fonts(|fonts| {
        let mut hasher = std::hash::DefaultHasher::new();
        fonts.definitions().families.get(&egui::FontFamily::Proportional).hash(&mut hasher);
        hasher.finish()
    })
}

/// The entries a tab lists, as indices into the session log
fn listed(app: &TextureCraftApp, channel: LogChannel) -> Vec<usize> {
    app.session.log.iter().enumerate().filter(|(_, entry)| channel_of(entry) == channel).map(|(index, _)| index).collect()
}

/// The channel a log entry belongs to: entries about assets go to the Assets tab
pub fn channel_of(entry: &LogEntry) -> LogChannel {
    if entry.is_asset() { LogChannel::Assets } else { LogChannel::General }
}

/// The line colour of a level: light grey for info, yellow-orange for warnings, red for errors
fn level_color(tokens: &Tokens, level: LogLevel) -> Color32 {
    match level {
        LogLevel::Info => tokens.text,
        LogLevel::Warning => tokens.warning,
        LogLevel::Error => tokens.danger,
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
    let tokens = Tokens::get(ui.ctx());
    let open_tab = channel_tabs(app, ui, &tokens);

    // A new session has a new log: indices chosen in the old one point at other lines
    if app.ui.log_id != app.session.log_id {
        app.ui.log_id = app.session.log_id;
        app.ui.log_selection.clear();
        app.ui.log_anchor = None;
    }

    // The list's box overlaps the tabs' bottom line, which is then cleared under the open tab; the panel's own margin is below it
    let inset = PANEL_INSET as i8;
    let outer = egui::Margin { left: inset, right: inset, top: -1, bottom: inset - crate::docking::LIST_PANEL_MARGIN.bottom };
    // The 1 px inner margin keeps the rows and scrollbars off the frame's line
    let frame = egui::Frame::new().fill(tokens.field).stroke(Stroke::new(1.0, tokens.field_border)).inner_margin(1).outer_margin(outer);
    frame.show(ui, |ui| entries(app, ui, &tokens));
    let joint = Rect::from_min_max(open_tab.left_bottom() + vec2(1.0, -1.0), open_tab.right_bottom() + vec2(-1.0, 0.0));
    ui.painter().rect_filled(joint, 0.0, tokens.field);
}

/// The shown tab's lines, under the Assets tab's filter button
fn entries(app: &mut TextureCraftApp, ui: &mut Ui, tokens: &Tokens) {
    if shown_assets_filter(app) {
        let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), FILTER_STRIP_HEIGHT), Sense::hover());
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        let mut strip_ui = ui.new_child(egui::UiBuilder::new().max_rect(strip.shrink2(vec2(PANEL_INSET, 0.0))).layout(layout));
        let filter = crate::unavailable::greyed(&mut strip_ui, |ui| ui.add(egui::Button::new(egui_phosphor::regular::WARNING).selected(false)));
        filter.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, false, tr("Warnings")));
        ui.painter().hline(strip.x_range(), strip.bottom(), Stroke::new(1.0, tokens.field_border));
    }

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

    // Lines never wrap: the list is as wide as its widest line and scrolls sideways
    let font = egui::FontId::proportional(LINE_TEXT_SIZE);
    let measure = |entry: &LogEntry| ui.painter().layout_no_wrap(row_text(entry), font.clone(), tokens.text).size().x;
    let widest = app.ui.log_widths.widest(&app.session.log, app.session.log_id, ui.ctx().pixels_per_point(), font_key(ui), shown, measure);
    let scroll_area = egui::ScrollArea::both().id_salt(("log", shown.name())).auto_shrink([false, false]).stick_to_bottom(true);
    panel_rows(ui, scroll_area, LINE_HEIGHT, rows.len(), |ui, range| {
        ui.set_min_width(widest + 2.0 * LINE_TEXT_LEFT);
        for index in rows[range].iter().copied() {
            let Some(entry) = app.session.log.get(index) else { continue };
            let response = log_line(ui, tokens, index, entry, app.ui.log_selection.contains(&index));
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
