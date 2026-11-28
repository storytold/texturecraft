//! The picture that follows a dragged asset or layer, with a badge saying whether the place under it takes it

use egui::{Color32, Context, Id, LayerId, Order, Pos2, Rect, Stroke, TextureId, Vec2, vec2};

use crate::shelf::ShelfDrag;
pub use crate::shelf_drag_ghost_shown::ShelfDragGhostShown;
pub use crate::shelf_drag_verdict::ShelfDragVerdict;
use crate::theme::Tokens;

/// Side of the dragged asset's thumbnail
const GHOST_SIZE: f32 = 48.0;

/// The thumbnail's top-left corner from the pointer: just under its tip
const GHOST_OFFSET: Vec2 = vec2(4.0, 4.0);

/// How opaque the thumbnail is, so the target shows through it
const GHOST_OPACITY: f32 = 0.6;

/// Corner rounding of the name chip
const CHIP_ROUNDING: f32 = 3.0;

/// Side of the badge
const BADGE_SIZE: f32 = 9.0;

/// The badge's centre from the pointer: past the pointer arrow's tail, on the thumbnail's corner
const BADGE_OFFSET: Vec2 = vec2(14.0, 22.0);

/// Size of the badge's icon
const BADGE_ICON_SIZE: f32 = 7.5;

/// Corner rounding of the square badge
const BADGE_ROUNDING: f32 = 2.0;

/// Fill of the square badge and colour of the icon on the red one
const BADGE_LIGHT: Color32 = Color32::from_gray(245);

/// Colour of the icon on the square badge
const BADGE_DARK: Color32 = Color32::from_gray(35);

/// Opacity of the thin dark outline that keeps the badge apart from whatever is under it
const BADGE_OUTLINE_ALPHA: u8 = 140;

/// Padding of the name chip shown when the asset has no thumbnail
const CHIP_PADDING: Vec2 = vec2(6.0, 3.0);

/// Text size of the name chip
const CHIP_TEXT_SIZE: f32 = 11.0;

/// Width of the outline around a drop target that takes the dragged asset
const TARGET_OUTLINE_WIDTH: f32 = 2.0;

/// How far the outline sits inside the target's edge, so a neighbouring panel doesn't cut it
const TARGET_OUTLINE_INSET: f32 = 1.0;

/// Corner rounding of the outline
const TARGET_OUTLINE_ROUNDING: f32 = 3.0;

/// Pass number of the last drop target that took the dragged asset (ctx temp data)
const ACCEPTED: &str = "shelf_drag_accepted";

/// What the last pass painted (ctx temp data)
const SHOWN: &str = "shelf_drag_shown";

/// The name of the layer being dragged in the layer stack, whose payload is only its id (ctx temp data)
const LAYER: &str = "shelf_drag_layer";

/// Note that the drop target being drawn takes the dragged asset or layer, where it paints its drop highlight
pub(crate) fn accept(ctx: &Context) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(Id::unique(ACCEPTED), pass));
}

/// Outlines drop target `rect` while the dragged asset hovers it, and notes that it takes the asset
pub(crate) fn outline_target(ui: &egui::Ui, rect: Rect) {
    let stroke = Stroke::new(TARGET_OUTLINE_WIDTH, Tokens::get(ui.ctx()).accent);
    ui.painter().rect_stroke(rect.shrink(TARGET_OUTLINE_INSET), TARGET_OUTLINE_ROUNDING, stroke, egui::StrokeKind::Inside);
    accept(ui.ctx());
}

/// Note the name of the layer whose drag starts: a chip with it follows the pointer, since an empty layer's thumbnail shows nothing
pub(crate) fn start_layer(ctx: &Context, label: &str) {
    ctx.data_mut(|data| data.insert_temp(Id::unique(LAYER), label.to_owned()));
}

/// The picture and name of what is being dragged: an asset, or a layer (its name only)
fn dragged(ctx: &Context) -> Option<(Option<TextureId>, String)> {
    if let Some(drag) = egui::DragAndDrop::payload::<ShelfDrag>(ctx) {
        return Some((drag.thumb.as_ref().map(egui::TextureHandle::id), drag.label.clone()));
    }

    if !egui::DragAndDrop::has_payload_of_type::<u64>(ctx) {
        return None;
    }

    ctx.data(|data| data.get_temp::<String>(Id::unique(LAYER))).map(|label| (None, label))
}

/// The ghost painted this pass, if an asset or a layer is being dragged
pub fn shown(ctx: &Context) -> Option<ShelfDragGhostShown> {
    ctx.data(|data| data.get_temp(Id::unique(SHOWN)))
}

/// Paint the dragged asset and its badge at the pointer, once a pass after every drop target
pub(crate) fn show(ctx: &Context) {
    let (Some((thumb, label)), Some(pointer)) = (dragged(ctx), ctx.input(|input| input.pointer.hover_pos())) else {
        ctx.data_mut(|data| data.remove::<ShelfDragGhostShown>(Id::unique(SHOWN)));
        return;
    };

    let pass = ctx.cumulative_pass_nr();
    let accepted = ctx.data(|data| data.get_temp::<u64>(Id::unique(ACCEPTED))) == Some(pass);
    let verdict = if accepted { ShelfDragVerdict::Valid } else { ShelfDragVerdict::NotValid };
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::unique("shelf_drag_ghost")));
    let tokens = Tokens::get(ctx);

    let ghost = match thumb {
        Some(thumb) => {
            let rect = Rect::from_min_size(pointer + GHOST_OFFSET, Vec2::splat(GHOST_SIZE));
            let uv = Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0));
            painter.image(thumb, rect, uv, Color32::WHITE.gamma_multiply(GHOST_OPACITY));
            rect
        }
        None => name_chip(&painter, &tokens, pointer + GHOST_OFFSET, &label),
    };
    let badge = badge(&painter, &tokens, pointer + BADGE_OFFSET, verdict);
    ctx.data_mut(|data| data.insert_temp(Id::unique(SHOWN), ShelfDragGhostShown { verdict, ghost, badge }));
}

/// The name in a chip at `top_left`, for what has no thumbnail (saved smart materials and masks, layers)
fn name_chip(painter: &egui::Painter, tokens: &Tokens, top_left: Pos2, label: &str) -> Rect {
    let galley = painter.layout_no_wrap(label.to_owned(), egui::FontId::proportional(CHIP_TEXT_SIZE), tokens.text_strong);
    let rect = Rect::from_min_size(top_left, galley.size() + CHIP_PADDING * 2.0);
    painter.rect_filled(rect, CHIP_ROUNDING, tokens.row_selected);
    painter.rect_stroke(rect, CHIP_ROUNDING, Stroke::new(1.0, tokens.accent), egui::StrokeKind::Inside);
    painter.galley(rect.min + CHIP_PADDING, galley, tokens.text_strong);
    rect
}

/// The badge at `centre`: a crosshair on a light square where a drop works, else a no-entry sign
fn badge(painter: &egui::Painter, tokens: &Tokens, centre: Pos2, verdict: ShelfDragVerdict) -> Rect {
    let rect = Rect::from_center_size(centre, Vec2::splat(BADGE_SIZE));
    let outline = Stroke::new(1.0, Color32::from_black_alpha(BADGE_OUTLINE_ALPHA));
    let (icon, color) = match verdict {
        ShelfDragVerdict::Valid => {
            painter.rect(rect, BADGE_ROUNDING, BADGE_LIGHT, outline, egui::StrokeKind::Outside);
            (egui_phosphor::regular::CROSSHAIR_SIMPLE, BADGE_DARK)
        }
        ShelfDragVerdict::NotValid => {
            painter.circle(centre, BADGE_SIZE / 2.0, tokens.danger, outline);
            (egui_phosphor::regular::PROHIBIT, BADGE_LIGHT)
        }
    };
    let glyph = painter.layout_no_wrap(icon.to_owned(), egui::FontId::proportional(BADGE_ICON_SIZE), color);
    painter.galley(centre - glyph.mesh_bounds.center().to_vec2(), glyph, color);
    rect.expand(outline.width)
}
