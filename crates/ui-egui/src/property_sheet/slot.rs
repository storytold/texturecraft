use egui::{Align2, CornerRadius, FontId, Painter, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use egui_phosphor::regular as ic;

use super::switch_control::PILL_SIZE;
use super::{ButtonIcon, CONTROL_RADIUS, PropertySheet, ROW_HEIGHT, SlotResponse, TEXT_SIZE};
use crate::theme::semibold;

/// Air between a slot card's top and its slot row
const SLOT_CARD_TOP: f32 = 6.0;

/// Height of a slot row
const SLOT_HEIGHT: f32 = 36.0;

/// Size of a slot's thumbnail
pub(super) const THUMB_SIZE: f32 = 26.0;

/// Air between the thumbnail and the slot's name
const THUMB_TO_NAME: f32 = 10.0;

/// Size of a slot's source line
const SOURCE_TEXT_SIZE: f32 = 10.0;

/// The slot's name line above its centre and its source line below it
const NAME_ABOVE_CENTER: f32 = 7.0;
const SOURCE_BELOW_CENTER: f32 = 8.0;

/// The square a slot's end glyph sits in
const SLOT_GLYPH_BOX: f32 = 18.0;

/// The extra margin the on/off pill answers clicks in
const TOGGLE_HIT_MARGIN: f32 = 3.0;

/// The seed's dice glyph size
const DICE_ICON_SIZE: f32 = 14.0;

/// From the right edge to the slot toggle's centre
const SLOT_TOGGLE_INSET: f32 = 11.0;

/// From the right edge to the × with a toggle beside it, and without
const CLEAR_INSET_WITH_TOGGLE: f32 = 40.0;
const CLEAR_INSET: f32 = 6.0;

/// Distance between the × and the ⟲ left of it
const RESET_STEP: f32 = 22.0;

/// Size of the slot's × and ⟲ glyphs
const SLOT_ICON_SIZE: f32 = 12.0;

/// From the seed field's right edge to its value's right end
const SEED_VALUE_INSET: f32 = 8.0;

/// Room the seed's dice button takes at the row's end, and the button's width
const DICE_ROOM: f32 = 34.0;
const DICE_WIDTH: f32 = 28.0;

/// Air between a full-width button's icon and its words
const BUTTON_ICON_GAP: f32 = 8.0;

/// Size of a full-width button's icon
const BUTTON_ICON_SIZE: f32 = 14.0;

impl PropertySheet<'_> {
    /// Opens a slot card with its top row: thumbnail, name over source, then ⟲, × and the toggle on the right; the row opens the source list
    pub(crate) fn slot(
        &mut self,
        thumbnail: &dyn Fn(&Painter, Rect),
        name: &str,
        source: &str,
        on: Option<&mut bool>,
        clear: bool,
        reset: bool,
    ) -> SlotResponse {
        self.open_card();
        self.scope = self.named_id("slot", name);
        self.space(SLOT_CARD_TOP);
        self.slot_row(thumbnail, name, source, on, clear, reset.then_some(true))
    }

    /// [`Self::slot`] whose ⟲ always shows beside the ×, faint and inert while what it restores is at its defaults (`changed` false)
    pub(crate) fn slot_restorable(&mut self, thumbnail: &dyn Fn(&Painter, Rect), name: &str, source: &str, changed: bool) -> SlotResponse {
        self.open_card();
        self.space(SLOT_CARD_TOP);
        self.slot_row(thumbnail, name, source, None, true, Some(changed))
    }

    /// [`Self::slot`]'s row inside the card open now (an input a mode adds, under that mode), not a card of its own
    pub(crate) fn slot_in_card(&mut self, thumbnail: &dyn Fn(&Painter, Rect), name: &str, source: &str, clear: bool) -> SlotResponse {
        self.slot_row(thumbnail, name, source, None, clear, None)
    }

    /// A slot row: thumbnail, name over source, then ⟲ (`Some(changed)`), × and the toggle on the right; the row opens the source list
    fn slot_row(
        &mut self,
        thumbnail: &dyn Fn(&Painter, Rect),
        name: &str,
        source: &str,
        on: Option<&mut bool>,
        clear: bool,
        reset: Option<bool>,
    ) -> SlotResponse {
        let rect = self.take(SLOT_HEIGHT);
        let center_y = rect.top() + SLOT_HEIGHT / 2.0;
        let enabled = on.as_deref().copied().unwrap_or(true);
        let has_toggle = on.is_some();
        let thumb = Rect::from_min_size(pos2(self.left, center_y - THUMB_SIZE / 2.0), vec2(THUMB_SIZE, THUMB_SIZE));
        let clear_x = self.right - if has_toggle { CLEAR_INSET_WITH_TOGGLE } else { CLEAR_INSET };
        let reset_x = if clear { clear_x - RESET_STEP } else { clear_x };

        // The picker spans the card while the trailing glyphs keep their own hit areas
        let row = self.bled_row(rect);
        let pick_id = self.next_id();
        let pick = self.ui.interact(row, pick_id, Sense::click());
        pick.widget_info(|| egui::WidgetInfo { current_text_value: Some(source.to_owned()), ..egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name) });
        let mut result = SlotResponse { pick, clear: false, reset: false, toggled: false };

        let tokens = self.tokens;
        let painter = self.ui.painter().clone();
        thumbnail(&painter, thumb);
        painter.rect_stroke(
            thumb,
            CornerRadius::ZERO,
            Stroke::new(1.0, if result.pick.hovered() { tokens.accent } else { tokens.field_border }),
            StrokeKind::Inside,
        );
        let text_x = thumb.right() + THUMB_TO_NAME;
        let (name_font, name_color) = if enabled { (semibold(TEXT_SIZE), tokens.text) } else { (FontId::proportional(TEXT_SIZE), tokens.text_dim) };
        painter.text(pos2(text_x, center_y - NAME_ABOVE_CENTER), Align2::LEFT_CENTER, name, name_font, name_color);
        painter.text(pos2(text_x, center_y + SOURCE_BELOW_CENTER), Align2::LEFT_CENTER, source, FontId::proportional(SOURCE_TEXT_SIZE), tokens.text_dim);

        for (shown, live, x, glyph, tip, slot) in [
            (clear, true, clear_x, ic::X, crate::i18n::n_("Clear"), &mut result.clear),
            (reset.is_some(), reset.unwrap_or(false), reset_x, ic::CLOCK_COUNTER_CLOCKWISE, crate::i18n::n_("Restore defaults"), &mut result.reset),
        ] {
            if !shown {
                continue;
            }

            let glyph_rect = Rect::from_center_size(pos2(x, center_y), Vec2::splat(SLOT_GLYPH_BOX));
            let id = self.next_id();

            // At the defaults the ⟲ is faint and inert, as on a band: a click there picks the source like the rest of the row
            let response = if live {
                self.tip(self.ui.interact(glyph_rect, id, Sense::click()), crate::i18n::tr(tip))
            } else {
                self.ui.interact(glyph_rect, id, Sense::hover())
            };
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, live, crate::i18n::tr(tip)));
            *slot = live && response.clicked();
            let color = self.reset_glyph_color(live, response.hovered(), tokens.text);
            painter.text(glyph_rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(SLOT_ICON_SIZE), color);
        }

        if let Some(on) = on {
            let pill = Rect::from_center_size(pos2(self.right - SLOT_TOGGLE_INSET, center_y), PILL_SIZE);
            let id = self.next_id();
            let response = self.ui.interact(pill.expand(TOGGLE_HIT_MARGIN), id, Sense::click());
            if response.clicked() {
                *on = !*on;
                result.toggled = true;
            }
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, *on, name));
            self.pill(pill.center(), *on, true);
        }
        self.drawn(thumb.bottom());
        result
    }

    /// A full-width button: centred icon and words
    pub(crate) fn button<'a>(&mut self, icon: impl Into<ButtonIcon<'a>>, text: &str) -> egui::Response {
        self.button_lit(icon, text, false)
    }

    /// A full-width button lit in the accent while what it turns on stays on
    pub(crate) fn button_lit<'a>(&mut self, icon: impl Into<ButtonIcon<'a>>, text: &str, lit: bool) -> egui::Response {
        self.button_enabled(icon, text, lit, true)
    }

    /// [`Self::button_lit`] that answers no clicks and dims its words while not `enabled`
    pub(crate) fn button_enabled<'a>(&mut self, icon: impl Into<ButtonIcon<'a>>, text: &str, lit: bool, enabled: bool) -> egui::Response {
        self.button_sensing(icon.into(), text, lit, if enabled { Sense::click() } else { Sense::hover() })
    }

    fn button_sensing(&mut self, icon: ButtonIcon<'_>, text: &str, lit: bool, sense: Sense) -> egui::Response {
        let enabled = sense != Sense::hover();
        let rect = self.take(ROW_HEIGHT);
        let button = self.control_box(self.left, rect.center().y);
        let id = self.next_id();
        let response = self.ui.interact(button, id, sense);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, text));

        let tokens = self.tokens;
        let painter = self.ui.painter();
        painter.rect(
            button,
            CornerRadius::same(CONTROL_RADIUS),
            match (lit, enabled && response.hovered()) {
                (true, _) => tokens.accent_dim,
                (false, true) => tokens.row_hover,
                (false, false) => tokens.button,
            },
            Stroke::new(1.0, if lit { tokens.accent } else { tokens.button_border }),
            StrokeKind::Inside,
        );
        let color = if enabled { tokens.text } else { tokens.text_dim };
        let font = FontId::proportional(TEXT_SIZE);
        let (icon_width, icon_galley) = match icon {
            ButtonIcon::Glyph(glyph) => {
                let galley = painter.layout_no_wrap(glyph.to_owned(), FontId::proportional(BUTTON_ICON_SIZE), color);
                (galley.size().x, Some(galley))
            }
            ButtonIcon::Drawn(_) => (BUTTON_ICON_SIZE, None),
        };
        let text_galley = painter.layout_no_wrap(text.to_owned(), font, color);
        let total = icon_width + BUTTON_ICON_GAP + text_galley.size().x;
        let x = button.center().x - total / 2.0;
        if let Some(galley) = icon_galley {
            painter.galley(pos2(x, button.center().y - galley.size().y / 2.0), galley, color);
        }
        if let ButtonIcon::Drawn(drawn) = icon {
            drawn.paint(painter, pos2(x + icon_width / 2.0, button.center().y), BUTTON_ICON_SIZE, color);
        }
        painter.galley(pos2(x + icon_width + BUTTON_ICON_GAP, button.center().y - text_galley.size().y / 2.0), text_galley, color);
        self.paint_control_focus(&response);
        self.drawn(button.bottom());
        response
    }

    /// A seed: its field (click to type) and the dice button, which picks a new one. Returns true when the seed changed
    pub(crate) fn seed(&mut self, label: &str, seed: &mut u32) -> bool {
        let (control, center_y) = self.row(label);
        let field = Rect::from_min_max(control.min, pos2(self.right - DICE_ROOM, control.bottom()));
        let dice = Rect::from_min_max(pos2(self.right - DICE_WIDTH, control.top()), control.max);

        let id = self.next_id();
        let typing_id = id.with((self.id, "typed"));
        let tokens = self.tokens;
        let painter = self.ui.painter().clone();
        painter.rect(field, CornerRadius::same(CONTROL_RADIUS), tokens.field, Stroke::new(1.0, tokens.field_border), StrokeKind::Inside);

        // The value sits right like every number; the box lights on hover and goes accent while typing
        let mut changed = false;
        if let Some(mut draft) = self.typing_draft(typing_id) {
            let response = self.typing_edit(field.left() + SEED_VALUE_INSET, field.right() - SEED_VALUE_INSET, center_y, typing_id, &mut draft);
            painter.rect_stroke(field, CornerRadius::same(CONTROL_RADIUS), Stroke::new(1.0, tokens.accent), StrokeKind::Inside);

            if response.lost_focus() {
                let untouched = self.end_typing(typing_id, &draft);
                if let Ok(typed) = draft.text.trim().parse::<u32>()
                    && !untouched
                    && typed != *seed
                    && !self.escape_pressed()
                {
                    *seed = typed;
                    changed = true;
                }
            } else {
                self.keep_typing(typing_id, draft);
            }
        } else {
            self.skip_typing_edit();
            let response = self.ui.interact(field, id, Sense::click()).on_hover_cursor(egui::CursorIcon::Text);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, crate::i18n::trf("{label} value", &[("label", &label)])));
            painter.text(pos2(field.right() - SEED_VALUE_INSET, center_y), Align2::RIGHT_CENTER, seed.to_string(), semibold(TEXT_SIZE), tokens.text);

            if response.hovered() {
                painter.rect_stroke(field, CornerRadius::same(CONTROL_RADIUS), Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
            }
            if response.clicked() {
                self.start_typing(typing_id, seed.to_string());
            }
        }

        // The dice picks the next seed
        let dice_id = self.next_id();
        let dice_response = self.tip(self.ui.interact(dice, dice_id, Sense::click()), crate::i18n::tr("Random"));
        painter.rect(
            dice,
            CornerRadius::same(CONTROL_RADIUS),
            if dice_response.hovered() { tokens.row_hover } else { tokens.button },
            Stroke::new(1.0, tokens.button_border),
            StrokeKind::Inside,
        );
        painter.text(dice.center(), Align2::CENTER_CENTER, ic::DICE_FIVE, FontId::proportional(DICE_ICON_SIZE), tokens.text);
        dice_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, crate::i18n::tr("Random")));

        if dice_response.clicked() {
            *seed = texturecraft_engine::folder_cmds::next_seed(*seed);
            changed = true;
        }
        changed
    }
}
