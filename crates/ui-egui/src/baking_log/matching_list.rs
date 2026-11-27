//! The Baking Log's Matching by Name tab: which low poly part the latest bake paired with which high poly part

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Role, Sense, Stroke, Ui, WidgetInfo, pos2, vec2};
use egui_phosphor::regular as ic;
use texturecraft_engine::bake::{BakeLogMatching, NamePair};

use super::{GLYPH_SIZE, HOVER_INSET, NAME_SIZE, ROW_HEIGHT};
use crate::i18n::{tr, trf};
use crate::property_sheet::SIDE_PADDING_POINTS;
use crate::theme::{Tokens, semibold};
use crate::widgets::elided;

/// Size of the column captions
const CAPTION_SIZE: f32 = 11.0;

/// Air between the end of a low poly name and the high poly column
const COLUMN_GAP: f32 = 8.0;

/// The two columns' captions and a row per pair (only the parts with no partner, if `issues_only`), each ruled off below
pub(super) fn matching_list(ui: &mut Ui, matching: &BakeLogMatching, issues_only: bool) {
    let tokens = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let captions = [caption(tr("Low poly mesh ({suffix})"), &matching.low_suffix), caption(tr("High poly mesh ({suffix})"), &matching.high_suffix)];
    for (column, caption) in columns(rect).into_iter().zip(captions) {
        column_text(ui, column, caption, CAPTION_SIZE, tokens.text_dim);
    }
    rule(ui, rect);

    for pair in matching.pairs.iter().filter(|pair| !issues_only || pair.unpaired()) {
        pair_row(ui, pair);
    }
}

/// A column caption in capitals, with the set's `suffix` kept as typed
fn caption(template: &str, suffix: &str) -> String {
    template.split("{suffix}").map(str::to_uppercase).collect::<Vec<_>>().join(suffix)
}

/// The low and high poly columns of `row`, each ending a gap short of the next column or the side padding
fn columns(row: Rect) -> [Rect; 2] {
    let low = row.with_min_x(row.left() + SIDE_PADDING_POINTS).with_max_x(row.center().x - COLUMN_GAP);
    let high = row.with_min_x(row.center().x).with_max_x(row.right() - SIDE_PADDING_POINTS);
    [low, high]
}

/// Paint `text` in `color` at the left end of `column`, centred on its height and cut short to its width; whether it was cut
fn column_text(ui: &Ui, column: Rect, text: String, size: f32, color: Color32) -> bool {
    let galley = elided(ui.painter(), text, size, color, column.width());
    let cut_short = galley.elided;
    ui.painter().galley(pos2(column.left(), column.center().y - galley.size().y / 2.0), galley, color);
    cut_short
}

/// The hairline under `row`, inside the side padding
fn rule(ui: &Ui, row: Rect) {
    let tokens = Tokens::get(ui.ctx());
    let x = row.left() + SIDE_PADDING_POINTS..=row.right() - SIDE_PADDING_POINTS;
    ui.painter().hline(x, row.bottom() - 0.5, Stroke::new(1.0, tokens.separator));
}

/// A pair's row: the low poly part, then the high poly part; a side with no partner is a warning glyph that tells why on hover
fn pair_row(ui: &mut Ui, pair: &NamePair) {
    let tokens = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::hover());
    let no_match = tr("No match");
    let name = format!("{}, {}", pair.low.as_deref().unwrap_or(no_match), pair.high.as_deref().unwrap_or(no_match));
    let reason = match (&pair.low, &pair.high) {
        (Some(low), None) => Some(trf("No high poly part pairs with {part} by name, so it bakes from none.", &[("part", low)])),
        (None, Some(high)) => Some(trf("No low poly part pairs with {part} by name, so no texture set bakes from it.", &[("part", high)])),
        _ => None,
    };
    response.widget_info(|| WidgetInfo::labeled(Role::Label, true, &name));
    super::describe(ui, &response, reason.as_deref());

    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect.shrink2(vec2(HOVER_INSET, 0.0)), CornerRadius::ZERO, tokens.row_hover);
    }
    let mut cut_short = false;
    for (column, part) in columns(rect).into_iter().zip([&pair.low, &pair.high]) {
        match part {
            Some(part) => cut_short |= column_text(ui, column, part.clone(), NAME_SIZE, tokens.text),
            None => {
                painter.text(pos2(column.left(), column.center().y), Align2::LEFT_CENTER, ic::WARNING, FontId::proportional(GLYPH_SIZE), tokens.warning);
            }
        }
    }
    rule(ui, rect);

    // A name cut short shows in full on hover; the row's label carries it in full already
    if reason.is_some() || cut_short {
        response.on_hover_ui(|ui| {
            if cut_short {
                ui.label(egui::RichText::new(&name).size(NAME_SIZE).color(tokens.text));
            }
            if let Some(reason) = reason {
                ui.label(egui::RichText::new(no_match).font(semibold(NAME_SIZE)).color(tokens.text));
                ui.label(egui::RichText::new(reason).size(NAME_SIZE).color(tokens.text_dim));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names longer than their column, in a list as narrow as a small docked log
    #[test]
    fn long_names_stay_in_their_own_column() {
        let long = |side: &str| format!("Left_shoulder_strap_buckle_with_its_rivets_and_stitching_{side}");
        let matching = BakeLogMatching {
            low_suffix: "_low".into(),
            high_suffix: "_high".into(),
            pairs: vec![NamePair { low: Some(long("low")), high: Some(long("high")) }],
        };
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(320.0, 200.0));
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| matching_list(ui, &matching, false));
        output.textures_delta.clear();

        let texts: Vec<Rect> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.visual_bounding_rect()),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 4, "two captions and two names");
        for (index, text) in texts.iter().enumerate() {
            assert!(screen.contains_rect(*text), "text {index} {text:?} stays inside the list");
            for other in &texts[index + 1..] {
                assert!(!text.intersects(*other), "text {index} {text:?} runs into {other:?}");
            }
        }
    }
}
