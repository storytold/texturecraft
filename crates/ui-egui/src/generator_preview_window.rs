//! A generator's preview enlarged in a tool window, following its settings while they change

use std::time::Duration;

use egui::{Align2, Color32, Context, CornerRadius, Rect, Role, Sense, WidgetInfo, pos2, vec2};
use texturecraft_engine::core::{EffectKind, LayerId};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::shelf_preview::ShelfPreview;
use crate::theme::Tokens;

/// The width the enlarged picture is rendered at, in pixels: the most a preview renders
const PICTURE_PX: usize = texturecraft_engine::orb_preview::MAX_PREVIEW_SIZE_PX;

/// The window's first width, in points
const DEFAULT_WIDTH: f32 = 420.0;

/// The narrowest the picture shrinks to, in points
const MIN_SIDE: f32 = 160.0;

/// From the small preview's left edge to the window's right edge when it first opens, in points: clear of the properties panel
const BESIDE_GAP: f32 = 24.0;

/// How soon a picture still rendering is looked for again
const PICTURE_POLL: Duration = Duration::from_millis(100);

/// The open enlarged preview, if any: closed by its ×, and when its generator is gone
pub(crate) fn show(app: &mut TextureCraftApp, ctx: &Context) {
    // Pictures of generators gone from the project are dropped, whether or not their window is open
    let project = &app.session.doc.project;
    app.thumbs.keep_generator_pictures(|effect| project.effect(LayerId(effect)).is_some());

    let Some(effect) = app.ui.windows.generator_preview else { return };
    let generator = match app.session.doc.project.effect(LayerId(effect)).map(|(_, effect)| &effect.kind) {
        Some(EffectKind::Generator(generator)) => generator.clone(),
        _ => {
            app.ui.windows.generator_preview = None;
            return;
        }
    };

    let set = ShelfPreview::generator_set(&generator, &app.session.images);
    let texture = app.thumbs.generator_set_picture(ctx, effect, set, PICTURE_PX);
    let tokens = Tokens::get(ctx);

    // It first opens beside the small preview it was asked from, then stays where it is moved
    let mut open = true;
    let beside =
        ctx.data(|data| data.get_temp::<Rect>(anchor_id())).map_or(ctx.content_rect().center(), |small| pos2(small.left() - BESIDE_GAP, small.center().y));
    let window = crate::windows::window("generator_preview", tr(generator.kind.label()), &mut open).default_width(DEFAULT_WIDTH);
    window.default_pos(beside).pivot(Align2::RIGHT_CENTER).show(ctx, |ui| {
        let side = ui.available_width().max(MIN_SIDE);
        let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
        response.widget_info(|| WidgetInfo::labeled(Role::Image, true, tr("Preview")));

        match &texture {
            Some(texture) => {
                ui.painter().image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                ui.painter().rect_filled(rect, CornerRadius::ZERO, tokens.button);
                ctx.request_repaint_after(PICTURE_POLL);
            }
        }
    });

    if !open {
        app.ui.windows.generator_preview = None;
    }
}

/// Opens generator effect `effect`'s enlarged preview beside its small one at `small`, or closes it when it is the one open
pub(crate) fn toggle(app: &mut TextureCraftApp, ctx: &Context, effect: u64, small: Rect) {
    ctx.data_mut(|data| data.insert_temp(anchor_id(), small));
    let open = &mut app.ui.windows.generator_preview;
    *open = if *open == Some(effect) { None } else { Some(effect) };
}

/// Where the small preview was when the window was asked for: it first opens beside it
fn anchor_id() -> egui::Id {
    egui::Id::unique("generator_preview_anchor")
}
