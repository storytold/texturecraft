//! A tool variant's icon: its tool's glyph with a small badge for the variant that is not the plain one

use egui::{Align2, Color32, FontId, Painter, Rect, vec2};
use texturecraft_engine::tool_variant::ToolVariant;

use crate::drawn_icon::DrawnIcon;
use crate::toolbar::tool_icon;
use crate::widgets::ICON_SCALE;

/// The badge's size, and its centre's inset from the lower-left corner, as fractions of the icon's square
const BADGE_SCALE: f32 = 0.38;
const BADGE_INSET_SCALE: f32 = 0.27;

/// The disc of ground under the badge that cuts it out of the glyph, as a fraction of the badge's size
const CUT_OUT_SCALE: f32 = 0.66;

/// Draws `variant`'s icon in the square `rect` in `color`; `ground` is the on-screen colour under it, parting badge from glyph
pub(crate) fn paint(painter: &Painter, rect: Rect, variant: ToolVariant, color: Color32, ground: Color32) {
    // A greyed UI fades what it paints, which would let the glyph show through the cut-out: fade the icon's colours instead
    let color = color.gamma_multiply(painter.opacity());
    let mut painter = painter.clone();
    painter.set_opacity(1.0);

    let side = rect.height();
    let glyph_size = side * ICON_SCALE;

    // The filled path is its own glyph: the nib drawn solid
    if variant == ToolVariant::PathFilled {
        DrawnIcon::FilledNib.paint(&painter, rect.center(), glyph_size, color);
        return;
    }

    painter.text(rect.center(), Align2::CENTER_CENTER, tool_icon(variant.tool()), FontId::proportional(glyph_size), color);
    let badge_size = side * BADGE_SCALE;
    let badge_centre = rect.left_bottom() + vec2(side, -side) * BADGE_INSET_SCALE;
    if variant.physical() {
        painter.circle_filled(badge_centre, badge_size * CUT_OUT_SCALE, ground);
        DrawnIcon::Drop.paint(&painter, badge_centre, badge_size, color);
    } else if variant == ToolVariant::CloneAbsolute {
        painter.circle_filled(badge_centre, badge_size * CUT_OUT_SCALE, ground);
        painter.text(badge_centre, Align2::CENTER_CENTER, egui_phosphor::regular::CROSSHAIR, FontId::proportional(badge_size), color);
    }
}
