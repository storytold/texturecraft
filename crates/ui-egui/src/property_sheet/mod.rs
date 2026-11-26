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
mod file_list;
mod file_list_add;
mod file_list_response;
mod file_row;
mod fold_section;
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
mod properties_scroll;
mod ramp_edit;
mod reference_row;
mod row;
mod row_lead;
mod section_tabs;
#[path = "property_sheet.rs"]
mod sheet;
mod sheet_measure;
mod size_columns;
mod size_columns_response;
mod slider;
mod slot;
mod slot_response;
mod source_card;
mod stack_slider;
mod stacked_rows;
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
pub(crate) use file_list_add::FileListAdd;
pub(crate) use file_list_response::FileListResponse;
pub(crate) use file_row::FileRow;
pub(crate) use fold_section::FoldSection;
pub(crate) use id_color_row_response::IdColorRowResponse;
pub(crate) use number_field::NumberField;
pub(crate) use preview_lower::PreviewLower;
pub(crate) use preview_size::PreviewSize;
pub(crate) use properties_scroll::{PropertiesScroll, PropertiesSelection};
pub(crate) use row_lead::RowLead;
pub(crate) use section_tabs::{
    SectionScope, TAB_GAP, TAB_ICON_CENTER, TAB_ICON_SIZE, TAB_NAME_LEFT, TAB_PADDING, TAB_ROUNDING, TAB_TEXT_SIZE, TAB_TOP, forget_sections,
    full_height_tab_rect, in_scope, room_past_end, section_tabs, shown_tab_box,
};
pub(crate) use sheet::PropertySheet;
pub(crate) use size_columns_response::SizeColumnsResponse;
pub(crate) use slot_response::SlotResponse;
pub(crate) use stack_slider::StackSlider;
pub(crate) use stacked_rows::{CHECK_SIZE, paint_check};

pub(crate) fn paint_keyboard_focus(ui: &egui::Ui, response: &egui::Response, tokens: &crate::theme::Tokens) {
    if response.has_focus() {
        switch_control::paint_focus_outline(ui.painter(), response.rect, tokens.text_strong);
    }
}

/// Side padding: every row starts and ends this far inside the panel
pub(crate) const SIDE_PADDING_POINTS: f32 = 12.0;

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
