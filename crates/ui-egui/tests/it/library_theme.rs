//! craft-ui's parts (the dock's frames and tab strips) draw in TextureCraft's colours, not the suite's default grey

use craft_ui::style::StyleCx;
use egui::Color32;
use egui_kittest::Harness;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::theme::{Brightness, Tokens};

use crate::{BAKE_RESOLUTION, SETTLE_FRAMES, TEXTURE_RESOLUTION};

/// A window wide enough for the dock's groups, strips and splitters
const WINDOW_SIZE: egui::Vec2 = egui::vec2(1200.0, 800.0);

fn srgba(color: craft_theme::Rgba) -> Color32 {
    let [r, g, b, a] = color.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

#[test]
fn the_dock_strips_and_panel_bodies_take_the_app_theme_colours() {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder()
        .with_size(WINDOW_SIZE)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    for brightness in [Brightness::Dark, Brightness::MediumDark, Brightness::MediumLight, Brightness::Light] {
        harness.state_mut().ui.prefs.brightness = brightness;
        harness.run_steps(SETTLE_FRAMES);
        let tokens = Tokens::get(&harness.ctx);
        let (strip, body, frame) = StyleCx::with_root(&harness.ctx, |cx| {
            let color = &cx.tokens.color;
            (color.surface.background, color.surface.panel, color.line.border)
        });
        assert_eq!(srgba(strip), tokens.chrome, "{brightness:?}: the tab strips lie on the window chrome");
        assert_eq!(srgba(body), tokens.panel, "{brightness:?}: the shown tab and the panel body are the panel card");
        assert_eq!(srgba(frame), tokens.separator, "{brightness:?}: the group frames are the cards' hairline");
    }
}
