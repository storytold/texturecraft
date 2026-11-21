//! The Properties panel's rows, groups and controls, shared by every panel so they line up

mod action_row;
mod action_row_response;
mod attribute_rows;
mod axis_rows;
mod band_response;
mod button_icon;
mod choice;
mod color_bar;
mod color_picker_rows;
mod compare_row;
mod compare_row_response;
mod eye_toggle;
mod factor_slider;
mod gradient_rows;
mod group;
mod hold_pad;
mod id_color_row;
mod id_color_row_response;
mod info_icon;
mod marker_drag;
mod material_rows;
mod name_field;
mod number_field;
mod open_card;
mod preview_block;
mod preview_edit;
mod preview_lower;
mod preview_size;
mod ramp_edit;
mod reference_row;
mod row;
mod row_lead;
mod section_tabs;
mod sheet_measure;
mod slider;
mod slot;
mod slot_response;
mod source_card;
mod stack_slider;
mod switch_control;
mod tooltip;
mod track;
mod value_text;
mod value_text_draft;

#[cfg(test)]
mod tests;

pub(crate) use action_row_response::ActionRowResponse;
pub(crate) use band_response::BandResponse;
pub(crate) use button_icon::ButtonIcon;
pub(crate) use compare_row_response::CompareRowResponse;
pub(crate) use id_color_row_response::IdColorRowResponse;
pub(crate) use number_field::NumberField;
pub(crate) use preview_lower::PreviewLower;
pub(crate) use preview_size::PreviewSize;
pub(crate) use row_lead::RowLead;
pub(crate) use section_tabs::{
    SectionScope, TAB_GAP, TAB_ICON_CENTER, TAB_ICON_SIZE, TAB_NAME_LEFT, TAB_PADDING, TAB_ROUNDING, TAB_TEXT_SIZE, TAB_TOP, forget_sections,
    full_height_tab_rect, in_scope, room_past_end, section_tabs, shown_tab_box,
};
pub(crate) use slot_response::SlotResponse;
pub(crate) use stack_slider::StackSlider;

use egui::{Color32, FontId, Id, Rect, Ui, pos2, vec2};

use crate::theme::{Tokens, semibold};
use open_card::OpenCard;
use sheet_measure::SheetMeasure;

type SheetMeasures = egui::cache::FramePublisher<Id, SheetMeasure>;

/// Side padding: every row starts and ends this far inside the panel
pub(crate) const PAD: f32 = 12.0;

/// Row pitch: every control row is one box this tall with its control centred
pub(crate) const ROW_HEIGHT: f32 = 28.0;

/// Height of dropdowns, fields, switches and buttons
const CONTROL_HEIGHT: f32 = 24.0;

/// Air between groups and after loose rows
const GROUP_GAP: f32 = 6.0;

/// The narrowest label column, from the panel's left padding to the controls
const LABEL_COLUMN_MIN: f32 = 90.0;

/// Labels wider than this stack above a full-width control and never widen the column
const LABEL_MAX: f32 = 132.0;

/// Air between the widest label and the controls column
const LABEL_TO_CONTROL: f32 = 12.0;

/// A label fits beside its control when it leaves at least this much air before the column
const LABEL_MIN_AIR: f32 = 8.0;

/// The least air a loose label leaves before the controls column
const LOOSE_LABEL_AIR: f32 = 2.0;

/// The narrowest value column at a slider's right end
const VALUE_MIN_WIDTH: f32 = 26.0;

/// Air between a slider track's end and the value column, room for the knob and a gap
const TRACK_TO_VALUE: f32 = 14.0;

/// Size of regular labels and SemiBold values
const TEXT_SIZE: f32 = 12.0;

/// Size of captions, pill names and the switch's words
const SMALL_TEXT_SIZE: f32 = 11.0;

/// Size of the caret and small glyphs drawn as icons
const ICON_SIZE: f32 = 12.0;

/// Size of a picture row's picture, and the air after it
const ROW_PICTURE_SIZE: f32 = 18.0;
const ROW_PICTURE_AIR: f32 = 8.0;

/// The click square of a glyph button inside a row
const GLYPH_TARGET: f32 = 18.0;

/// How far above and below a row's centre a slider track or a typed value takes the pointer
const ROW_HIT_HALF_HEIGHT: f32 = 10.0;

/// Size of a row's glyph button glyphs (eyedropper, remove, arrow)
const GLYPH_SIZE: f32 = 13.0;

/// Corner radius of a hovered row's face
const ROW_RADIUS: u8 = 6;

/// Corner radius of dropdowns, fields and buttons
const CONTROL_RADIUS: u8 = 4;

/// Air above and below a picture block, as a control has in its row
const BLOCK_MARGIN: f32 = 2.0;

/// The Properties panel being laid out this frame: where the controls column is, which card is open, and what this frame measured for the next one
pub(crate) struct PropertySheet<'u> {
    pub(crate) ui: &'u mut Ui,
    tokens: Tokens,
    id: Id,
    scope: Id,
    band_scope: Id,
    named: Vec<Id>,
    left: f32,
    right: f32,
    used: SheetMeasure,
    seen: SheetMeasure,
    card: Option<OpenCard>,
    air_at: Option<f32>,
    drawn_bottom: f32,
    empty: bool,
    sections: Vec<(String, f32)>,
    info: Option<&'u str>,
    held: bool,
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
            left: rect.left() + PAD,
            right: rect.right() - PAD,
            used,
            seen: SheetMeasure::default(),
            card: None,
            air_at: None,
            drawn_bottom: rect.top(),
            empty: true,
            sections: Vec::new(),
            info: None,
            held: false,
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

    /// The controls column: wide enough for this panel's labels, capped at [`LABEL_MAX`]
    fn column(&self) -> f32 {
        self.left + LABEL_COLUMN_MIN.max(self.used.label_width + LABEL_TO_CONTROL)
    }

    /// Where the knob of a slider's track ends: left of this panel's widest value
    fn track_end(&self) -> f32 {
        self.right - self.used.value_width.max(VALUE_MIN_WIDTH) - TRACK_TO_VALUE
    }

    /// Records a value's width for the panel's value column
    fn measure_value(&mut self, text: &str) {
        let width = self.text_width(text, semibold(TEXT_SIZE));
        self.seen.value_width = self.seen.value_width.max(width);
    }

    fn text_width(&self, text: &str, font: FontId) -> f32 {
        self.ui.painter().layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x
    }

    /// Takes the next `height` pixels of the panel, full width; marks what it draws down to
    fn take(&mut self, height: f32) -> Rect {
        let (rect, _) = self.ui.allocate_exact_size(vec2(self.ui.available_width(), height), egui::Sense::hover());
        self.empty = false;
        rect
    }

    /// Leaves `height` pixels of air
    fn space(&mut self, height: f32) {
        self.ui.add_space(height);
    }

    fn cursor_y(&self) -> f32 {
        self.ui.cursor().top()
    }

    /// Notes that something was painted down to `bottom`, so an open card ends its padding below it
    fn drawn(&mut self, bottom: f32) {
        self.drawn_bottom = self.drawn_bottom.max(bottom);
    }

    /// A control's box: [`CONTROL_HEIGHT`] tall, centred in its row, from `x0` to the right edge
    fn control_box(&self, x0: f32, center_y: f32) -> Rect {
        Rect::from_min_max(pos2(x0, center_y - CONTROL_HEIGHT / 2.0), pos2(self.right, center_y + CONTROL_HEIGHT / 2.0))
    }

    /// The colour of a top-level band: between the window frame and a card
    fn band_fill(&self) -> Color32 {
        let tokens = &self.tokens;
        let mid = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
        Color32::from_rgb(mid(tokens.chrome.r(), tokens.row.r()), mid(tokens.chrome.g(), tokens.row.g()), mid(tokens.chrome.b(), tokens.row.b()))
    }
}
