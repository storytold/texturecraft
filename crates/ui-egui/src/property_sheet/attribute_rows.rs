//! Read-only rows (an asset's Attributes): a label and its value as dim text, nothing to edit

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{FontId, Galley, Sense, pos2};
use std::sync::Arc;

use super::{PropertySheet, TEXT_SIZE};

/// Height of the label line above a wrapped text
const TEXT_LABEL_LINE: f32 = 18.0;

/// From the label line's top to the label's centre
const TEXT_LABEL_CENTER: f32 = 8.0;

/// Air above a wrapped text's label
const TEXT_LABEL_AIR: f32 = 4.0;

/// Height of a wrapped text's lines
const TEXT_LINE_HEIGHT: f32 = 18.0;

/// Air under a wrapped text's last line
const TEXT_AIR_BELOW: f32 = 6.0;

impl PropertySheet<'_> {
    /// `label` beside its value as dim text, cut with "…" where it would pass the right edge (the whole value then shows on hover)
    pub(crate) fn attribute(&mut self, label: &str, value: &str) {
        let (rect, center_y) = self.row(label);
        let galley = self.dim_text(value, rect.width(), Some(1));
        let cut = galley.elided;
        self.ui.painter().galley(pos2(rect.left(), center_y - galley.size().y / 2.0), galley, self.tokens.text_dim);
        let response = self.ui.interact(rect, self.id.with(("attribute", label)), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{label}: {value}")));
        if cut {
            self.tip(response, value);
        }
    }

    /// `label` on its own line, then `text` wrapped across the panel in dim lines
    pub(crate) fn attribute_text(&mut self, label: &str, text: &str) {
        self.space(TEXT_LABEL_AIR);
        let line = self.take(TEXT_LABEL_LINE);
        let color = self.tokens.text;
        self.ui.painter().text(pos2(self.left, line.top() + TEXT_LABEL_CENTER), egui::Align2::LEFT_CENTER, label, FontId::proportional(TEXT_SIZE), color);
        let galley = self.dim_text(text, self.right - self.left, None);
        let body = self.take(galley.size().y.max(TEXT_LINE_HEIGHT) + TEXT_AIR_BELOW);
        let response = self.ui.interact(body, self.id.with(("attribute", label)), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{label}: {text}")));
        self.ui.painter().galley(pos2(self.left, body.top()), galley, self.tokens.text_dim);
        self.drawn(body.bottom() - TEXT_AIR_BELOW);
    }

    /// `text` laid out dim in `width`, wrapped on lines [`TEXT_LINE_HEIGHT`] apart, at most `rows` of them (the last cut with "…")
    fn dim_text(&self, text: &str, width: f32, rows: Option<usize>) -> Arc<Galley> {
        let format =
            TextFormat { font_id: FontId::proportional(TEXT_SIZE), color: self.tokens.text_dim, line_height: Some(TEXT_LINE_HEIGHT), ..Default::default() };
        let mut job = LayoutJob::single_section(text.to_owned(), format);
        job.wrap = TextWrapping { max_width: width, max_rows: rows.unwrap_or(usize::MAX), break_anywhere: rows.is_some(), overflow_character: Some('…') };
        self.ui.painter().layout_job(job)
    }
}
