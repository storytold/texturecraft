//! The Baking Log's High Poly Parameters tab: each texture set of the latest bake and the high poly files it loads

use egui::{Align2, CornerRadius, FontId, Id, Role, Sense, Ui, WidgetInfo, pos2, vec2};
use egui_phosphor::regular as ic;
use texturecraft_engine::bake::{BakeHighState, BakeLogHigh};

use super::{GLYPH_CENTER, GLYPH_SIZE, HOVER_INSET, NAME_LEFT, NAME_SIZE, ROW_HEIGHT};
use crate::i18n::{tr, trf};
use crate::theme::{Tokens, semibold};

/// How far a file's row sits right of its texture set's row
const FILE_INDENT: f32 = 22.0;

/// A row per texture set that loads high polys, folding open onto a row per file (with an issue, if `issues_only`)
pub(super) fn high_poly_list(ui: &mut Ui, highs: &[BakeLogHigh], issues_only: bool) {
    let mut start = 0;

    while start < highs.len() {
        let set = highs[start].texture_set;
        let end = highs[start..].iter().position(|row| row.texture_set != set).map_or(highs.len(), |length| start + length);
        let files: Vec<&BakeLogHigh> = highs[start..end].iter().filter(|row| !issues_only || row.state.is_issue()).collect();
        start = end;

        if files.is_empty() {
            continue;
        }

        if set_row(ui, set, &files[0].set_name) {
            for row in files {
                file_row(ui, row);
            }
        }
    }
}

/// A texture set's row: a caret and its name in SemiBold, named for what it holds; a click folds it. Returns whether it is open
fn set_row(ui: &mut Ui, set: usize, name: &str) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let id = Id::unique(("baking_log_high_poly_set", set));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(Role::Button, true, trf("{set}, High poly files", &[("set", &name)])));

    let mut open = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(true);
    if response.clicked() {
        open = !open;
        ui.data_mut(|data| data.insert_temp(id, open));
    }

    ui.ctx().accesskit_node_builder(response.id, |node| node.set_expanded(open));

    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect.shrink2(vec2(HOVER_INSET, 0.0)), CornerRadius::ZERO, tokens.row_hover);
    }

    let center_y = rect.center().y;
    let caret = if open { ic::CARET_DOWN } else { ic::CARET_RIGHT };
    painter.text(pos2(rect.left() + GLYPH_CENTER, center_y), Align2::CENTER_CENTER, caret, FontId::proportional(GLYPH_SIZE), tokens.text_dim);
    painter.text(pos2(rect.left() + NAME_LEFT, center_y), Align2::LEFT_CENTER, name, semibold(NAME_SIZE), tokens.text);
    if response.has_focus() {
        crate::property_sheet::paint_focus_outline(painter, rect, tokens.text_strong);
    }

    open
}

/// A high poly file's row under its texture set: glyph and file name; a file not loaded tells why on hover
fn file_row(ui: &mut Ui, row: &BakeLogHigh) {
    let tokens = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let (glyph, glyph_color, state) = match &row.state {
        BakeHighState::Waiting => (ic::CIRCLE_DASHED, tokens.text_dim, tr("Waiting")),
        BakeHighState::Loading => (ic::DOTS_THREE, tokens.accent, tr("Loading")),
        BakeHighState::Loaded => (ic::CHECK, tokens.text, tr("Loaded")),
        BakeHighState::Failed(_) => (ic::X_CIRCLE, tokens.danger, tr("Not loaded")),
        BakeHighState::Canceled => (ic::WARNING, tokens.warning, tr("Canceled")),
    };
    let reason = match &row.state {
        BakeHighState::Failed(reason) => Some(reason.as_str()),
        BakeHighState::Canceled => Some(tr("The bake was canceled before this file was loaded.")),
        _ => None,
    };

    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, format!("{}, {state}", row.file)));
    super::describe(ui, &response, reason);

    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect.shrink2(vec2(HOVER_INSET, 0.0)), CornerRadius::ZERO, tokens.row_hover);
    }

    let center_y = rect.center().y;
    painter.text(pos2(rect.left() + GLYPH_CENTER + FILE_INDENT, center_y), Align2::CENTER_CENTER, glyph, FontId::proportional(GLYPH_SIZE), glyph_color);
    let (font, color) = match row.state {
        BakeHighState::Loading => (semibold(NAME_SIZE), tokens.text),
        BakeHighState::Waiting => (FontId::proportional(NAME_SIZE), tokens.text_dim),
        _ => (FontId::proportional(NAME_SIZE), tokens.text),
    };
    painter.text(pos2(rect.left() + NAME_LEFT + FILE_INDENT, center_y), Align2::LEFT_CENTER, &row.file, font, color);

    if let Some(reason) = reason {
        response.on_hover_ui(|ui| {
            ui.label(egui::RichText::new(state).font(semibold(NAME_SIZE)).color(tokens.text));
            ui.label(egui::RichText::new(reason).size(NAME_SIZE).color(tokens.text_dim));
        });
    }
}
