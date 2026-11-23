//! The Material section's own rows: the orb beside the Material | Channels switch and a uniform channel's gray ramp

use std::ops::RangeInclusive;

use egui::{Color32, Mesh, Painter, Rect, Sense, Vec2, pos2, vec2};

use super::ramp_edit::RampEdit;
use super::slider::KNOB_RADIUS;
use super::track::Track;
use super::{CONTROL_HEIGHT, GLYPH_TARGET, PropertySheet, ROW_HEIGHT};
use crate::drawn_icon::DrawnIcon;

/// Side of the material's orb
const ORB_SIZE: f32 = 90.0;

/// Air between the orb and the switch beside it
const ORB_TO_SWITCH: f32 = 12.0;

/// Air under the orb
const ORB_AIR: f32 = 14.0;

/// From the right edge to the end of a ramp's track, where its value and invert glyph sit
const RAMP_END_INSET: f32 = 56.0;

/// From the right edge to a ramp's value: where the linked Tiling and Offset values end
const RAMP_VALUE_INSET: f32 = super::preview_block::LINK_ROOM;

/// From the right edge to a ramp's invert glyph's centre
const INVERT_INSET: f32 = 8.0;

/// Width of a ramp's typed value
const RAMP_VALUE_WIDTH: f32 = 34.0;

/// Height of a ramp's track
const RAMP_HEIGHT: f32 = 4.0;

/// Size of the invert glyph
const INVERT_GLYPH_SIZE: f32 = 14.0;

/// The gaps between the widgets of a card's own body
const CARD_ITEM_SPACING: Vec2 = vec2(8.0, 4.0);

impl PropertySheet<'_> {
    /// The material's orb with the Material | Channels switch beside it (none without `options`); returns the segment clicked when it was not the chosen one
    pub(crate) fn orb_switch(&mut self, orb: &dyn Fn(&Painter, Rect), options: &[&str], selected: usize) -> Option<usize> {
        let rect = self.take(ORB_SIZE);
        let square = Rect::from_min_size(pos2(self.left, rect.top()), vec2(ORB_SIZE, ORB_SIZE));
        orb(self.ui.painter(), square);
        let switch = Rect::from_min_max(
            pos2(square.right() + ORB_TO_SWITCH, square.center().y - CONTROL_HEIGHT / 2.0),
            pos2(self.right, square.center().y + CONTROL_HEIGHT / 2.0),
        );
        let picked = if options.is_empty() { None } else { self.segments(switch, options, selected, super::switch_control::untinted, None) };
        self.drawn(square.bottom());
        self.space(ORB_AIR);
        picked
    }

    /// A uniform gray channel: a black-to-white ramp across the card (fills from the middle when `range` is signed), its value and the invert glyph
    pub(crate) fn ramp(&mut self, label: &str, value: &mut f32, range: RangeInclusive<f32>) -> RampEdit {
        let row = self.take(ROW_HEIGHT);
        let center_y = row.center().y;
        let id = self.next_id();
        let track =
            Track { start: self.left + KNOB_RADIUS, end: self.right - RAMP_END_INSET, center_y, zero: (*range.start() < 0.0).then_some(0.0), ramp: true };
        let ramp = Rect::from_min_max(pos2(track.start, center_y - RAMP_HEIGHT / 2.0), pos2(track.end, center_y + RAMP_HEIGHT / 2.0));
        self.ui.painter().add(gray_ramp(ramp));
        let dragged = self.track(id, label, &track, value, &range);
        let typed = self.value_field_at(id, label, self.right - RAMP_VALUE_INSET, RAMP_VALUE_WIDTH, center_y, *value, &range, 2, "");

        let glyph = Rect::from_center_size(pos2(self.right - INVERT_INSET, center_y), vec2(GLYPH_TARGET, GLYPH_TARGET));
        let invert_id = self.next_id();
        let tip = crate::i18n::tr("Invert");
        let response = self.tip(self.ui.interact(glyph, invert_id, Sense::click()), tip);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
        let tokens = self.tokens;
        let color = if response.hovered() { tokens.text } else { tokens.text_dim };
        DrawnIcon::Invert.paint(self.ui.painter(), glyph.center(), INVERT_GLYPH_SIZE, color);
        self.drawn(center_y + KNOB_RADIUS);
        RampEdit { changed: Self::apply_typed(value, typed, dragged), invert: response.clicked() }
    }

    /// Controls the sheet has no row for yet, laid out by egui between the panel's edges
    pub(crate) fn embed<R>(&mut self, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
        let width = self.right - self.left;
        let top = self.cursor_y();
        let inner = self.ui.horizontal(|ui| {
            ui.add_space(self.left - ui.cursor().left());
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = CARD_ITEM_SPACING;
                body(ui)
            })
            .inner
        });
        let bottom = inner.response.rect.bottom().max(top);
        self.empty = false;
        self.drawn(bottom);
        inner.inner
    }
}

/// A horizontal black-to-white ramp filling `rect`
fn gray_ramp(rect: Rect) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), Color32::BLACK);
    mesh.colored_vertex(rect.right_top(), Color32::WHITE);
    mesh.colored_vertex(rect.right_bottom(), Color32::WHITE);
    mesh.colored_vertex(rect.left_bottom(), Color32::BLACK);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    mesh
}
