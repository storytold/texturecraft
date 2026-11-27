//! The picture a slot card shows of a fill channel's source

use egui::{Align2, Color32, CornerRadius, FontId, Painter, Rect, Stroke, StrokeKind, TextureHandle, TextureId, pos2};
use egui_phosphor::regular as ic;
use texturecraft_engine::compose::ThumbTarget;
use texturecraft_engine::core::Source;

use crate::TextureCraftApp;
use crate::fill_material_panel::shown_color;
use crate::theme::Tokens;

/// The picture a slot shows of its source
pub(crate) enum SlotThumbnail {
    Swatch(Color32),
    Texture(TextureHandle),
    /// A picture of the layer stack (an anchor point's), drawn over the field colour where the stack has nothing
    Stack(TextureId),
    Glyph(&'static str),
}

/// The picture of `source`: its uniform colour, the procedural's or image's thumbnail, the anchored stack, a glyph for what has none
pub(crate) fn thumbnail(app: &mut TextureCraftApp, ctx: &egui::Context, source: &Source) -> SlotThumbnail {
    match source {
        Source::Uniform(value) => SlotThumbnail::Swatch(shown_color(*value)),
        Source::Procedural { pattern, .. } => app.thumbs.procedural(ctx, pattern.kind).map_or(SlotThumbnail::Glyph(ic::WAVE_SINE), SlotThumbnail::Texture),
        Source::Image { name, .. } => match app.session.images.get(name).cloned() {
            Some(image) => SlotThumbnail::Texture(app.thumbs.image(ctx, name, &image)),
            None => SlotThumbnail::Glyph(ic::IMAGE),
        },
        // The stack the anchor point keeps, as the Layers panel shows that point (the glyph while it renders)
        Source::Anchor { anchor, channel, .. } => app
            .layer_panel
            .texture_outside(ctx, &app.session, ThumbTarget::Stage { effect: *anchor, channel: *channel })
            .map_or(SlotThumbnail::Glyph(ic::ANCHOR), SlotThumbnail::Stack),
    }
}

/// The glyph size of a slot without a picture
const GLYPH_SIZE: f32 = 14.0;

/// Corner radius of a rounded slot picture and of the empty square
pub(crate) const THUMB_RADIUS: u8 = 3;

/// How far the empty square's outline sits inside the slot's square
const EMPTY_SLOT_INSET: f32 = 1.0;

/// The whole of a texture, as the part of it a picture shows
pub(crate) const FULL_UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

/// Paints `thumbnail` into a slot's square
pub(crate) fn paint_thumbnail(painter: &Painter, rect: Rect, thumbnail: &SlotThumbnail, tokens: &Tokens) {
    match thumbnail {
        SlotThumbnail::Swatch(color) => {
            painter.rect_filled(rect, CornerRadius::ZERO, *color);
        }
        SlotThumbnail::Texture(texture) => {
            painter.image(texture.id(), rect, FULL_UV, Color32::WHITE);
        }
        SlotThumbnail::Stack(texture) => {
            painter.rect_filled(rect, CornerRadius::ZERO, tokens.field);
            painter.image(*texture, rect, FULL_UV, Color32::WHITE);
        }
        SlotThumbnail::Glyph(glyph) => {
            painter.rect_filled(rect, CornerRadius::ZERO, tokens.field);
            painter.text(rect.center(), Align2::CENTER_CENTER, *glyph, FontId::proportional(GLYPH_SIZE), tokens.text_dim);
        }
    }
}

/// A slot without a picture (no ID map, a missing source): an empty outlined square
pub(crate) fn paint_empty_slot(painter: &Painter, rect: Rect, tokens: &Tokens) {
    painter.rect_stroke(rect.shrink(EMPTY_SLOT_INSET), CornerRadius::same(THUMB_RADIUS), Stroke::new(1.0, tokens.text_dim), StrokeKind::Inside);
}
