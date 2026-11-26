use egui::{Color32, FontId, Id, Rect, Ui, pos2, vec2};

use super::{
    CONTROL_HEIGHT, LABEL_COLUMN_MIN, LABEL_TO_CONTROL, SIDE_PADDING_POINTS, TEXT_SIZE, TRACK_TO_VALUE, VALUE_MIN_WIDTH, open_card::OpenCard, section_tabs,
    sheet_measure::SheetMeasure,
};
use crate::theme::{Tokens, semibold};

type SheetMeasures = egui::cache::FramePublisher<Id, SheetMeasure>;

/// The Properties panel being laid out this frame: where the controls column is, which card is open, and what this frame measured for the next one
pub(crate) struct PropertySheet<'u> {
    pub(crate) ui: &'u mut Ui,
    pub(super) tokens: Tokens,
    pub(super) id: Id,
    pub(super) scope: Id,
    pub(super) band_scope: Id,
    pub(super) named: Vec<Id>,
    pub(super) left: f32,
    pub(super) right: f32,
    pub(super) used: SheetMeasure,
    pub(super) seen: SheetMeasure,
    pub(super) card: Option<OpenCard>,
    pub(super) air_at: Option<f32>,
    pub(super) drawn_bottom: f32,
    pub(super) empty: bool,
    pub(super) sections: Vec<(String, f32)>,
    pub(super) info: Option<&'u str>,
    pub(super) held: bool,
    /// Sub-headers so far in the current folding section: a later one takes air above it
    pub(super) sub_headers: usize,
}

impl<'u> PropertySheet<'u> {
    /// Lays out `body` as one panel; when its measured columns change, egui reruns the frame unseen
    pub(crate) fn show<R>(ui: &'u mut Ui, id_salt: impl egui::AsIdSalt, body: impl FnOnce(&mut PropertySheet<'_>) -> R) -> R {
        let id = ui.make_persistent_id(id_salt);
        let used = ui.memory_mut(|memory| memory.caches.cache::<SheetMeasures>().get(&id).copied()).unwrap_or_default();
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let rect = ui.available_rect_before_wrap();
        let mut sheet = PropertySheet {
            tokens: Tokens::get(ui.ctx()),
            id,
            scope: id,
            band_scope: id,
            named: Vec::new(),
            left: rect.left() + SIDE_PADDING_POINTS,
            right: rect.right() - SIDE_PADDING_POINTS,
            used,
            seen: SheetMeasure::default(),
            card: None,
            air_at: None,
            drawn_bottom: rect.top(),
            empty: true,
            sections: Vec::new(),
            info: None,
            held: false,
            sub_headers: 0,
            ui,
        };
        let result = body(&mut sheet);
        sheet.close_card();
        if !sheet.sections.is_empty() {
            let view_top = sheet.ui.clip_rect().top();
            section_tabs::remember(sheet.ui, section_tabs::SectionTops { tops: std::mem::take(&mut sheet.sections), view_top });
        }

        // Refresh visible sheets so measurements of closed sheets expire
        let seen = sheet.seen;
        sheet.ui.memory_mut(|memory| memory.caches.cache::<SheetMeasures>().set(id, seen));
        if seen != used {
            sheet.ui.ctx().request_discard("Properties columns changed");
        }

        result
    }

    /// The controls column: wide enough for this panel's labels, capped at [`super::LABEL_MAX`]
    pub(super) fn column(&self) -> f32 {
        self.left + LABEL_COLUMN_MIN.max(self.used.label_width + LABEL_TO_CONTROL)
    }

    /// Where the knob of a slider's track ends: left of this panel's widest value
    pub(super) fn track_end(&self) -> f32 {
        self.right - self.used.value_width.max(VALUE_MIN_WIDTH) - TRACK_TO_VALUE
    }

    /// Records a value's width for the panel's value column
    pub(super) fn measure_value(&mut self, text: &str) {
        let width = self.text_width(text, semibold(TEXT_SIZE));
        self.seen.value_width = self.seen.value_width.max(width);
    }

    pub(super) fn text_width(&self, text: &str, font: FontId) -> f32 {
        self.ui.painter().layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x
    }

    /// Takes the next `height` pixels of the panel, full width; marks what it draws down to
    pub(super) fn take(&mut self, height: f32) -> Rect {
        let (rect, _) = self.ui.allocate_exact_size(vec2(self.ui.available_width(), height), egui::Sense::hover());
        self.empty = false;
        rect
    }

    /// Leaves `height` pixels of air
    pub(super) fn space(&mut self, height: f32) {
        self.ui.add_space(height);
    }

    pub(super) fn cursor_y(&self) -> f32 {
        self.ui.cursor().top()
    }

    /// Notes that something was painted down to `bottom`, so an open card ends its padding below it
    pub(super) fn drawn(&mut self, bottom: f32) {
        self.drawn_bottom = self.drawn_bottom.max(bottom);
    }

    /// A control's box: [`CONTROL_HEIGHT`] tall, centred in its row, from `x0` to the right edge
    pub(super) fn control_box(&self, x0: f32, center_y: f32) -> Rect {
        Rect::from_min_max(pos2(x0, center_y - CONTROL_HEIGHT / 2.0), pos2(self.right, center_y + CONTROL_HEIGHT / 2.0))
    }

    /// The colour of a top-level band: between the window frame and a card
    pub(super) fn band_fill(&self) -> Color32 {
        let tokens = &self.tokens;
        let mid = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
        Color32::from_rgb(mid(tokens.chrome.r(), tokens.row.r()), mid(tokens.chrome.g(), tokens.row.g()), mid(tokens.chrome.b(), tokens.row.b()))
    }
}
