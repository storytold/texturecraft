//! A card for the layer something draws: its thumbnail, its name over a link and its texture set, and an arrow; a click goes to it

use egui::{Align2, CornerRadius, FontId, Painter, Rect, Sense, pos2, vec2};
use egui_phosphor::regular as ic;

use super::group::CARD_RADIUS;
use super::slot::THUMB_SIZE;
use super::{GROUP_GAP, PropertySheet, SMALL_TEXT_SIZE, TEXT_SIZE};
use crate::theme::semibold;

/// Height of the card: 11 px of air above and below the thumbnail, as a slot card has
const CARD_HEIGHT: f32 = 48.0;

/// Air between the section caption and the card, as above a slot card
const CARD_TOP_AIR: f32 = 6.0;

/// From the left edge to the name, to the link glyph's centre and to the texture set
const NAME_LEFT: f32 = 36.0;
const LINK_CENTER: f32 = 41.0;
const SET_LEFT: f32 = 51.0;

/// From the card's centre line up to the name's and down to the texture set's
const NAME_RISE: f32 = 7.0;
const SET_DROP: f32 = 9.0;

/// Size of the link glyph and of the arrow
const LINK_SIZE: f32 = 12.0;
const ARROW_SIZE: f32 = 14.0;

/// From the right edge to the arrow's centre
const ARROW_INSET: f32 = 7.0;

impl PropertySheet<'_> {
    /// The card of `name` in texture set `set`, right under its section caption; a `missing` one is dim, shows a broken link and takes no click. Returns true when it was clicked
    pub(crate) fn source_card(&mut self, thumbnail: &dyn Fn(&Painter, Rect), name: &str, set: &str, missing: bool) -> bool {
        self.close_card();
        self.space(CARD_TOP_AIR);
        let rect = self.take(CARD_HEIGHT);
        let center_y = rect.center().y;
        let face = self.bled_row(rect);
        let id = self.next_id();
        let mut response = self.ui.interact(face, id, if missing { Sense::hover() } else { Sense::click() });
        if !missing {
            response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, !missing, format!("{name} {set}")));
        let hovered = !missing && response.hovered();

        let tokens = self.tokens;
        let painter = self.ui.painter().clone();
        painter.rect_filled(face, CornerRadius::same(CARD_RADIUS), if hovered { tokens.button } else { tokens.row });
        thumbnail(&painter, Rect::from_min_size(pos2(self.left, center_y - THUMB_SIZE / 2.0), vec2(THUMB_SIZE, THUMB_SIZE)));
        let (name_font, name_color) = if missing { (FontId::proportional(TEXT_SIZE), tokens.text_dim) } else { (semibold(TEXT_SIZE), tokens.text) };
        painter.text(pos2(self.left + NAME_LEFT, center_y - NAME_RISE), Align2::LEFT_CENTER, name, name_font, name_color);
        let link = if missing { ic::LINK_BREAK } else { ic::LINK };
        painter.text(pos2(self.left + LINK_CENTER, center_y + SET_DROP), Align2::CENTER_CENTER, link, FontId::proportional(LINK_SIZE), tokens.text_dim);
        painter.text(pos2(self.left + SET_LEFT, center_y + SET_DROP), Align2::LEFT_CENTER, set, FontId::proportional(SMALL_TEXT_SIZE), tokens.text_dim);
        if !missing {
            let arrow_color = if hovered { tokens.text } else { tokens.text_dim };
            painter.text(pos2(self.right - ARROW_INSET, center_y), Align2::CENTER_CENTER, ic::ARROW_RIGHT, FontId::proportional(ARROW_SIZE), arrow_color);
        }
        self.drawn(rect.bottom());
        self.space(GROUP_GAP);
        self.air_at = Some(self.cursor_y());
        response.clicked()
    }
}
