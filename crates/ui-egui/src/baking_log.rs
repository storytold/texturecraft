//! The Baking Log panel: the latest bake under three tabs, read from the session's bake record

use egui::{Align2, CornerRadius, FontId, Id, Rect, Role, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::bake::{BakeLog, BakeLogMap, BakeLogMatching, BakeMapState};
use texturecraft_engine::core::BakedMap;

use crate::TextureCraftApp;
use crate::docking::Panel;
use crate::i18n::{baked_map_label, tr, trf};
use crate::property_sheet::{SIDE_PADDING_POINTS, TAB_GAP, TAB_ICON_CENTER, TAB_ICON_SIZE, TAB_NAME_LEFT, TAB_PADDING, TAB_TEXT_SIZE, TAB_TOP, shown_tab_box};
use crate::theme::{Tokens, semibold};

mod baking_log_tab;
mod high_poly_list;
mod matching_list;

pub use baking_log_tab::BakingLogTab;

/// Height of the tab strip, as the Properties section tabs
const STRIP_HEIGHT: f32 = 30.0;

/// How far the first tab reaches left of the rows' side padding, as the Properties section tabs
const FIRST_TAB_OUTDENT: f32 = 4.0;

/// Height of one row of a list
const ROW_HEIGHT: f32 = 24.0;

/// Air above the first row
const LIST_TOP: f32 = 4.0;

/// How far a hovered row's fill stays inside the panel's sides
const HOVER_INSET: f32 = 3.0;

/// From the panel's left edge to a row's glyph centre, and to its name
const GLYPH_CENTER: f32 = SIDE_PADDING_POINTS + 6.0;
const NAME_LEFT: f32 = SIDE_PADDING_POINTS + 22.0;

/// Size of a row's glyph and of its name
const GLYPH_SIZE: f32 = 13.0;
const NAME_SIZE: f32 = 12.0;

/// Size of the open-settings arrow at an issue row's right end, and of the square it answers clicks in
const OPEN_ICON_SIZE: f32 = 14.0;
const OPEN_BUTTON_SIZE: f32 = 20.0;

/// Height of the issues filter toggle, and its width without a count
const FILTER_HEIGHT: f32 = 22.0;
const FILTER_BUTTON_WIDTH: f32 = 30.0;

/// How far the issues filter sits below the strip's centre, and how far it reaches past the side padding
const FILTER_DROP: f32 = 1.0;
const FILTER_OUTDENT: f32 = 4.0;

/// Corner rounding of the issues filter's pressed and hovered fill
const FILTER_ROUNDING: f32 = 5.0;

/// From the issues filter's left edge to its glyph's centre and to its count, and the glyph's size
const FILTER_GLYPH_CENTER: f32 = 15.0;
const FILTER_COUNT_LEFT: f32 = 26.0;
const FILTER_GLYPH_SIZE: f32 = 14.0;

/// Air between the issue count and the toggle's right end
const FILTER_COUNT_GAP: f32 = 4.0;

/// Height of the two lines the issues filter leaves when nothing is wrong, to centre them
const EMPTY_TEXT_HEIGHT: f32 = 40.0;

pub(crate) fn body(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), STRIP_HEIGHT), Sense::hover());
    ui.painter().rect_filled(strip, CornerRadius::ZERO, tokens.chrome);

    // The tabs keep clear of the issues filter at its widest, so picking a tab never moves the edge they scroll under
    let filter_room = BakingLogTab::ALL.into_iter().map(|tab| filter_width(ui, issue_count(&app.session.bake_log, tab))).fold(0.0, f32::max);
    let tabs_right = strip.right() - SIDE_PADDING_POINTS + FILTER_OUTDENT - filter_room - TAB_GAP;
    if let Some(tab) = tabs(ui, strip.with_max_x(tabs_right.max(strip.left())), app.ui.baking_log_tab, &app.session.bake_log) {
        app.ui.baking_log_tab = tab;
    }
    let tab = app.ui.baking_log_tab;
    let issue_count = issue_count(&app.session.bake_log, tab);
    if issues_filter(ui, strip, issue_count, app.ui.baking_log_issues_only) {
        app.ui.baking_log_issues_only = !app.ui.baking_log_issues_only;
    }

    let issues_only = app.ui.baking_log_issues_only;
    let log = &app.session.bake_log;
    let baked = !log.maps.is_empty();
    if tab == BakingLogTab::MatchingByName && baked && log.matching.is_none() {
        matching_not_enabled(ui);
        return;
    }

    if issues_only && issue_count == 0 {
        no_issues(ui, tab);
        return;
    }

    let opened = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Rows sit edge to edge at ROW_HEIGHT, as the matching list's rules need
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(LIST_TOP);
            match tab {
                BakingLogTab::BakingProcess => baking_process(ui, &app.session.bake_log.maps, issues_only),
                BakingLogTab::HighPolyParameters => {
                    high_poly_list::high_poly_list(ui, &app.session.bake_log.highs, issues_only);
                    None
                }
                BakingLogTab::MatchingByName => {
                    if let Some(matching) = &app.session.bake_log.matching {
                        matching_list::matching_list(ui, matching, issues_only);
                    }
                    None
                }
            }
        })
        .inner;
    if let Some((texture_set, map)) = opened {
        open_map_settings(app, texture_set, map);
    }
}

/// Show `map`'s settings of `texture_set` in Mesh Map Settings, as picking it in Mesh Map Bakers does
fn open_map_settings(app: &mut TextureCraftApp, texture_set: usize, map: BakedMap) {
    // A guard against a row naming a set the session no longer has
    if texture_set >= app.session.set_count() {
        return;
    }

    if texture_set != app.session.ts() {
        app.run("textureSet.select", json!({ "ts": texture_set }));
    }
    app.ui.selected_baked_map = Some(map);
    app.ui.dock.focus(Panel::MeshMapSettings);
}

/// How many rows of `tab` have an issue, for the issues filter's count
fn issue_count(log: &BakeLog, tab: BakingLogTab) -> usize {
    match tab {
        BakingLogTab::BakingProcess => log.maps.iter().filter(|row| row.state.is_issue()).count(),
        BakingLogTab::HighPolyParameters => log.highs.iter().filter(|row| row.state.is_issue()).count(),
        BakingLogTab::MatchingByName => log.matching.as_ref().map_or(0, BakeLogMatching::issue_count),
    }
}

/// The tabs on the `strip` along the panel's top edge, as the Properties section tabs; they scroll sideways when wider than it
fn tabs(ui: &mut Ui, strip: Rect, shown: BakingLogTab, log: &BakeLog) -> Option<BakingLogTab> {
    let tokens = Tokens::get(ui.ctx());
    let names = BakingLogTab::ALL.map(|tab| tr(tab.name()));
    let widths = names.map(|name| ui.painter().layout_no_wrap(name.to_owned(), semibold(TAB_TEXT_SIZE), tokens.text).size().x + TAB_PADDING);
    let first_left = SIDE_PADDING_POINTS - FIRST_TAB_OUTDENT;
    let content_width = first_left + widths.iter().sum::<f32>() + TAB_GAP * (widths.len() - 1) as f32;

    let mut strip_ui = ui.new_child(egui::UiBuilder::new().max_rect(strip).id_salt("baking_log_tabs"));
    egui::ScrollArea::horizontal()
        .id_salt("baking_log_tabs")
        .auto_shrink([false, false])
        .show(&mut strip_ui, |ui| {
            let (content, _) = ui.allocate_exact_size(vec2(content_width, strip.height()), Sense::hover());
            let mut picked = None;
            let mut left = content.left() + first_left;
            for ((tab, name), width) in BakingLogTab::ALL.into_iter().zip(names).zip(widths) {
                if tab_button(ui, content, left, width, (tab, name), tab == shown, log) {
                    picked = Some(tab);
                }
                left += width + TAB_GAP;
            }
            picked
        })
        .inner
}

/// One tab `width` wide from `left` on the `strip`, scrolled into view when the focus reaches it; dots for its icon while its work runs
fn tab_button(ui: &mut Ui, strip: Rect, left: f32, width: f32, (tab, name): (BakingLogTab, &str), on: bool, log: &BakeLog) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let rect = Rect::from_min_max(pos2(left, strip.top() + TAB_TOP), pos2(left + width, strip.bottom()));
    let response = ui.interact(rect, ui.scope_id().with(("baking_log_tab", tab)), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Tab, true, on, name));
    if response.gained_focus() {
        response.scroll_to_me(None);
    }

    let painter = ui.painter();
    if on {
        shown_tab_box(painter, &tokens, rect);
    }
    let color = if on || response.hovered() { tokens.text } else { tokens.text_dim };
    let in_progress = match tab {
        BakingLogTab::BakingProcess => log.is_running(),
        BakingLogTab::HighPolyParameters => log.is_loading_high_polys(),
        BakingLogTab::MatchingByName => false,
    };
    let (icon, icon_color) = if in_progress { (ic::DOTS_THREE, tokens.accent) } else { (tab.icon(), color) };
    let center_y = strip.center().y + TAB_TOP / 2.0;
    painter.text(pos2(left + TAB_ICON_CENTER, center_y), Align2::CENTER_CENTER, icon, FontId::proportional(TAB_ICON_SIZE), icon_color);
    let font = if on { semibold(TAB_TEXT_SIZE) } else { FontId::proportional(TAB_TEXT_SIZE) };
    painter.text(pos2(left + TAB_NAME_LEFT, center_y), Align2::LEFT_CENTER, name, font, color);
    if response.has_focus() {
        crate::property_sheet::paint_focus_outline(painter, response.rect, tokens.text_strong);
    }
    response.clicked()
}

/// The issues filter's width when it counts `issue_count` rows
fn filter_width(ui: &Ui, issue_count: usize) -> f32 {
    let count_width = if issue_count == 0 {
        0.0
    } else {
        let tokens = Tokens::get(ui.ctx());
        ui.painter().layout_no_wrap(issue_count.to_string(), semibold(NAME_SIZE), tokens.text).size().x + FILTER_COUNT_GAP
    };
    FILTER_BUTTON_WIDTH + count_width
}

/// The toggle at the `strip`'s right end that keeps only the rows with an issue: a warning glyph and how many there are
fn issues_filter(ui: &mut Ui, strip: Rect, issue_count: usize, on: bool) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let count = (issue_count > 0).then(|| issue_count.to_string());
    let right = strip.right() - SIDE_PADDING_POINTS + FILTER_OUTDENT;
    let center_y = strip.center().y + FILTER_DROP;
    let rect = Rect::from_min_max(pos2(right - filter_width(ui, issue_count), center_y - FILTER_HEIGHT / 2.0), pos2(right, center_y + FILTER_HEIGHT / 2.0));

    let name = match &count {
        Some(count) => trf("{count}, Show only rows with an issue", &[("count", count)]),
        None => tr("Show only rows with an issue").to_owned(),
    };
    let response = ui.interact(rect, ui.scope_id().with("baking_log_issues_filter"), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, on, &name));

    let painter = ui.painter();
    if on {
        painter.rect(rect, FILTER_ROUNDING, tokens.tool_active, Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
    } else if response.hovered() {
        painter.rect_filled(rect, FILTER_ROUNDING, tokens.row_hover);
    }
    let glyph_color = if count.is_some() { tokens.warning } else { tokens.text_dim };
    painter.text(pos2(rect.left() + FILTER_GLYPH_CENTER, center_y), Align2::CENTER_CENTER, ic::WARNING, FontId::proportional(FILTER_GLYPH_SIZE), glyph_color);
    if let Some(count) = count {
        painter.text(pos2(rect.left() + FILTER_COUNT_LEFT, center_y), Align2::LEFT_CENTER, count, semibold(NAME_SIZE), tokens.text);
    }
    if response.has_focus() {
        crate::property_sheet::paint_focus_outline(painter, rect, tokens.text_strong);
    }
    response.on_hover_text(tr("Show only rows with an issue")).clicked()
}

/// What the issues filter leaves when `tab` has no issue: one line for that tab, and how to see every row
fn no_issues(ui: &mut Ui, tab: BakingLogTab) {
    let tokens = Tokens::get(ui.ctx());
    let message = match tab {
        BakingLogTab::BakingProcess => tr("Every map so far baked without a problem."),
        BakingLogTab::HighPolyParameters => tr("Every high poly file so far loaded."),
        BakingLogTab::MatchingByName => tr("Every part so far has its partner."),
    };
    let area = ui.available_rect_before_wrap();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Center)));
    child.add_space((area.height() - EMPTY_TEXT_HEIGHT).max(0.0) / 2.0);
    child.label(egui::RichText::new(message).font(semibold(NAME_SIZE)).color(tokens.text));
    child.label(egui::RichText::new(tr("Turn off the issues filter to see every row.")).size(NAME_SIZE).color(tokens.text_dim));
    ui.allocate_rect(area, Sense::hover());
}

/// What the Matching by Name tab says when no texture set of the latest bake matched by mesh name: the setting and its value in SemiBold
fn matching_not_enabled(ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    let mut text = egui::text::LayoutJob::default();
    let format = |font| egui::TextFormat { font_id: font, color: tokens.text, ..Default::default() };

    // One sentence to translate, its setting and value kept apart to set them in SemiBold
    let sentence = tr("Set {setting} to {value} to pair parts by name.");
    let (before, rest) = sentence.split_once("{setting}").unwrap_or(("", sentence));
    let (between, after) = rest.split_once("{value}").unwrap_or((rest, ""));
    text.append(before, 0.0, format(FontId::proportional(NAME_SIZE)));
    text.append(tr("Match"), 0.0, format(semibold(NAME_SIZE)));
    text.append(between, 0.0, format(FontId::proportional(NAME_SIZE)));
    text.append(tr("By Mesh Name"), 0.0, format(semibold(NAME_SIZE)));
    text.append(after, 0.0, format(FontId::proportional(NAME_SIZE)));

    let area = ui.available_rect_before_wrap();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Center)));
    child.add_space((area.height() - EMPTY_TEXT_HEIGHT / 2.0).max(0.0) / 2.0);
    child.label(text);
    ui.allocate_rect(area, Sense::hover());
}

/// One row per texture set and map (with an issue, if `issues_only`) in bake order, following the map being baked; returns the map to open
fn baking_process(ui: &mut Ui, maps: &[BakeLogMap], issues_only: bool) -> Option<(usize, BakedMap)> {
    let followed = maps.iter().position(|row| row.state == BakeMapState::Baking);
    let follow_id = Id::unique("baking_log_followed_map");
    let newly_followed = followed.is_some() && ui.data(|data| data.get_temp::<Option<usize>>(follow_id)) != Some(followed);
    ui.data_mut(|data| data.insert_temp(follow_id, followed));

    let mut opened = None;
    for (index, row) in maps.iter().enumerate().filter(|(_, row)| !issues_only || row.state.is_issue()) {
        let (rect, open_clicked) = map_row(ui, row);

        // Bring the next map into view too, so the list shows what comes after the map being baked
        if newly_followed && followed == Some(index) {
            ui.scroll_to_rect(rect.with_max_y(rect.bottom() + ROW_HEIGHT), None);
        }
        if open_clicked {
            opened = Some((row.texture_set, row.map));
        }
    }
    opened
}

/// A map's row: glyph, then "<texture set> <map>"; a map not baked tells why on hover and has an arrow to its settings; returns the row and whether the arrow was clicked
fn map_row(ui: &mut Ui, row: &BakeLogMap) -> (Rect, bool) {
    let tokens = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let name = format!("{} {}", row.set_name, tr(baked_map_label(row.map)));
    let (glyph, glyph_color, state) = match &row.state {
        BakeMapState::Waiting => (ic::CIRCLE_DASHED, tokens.text_dim, tr("Waiting")),
        BakeMapState::Baking => (ic::DOTS_THREE, tokens.accent, tr("Baking")),
        BakeMapState::Baked => (ic::CHECK, tokens.text, tr("Baked")),
        BakeMapState::Failed(_) => (ic::X_CIRCLE, tokens.danger, tr("Not baked")),
        BakeMapState::Canceled => (ic::WARNING, tokens.warning, tr("Canceled")),
    };
    let reason = match &row.state {
        BakeMapState::Failed(reason) => Some(reason.as_str()),
        BakeMapState::Canceled => Some(tr("The bake was canceled before this map was kept.")),
        _ => None,
    };
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, format!("{name}, {state}")));
    describe(ui, &response, reason);
    let open = row.state.is_issue().then(|| open_settings_button(ui, rect, (row.texture_set, row.map), &name));
    let row_hovered = response.hovered() || open.as_ref().is_some_and(|open| open.hovered());

    let painter = ui.painter();
    if row_hovered {
        painter.rect_filled(rect.shrink2(vec2(HOVER_INSET, 0.0)), CornerRadius::ZERO, tokens.row_hover);
    }
    let center_y = rect.center().y;
    painter.text(pos2(rect.left() + GLYPH_CENTER, center_y), Align2::CENTER_CENTER, glyph, FontId::proportional(GLYPH_SIZE), glyph_color);
    let (font, color) = match row.state {
        BakeMapState::Baking => (semibold(NAME_SIZE), tokens.text),
        BakeMapState::Waiting => (FontId::proportional(NAME_SIZE), tokens.text_dim),
        _ => (FontId::proportional(NAME_SIZE), tokens.text),
    };
    painter.text(pos2(rect.left() + NAME_LEFT, center_y), Align2::LEFT_CENTER, &name, font, color);
    if let Some(open) = &open
        && (row_hovered || open.has_focus())
    {
        let color = if open.hovered() || open.has_focus() { tokens.text } else { tokens.text_dim };
        painter.text(open.rect.center(), Align2::CENTER_CENTER, ic::ARROW_SQUARE_OUT, FontId::proportional(OPEN_ICON_SIZE), color);
        if open.has_focus() {
            crate::property_sheet::paint_focus_outline(painter, open.rect, tokens.text_strong);
        }
    }

    if let Some(reason) = reason {
        response.on_hover_ui(|ui| {
            ui.label(egui::RichText::new(state).font(semibold(NAME_SIZE)).color(tokens.text));
            ui.label(egui::RichText::new(reason).size(NAME_SIZE).color(tokens.text_dim));
        });
    }
    let open_clicked = open.is_some_and(|open| open.clicked());
    (rect, open_clicked)
}

/// Give a row's node the `reason` its tooltip shows, so a screen reader reads it without hovering
pub(super) fn describe(ui: &Ui, response: &egui::Response, reason: Option<&str>) {
    if let Some(reason) = reason {
        ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(reason));
    }
}

/// The open-settings arrow's square at the right end of `row`; it shows only while the row is hovered or it has focus
fn open_settings_button(ui: &mut Ui, row: Rect, set_map: (usize, BakedMap), name: &str) -> egui::Response {
    let center = pos2(row.right() - SIDE_PADDING_POINTS - OPEN_BUTTON_SIZE / 2.0, row.center().y);
    let rect = Rect::from_center_size(center, vec2(OPEN_BUTTON_SIZE, OPEN_BUTTON_SIZE));
    let label = trf("Open the {name} settings", &[("name", &name)]);
    let response = ui.interact(rect, ui.scope_id().with(("baking_log_open", set_map)), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, &label));
    response.on_hover_text(label)
}
