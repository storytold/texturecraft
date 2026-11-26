//! State behind the Layers panel: the thumbnail service, the GPU textures made from its pictures,
//! which effect stack is open, and the one place the preview size is decided.

use std::collections::HashMap;
use std::time::Duration;

use egui::{Context, TextureHandle, TextureId, TextureOptions};
use texturecraft_engine::Session;
use texturecraft_engine::compose::ThumbTarget;
use texturecraft_engine::core::LayerId;
use texturecraft_engine::thumbs::{CACHE_ENTRIES, ThumbService};

use crate::TextureCraftApp;

/// Edge (points) of a layer's content and mask thumbnails. The Layers "Preview size" setting
/// plugs in by changing [`layer_preview_size`]; every row measure below derives from it.
pub(crate) const LAYER_PREVIEW_SIZE: f32 = 36.0;

/// The thumbnail edge in points, read once per row.
pub(crate) fn layer_preview_size(_app: &TextureCraftApp) -> f32 {
    LAYER_PREVIEW_SIZE
}

/// Height of a layer row for thumbnails of `size`.
pub(crate) fn layer_row_height(size: f32) -> f32 {
    (size + 12.0).max(48.0)
}

/// Edge of an effect's stage thumbnail for layer thumbnails of `size` (18 to 28 points).
pub(crate) fn effect_preview_size(size: f32) -> f32 {
    (size * 0.6).clamp(18.0, 28.0)
}

/// Height of an effect row.
pub(crate) fn effect_row_height(size: f32) -> f32 {
    (effect_preview_size(size) + 4.0).max(26.0)
}

#[derive(Default)]
pub struct LayerPanel {
    service: ThumbService,
    textures: HashMap<ThumbTarget, (u64, TextureHandle, u64)>,
    frame: u64,
    /// Whether this frame's pictures are begun and not yet finished by [`LayerPanel::end`]
    pictures_begun: bool,
    /// The open stack, with `true` for a layer's mask. This belongs to the panel view.
    pub(crate) expanded_effects: Option<(LayerId, bool)>,
}

/// How soon to look again for pictures still rendering (milliseconds)
const POLL_INTERVAL_MS: u64 = 40;

impl LayerPanel {
    pub(crate) fn toggle_effects(&mut self, stack: Option<(LayerId, bool)>) {
        self.expanded_effects = stack.filter(|stack| Some(*stack) != self.expanded_effects);
    }

    /// Start drawing the panel for this frame.
    pub(crate) fn begin(&mut self, session: &Session, now: f64) {
        self.begin_pictures(session, now);

        self.expanded_effects = self.expanded_effects.filter(|(layer, _)| session.doc.project.layers.contains_key(layer));
    }

    /// Advance the picture frame once, whether the panel or a picture outside it asks first
    fn begin_pictures(&mut self, session: &Session, now: f64) {
        if !self.pictures_begun {
            self.pictures_begun = true;
            self.frame += 1;
            self.service.begin_frame(session, now);
        }
    }

    /// The texture of `target` for a picture outside the panel (a fill source slot, a source picker's anchor points), which renders while the panel is hidden too
    pub(crate) fn texture_outside(&mut self, ctx: &Context, session: &Session, target: ThumbTarget) -> Option<TextureId> {
        self.begin_pictures(session, ctx.input(|input| input.time));
        self.texture(ctx, session, target)
    }

    /// The texture of `target`, queuing its render (call only for rows that are on screen).
    pub(crate) fn texture(&mut self, ctx: &Context, session: &Session, target: ThumbTarget) -> Option<TextureId> {
        let view = self.service.get(session, target);
        let image = view.image?;
        let frame = self.frame;
        if let Some((version, tex, used)) = self.textures.get_mut(&target)
            && *version == view.version
        {
            *used = frame;
            return Some(tex.id());
        }
        let pixels = egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.rgba);
        let tex = ctx.load_texture("layer-thumb", pixels, TextureOptions::LINEAR);
        let id = tex.id();
        self.textures.insert(target, (view.version, tex, frame));
        Some(id)
    }

    /// Finish the frame once everything shown has asked for its pictures: collect them, start the missing ones and ask for another frame while work is outstanding
    pub(crate) fn end(&mut self, ctx: &Context, session: &Session) {
        if !std::mem::take(&mut self.pictures_begun) {
            return;
        }

        if self.service.poll(session) {
            ctx.request_repaint_after(Duration::from_millis(POLL_INTERVAL_MS));
        }
        if self.textures.len() > CACHE_ENTRIES * 2 {
            let frame = self.frame;
            self.textures.retain(|_, (_, _, used)| *used + 1 >= frame);
        }
    }

    /// The service, for tests.
    pub fn service(&self) -> &ThumbService {
        &self.service
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_engine::core::Channel;
    use texturecraft_engine::geom::primitives;

    #[test]
    fn deleted_layers_close_their_effect_stack() {
        let mut session = Session::new_project("thumb", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        let layer = session.selected_layer().unwrap();
        let mut panel = LayerPanel::default();
        panel.expanded_effects = Some((layer, false));
        panel.begin(&session, 0.0);
        assert_eq!(panel.expanded_effects, Some((layer, false)), "existing layers keep their view settings");

        texturecraft_engine::execute(&mut session, "layer.remove", &serde_json::json!({"layer": layer.0})).unwrap();
        panel.begin(&session, 1.0);

        assert_eq!(panel.expanded_effects, None, "the deleted layer leaves no open stack");
    }

    #[test]
    fn a_picture_outside_the_panel_renders_while_the_panel_is_hidden() {
        // Long enough for the render worker on a busy machine
        const ATTEMPTS: usize = 500;
        const WAIT: Duration = Duration::from_millis(10);
        let session = Session::new_project("thumb", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        let target = ThumbTarget::Content { layer: session.selected_layer().unwrap(), channel: Channel::BaseColor };
        let context = Context::default();
        let mut panel = LayerPanel::default();

        // The panel never begins a frame: only the outside picture asks
        let picture = (0..ATTEMPTS).find_map(|_| {
            let picture = panel.texture_outside(&context, &session, target);
            panel.end(&context, &session);
            if picture.is_none() {
                std::thread::sleep(WAIT);
            }
            picture
        });

        assert!(picture.is_some(), "the picture renders without the Layers panel");
    }

    #[test]
    fn outside_pictures_release_old_textures_while_the_panel_is_hidden() {
        let session = Session::new_project("thumb", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        let context = Context::default();
        let initial_textures = context.tex_manager().read().num_allocated();
        let mut panel = LayerPanel::default();

        // Outside pictures begin frames even when the Layers panel never does
        let missing = ThumbTarget::Content { layer: LayerId(u64::MAX), channel: Channel::BaseColor };
        panel.texture_outside(&context, &session, missing);
        for index in 0..CACHE_ENTRIES * 2 + 1 {
            let target = ThumbTarget::Content { layer: LayerId(index as u64), channel: Channel::BaseColor };
            let image = egui::ColorImage::filled([64, 64], egui::Color32::WHITE);
            let texture = context.load_texture("layer-thumb", image, TextureOptions::LINEAR);
            panel.textures.insert(target, (1, texture, panel.frame));
        }
        assert_eq!(context.tex_manager().read().num_allocated(), initial_textures + CACHE_ENTRIES * 2 + 1);
        panel.end(&context, &session);

        for _ in 0..2 {
            panel.texture_outside(&context, &session, missing);
            panel.end(&context, &session);
        }

        assert_eq!(panel.textures.len(), 0, "old outside pictures should leave the GPU cache");
        assert_eq!(context.tex_manager().read().num_allocated(), initial_textures);
    }

    #[test]
    fn evicted_and_dropped_thumbnail_handles_release_textures() {
        let session = Session::new_project("thumb", primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        let context = Context::default();
        let initial_textures = context.tex_manager().read().num_allocated();
        let mut panel = LayerPanel::default();
        let mut evicted = Vec::new();
        panel.begin(&session, 0.0);

        // Simulate scrolling through more textures than the UI cache retains
        for index in 0..CACHE_ENTRIES * 2 + 1 {
            let target = ThumbTarget::Content { layer: LayerId(index as u64), channel: Channel::BaseColor };
            let image = egui::ColorImage::filled([64, 64], egui::Color32::WHITE);
            let texture = context.load_texture("layer-thumb", image, TextureOptions::LINEAR);
            evicted.push(texture.id());
            panel.textures.insert(target, (1, texture, panel.frame));
        }
        let kept = evicted.pop().unwrap();
        panel.end(&context, &session);
        panel.begin(&session, 1.0);
        panel.end(&context, &session);
        panel.begin(&session, 2.0);
        panel.textures.values_mut().find(|(_, texture, _)| texture.id() == kept).unwrap().2 = panel.frame;
        panel.end(&context, &session);

        assert_eq!(context.tex_manager().read().num_allocated(), initial_textures + 1);
        assert!(evicted.iter().all(|id| context.tex_manager().read().meta(*id).is_none()));
        assert!(context.tex_manager().read().meta(kept).is_some());

        drop(panel);

        assert_eq!(context.tex_manager().read().num_allocated(), initial_textures);
    }
}
