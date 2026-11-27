//! A list of loaded files under its label, one row per file, with its add button beside it

use egui::{Align2, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, WidgetInfo, pos2, vec2};
use egui_phosphor::regular as ic;

use super::FileListAdd;
use super::stacked_rows::{LABEL_LINE, STACK_ROW_AIR};
use super::{CONTROL_RADIUS, FileListResponse, FileRow, PropertySheet, SMALL_TEXT_SIZE, TEXT_SIZE};
use crate::theme::semibold;

/// One file's row, and the list's inner margin above and below its rows
const FILE_ROW_HEIGHT: f32 = 40.0;
const LIST_MARGIN: f32 = 2.0;

/// The add button's square and the room it takes beside the list
const ADD_BUTTON_SIZE: f32 = 28.0;
const ADD_BUTTON_ROOM: f32 = 34.0;

/// Where a row's cube sits, where its words start, and its × from the list's right edge
const CUBE_CENTER: f32 = 16.0;
const CUBE_SIZE: f32 = 14.0;
const NAME_LEFT: f32 = 32.0;
const REMOVE_INSET: f32 = 14.0;
const REMOVE_SIZE: f32 = 12.0;
const REMOVE_BOX: f32 = 18.0;

/// The name's and the folder's centres from the row's top, when the row shows both
const NAME_CENTER: f32 = 13.0;
const FOLDER_CENTER: f32 = 28.0;

/// The hairline between rows stops this far from the list's sides
const DIVIDER_INSET: f32 = 8.0;

/// Size of the add button's glyph
const ADD_GLYPH_SIZE: f32 = 14.0;

/// What stands for the folders left out of a shortened path
const ELLIPSIS: &str = "…";

impl PropertySheet<'_> {
    /// `rows` in a box at least `min_rows` tall, with the `add` button beside it; each loaded file's × (`remove_label`) asks to remove it
    pub(crate) fn file_list(&mut self, label: &str, rows: &[FileRow<'_>], min_rows: usize, add: FileListAdd<'_>, remove_label: &str) -> FileListResponse {
        let line = self.take(LABEL_LINE);
        let tokens = self.tokens;
        self.ui.painter().text(pos2(self.left, line.center().y), Align2::LEFT_CENTER, label, FontId::proportional(TEXT_SIZE), tokens.text);

        let height = rows.len().max(min_rows) as f32 * FILE_ROW_HEIGHT + 2.0 * LIST_MARGIN;
        let area = self.take(height);
        let list = Rect::from_min_max(pos2(self.left, area.top()), pos2(self.right - ADD_BUTTON_ROOM, area.bottom()));
        let list_id = self.next_id();
        let list_response = self.ui.interact(list, list_id, Sense::hover());
        let enabled = self.ui.is_enabled();
        list_response.widget_info(|| WidgetInfo::labeled(egui::Role::List, enabled, label));
        self.ui.painter().rect(list, CornerRadius::same(CONTROL_RADIUS), tokens.field, Stroke::new(1.0, tokens.field_border), StrokeKind::Inside);

        let mut answer = FileListResponse::default();
        for (index, row) in rows.iter().enumerate() {
            let top = list.top() + LIST_MARGIN + index as f32 * FILE_ROW_HEIGHT;
            let rect = Rect::from_min_max(pos2(list.left(), top), pos2(list.right(), top + FILE_ROW_HEIGHT));
            if index > 0 {
                let divider = [pos2(list.left() + DIVIDER_INSET, top), pos2(list.right() - DIVIDER_INSET, top)];
                self.ui.painter().line_segment(divider, Stroke::new(1.0, tokens.separator));
            }
            if self.file_row(rect, row, remove_label) {
                answer.removed = Some(index);
            }
        }

        let add_rect = Rect::from_min_size(pos2(self.right - ADD_BUTTON_SIZE, area.top()), vec2(ADD_BUTTON_SIZE, ADD_BUTTON_SIZE));
        let add_id = self.next_id();
        let can_add = add.enabled && enabled;
        let add_response = self.ui.interact(add_rect, add_id, if can_add { Sense::click() } else { Sense::hover() });
        add_response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, can_add, add.label));
        let add_response = self.tip(add_response, add.label);
        let fill = if can_add && add_response.hovered() { tokens.row_hover } else { tokens.button };
        let glyph_color = if can_add { tokens.text } else { tokens.text_dim };
        let painter = self.ui.painter();
        painter.rect(add_rect, CornerRadius::same(CONTROL_RADIUS), fill, Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
        painter.text(add_rect.center(), Align2::CENTER_CENTER, add.glyph, FontId::proportional(ADD_GLYPH_SIZE), glyph_color);
        self.paint_control_focus(&add_response);
        answer.add = can_add && add_response.clicked();

        self.drawn(area.bottom());
        self.space(2.0 * STACK_ROW_AIR);
        answer
    }

    /// One file's row in `rect`; returns whether its × was clicked
    fn file_row(&mut self, rect: Rect, row: &FileRow<'_>, remove_label: &str) -> bool {
        let tokens = self.tokens;
        let words_right = rect.right() - REMOVE_INSET - REMOVE_BOX / 2.0;
        let name_color = if row.loading { tokens.text_dim } else { tokens.text };
        let name_font = if row.loading { FontId::proportional(TEXT_SIZE) } else { semibold(TEXT_SIZE) };
        let name_y = if row.folder.is_some() { rect.top() + NAME_CENTER } else { rect.center().y };
        let painter = self.ui.painter().clone();
        painter.text(pos2(rect.left() + CUBE_CENTER, rect.center().y), Align2::CENTER_CENTER, ic::CUBE, FontId::proportional(CUBE_SIZE), tokens.text_dim);
        let mut job = egui::text::LayoutJob::simple_singleline(row.name.to_owned(), name_font, name_color);
        job.wrap = egui::text::TextWrapping::truncate_at_width((words_right - rect.left() - NAME_LEFT).max(0.0));
        let name = painter.layout_job(job);
        painter.galley(pos2(rect.left() + NAME_LEFT, name_y - name.size().y / 2.0), name, name_color);
        if let Some(folder) = row.folder {
            let shown = folder_galley(&painter, folder, words_right - rect.left() - NAME_LEFT, tokens.text_dim);
            painter.galley(pos2(rect.left() + NAME_LEFT, rect.top() + FOLDER_CENTER - shown.size().y / 2.0), shown, tokens.text_dim);
        }

        let words = Rect::from_min_max(pos2(rect.left(), rect.top()), pos2(words_right, rect.bottom()));
        let words_id = self.next_id();
        let words_response = self.ui.interact(words, words_id, Sense::hover());
        let enabled = self.ui.is_enabled() && !row.loading;
        words_response.widget_info(|| WidgetInfo::labeled(egui::Role::ListItem, enabled, row.name));
        let full = match row.folder {
            Some(folder) => format!("{}\n{folder}", row.name),
            None => row.name.to_owned(),
        };
        self.tip(words_response, &full);
        if row.loading {
            return false;
        }

        let remove = Rect::from_center_size(pos2(rect.right() - REMOVE_INSET, rect.center().y), vec2(REMOVE_BOX, REMOVE_BOX));
        let id = self.next_id();
        let response = self.ui.interact(remove, id, Sense::click());
        response.widget_info(|| WidgetInfo::labeled(egui::Role::Button, self.ui.is_enabled(), remove_label));
        let response = self.tip(response, remove_label);
        let color = if response.hovered() { tokens.text_strong } else { tokens.text_dim };
        painter.text(remove.center(), Align2::CENTER_CENTER, ic::X, FontId::proportional(REMOVE_SIZE), color);
        self.paint_control_focus(&response);
        response.clicked()
    }
}

/// `folder` shortened to fit `room`, cut with "…" when even its first and last folders are too wide
fn folder_galley(painter: &egui::Painter, folder: &str, room: f32, color: egui::Color32) -> std::sync::Arc<egui::Galley> {
    crate::widgets::elided(painter, shortened_folder(painter, folder, room), SMALL_TEXT_SIZE, color, room)
}

/// `folder` with folders after its first one left out, from the second on, until it fits in `room`
fn shortened_folder(painter: &egui::Painter, folder: &str, room: f32) -> String {
    let fits = |text: &str| painter.layout_no_wrap(text.to_owned(), FontId::proportional(SMALL_TEXT_SIZE), egui::Color32::WHITE).size().x <= room;
    let mut parts: Vec<&str> = folder.split(['/', '\\']).collect();
    let mut shown = parts.join("/");
    while !fits(&shown) && parts.len() > 2 {
        if parts[1] == ELLIPSIS {
            parts.remove(2);
        } else {
            parts[1] = ELLIPSIS;
        }
        shown = parts.join("/");
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_too_long_even_when_shortened_is_cut_to_its_room() {
        let room = 120.0;
        let folder = format!("C:\\{}", "W".repeat(80));
        let mut laid_out = None;
        let mut output = egui::Context::default().run_ui(egui::RawInput::default(), |ui| {
            laid_out = Some(folder_galley(ui.painter(), &folder, room, egui::Color32::WHITE));
        });

        // The font atlas upload is not under test
        output.textures_delta.clear();
        let galley = laid_out.unwrap();

        assert!(galley.size().x <= room);
        assert_eq!(galley.rows[0].glyphs.last().map(|glyph| glyph.chr), Some('…'));
    }
}
