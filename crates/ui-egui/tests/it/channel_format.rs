//! Texture Set Settings: every channel row has a storage format menu.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::core::{Channel, ChannelFormat};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    harness.state_mut().ui.dock.focus(Panel::TextureSetSettings);
    harness.run_steps(4);
    harness
}

fn format(h: &Harness<'_, TextureCraftApp>, c: Channel) -> ChannelFormat {
    let ts = h.state().session.ts();
    h.state().session.doc.project.texture_sets[ts].channels.iter().find(|(x, _)| *x == c).unwrap().1
}

#[test]
fn a_channel_row_changes_its_format_and_undo_restores_it() {
    let mut h = app();
    assert_eq!(format(&h, Channel::BaseColor), ChannelFormat::Srgb8);

    h.get_all_by_role(Role::ComboBox).find(|n| n.accesskit_node().value().as_deref() == Some("sRGB8")).expect("the base color format menu").click();
    h.run_steps(3);
    h.get_by_label("RGB16F").click();
    h.run_steps(3);
    assert_eq!(format(&h, Channel::BaseColor), ChannelFormat::Rgb16F);

    h.state_mut().run("edit.undo", serde_json::Value::Null);
    h.run_steps(3);
    assert_eq!(format(&h, Channel::BaseColor), ChannelFormat::Srgb8);
}
