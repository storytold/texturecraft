//! The color picker popup keeps its place while the color changes, picks from its square and hue
//! strip, edits hex colors, pastes RGB values, and copies its values.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_ui_egui::{theme, widgets};

/// A swatch at the right edge of the window, like the Properties panel's, with its picker open.
fn open_picker(color: [f32; 3]) -> Harness<'static, [f32; 3]> {
    let mut h = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, color: &mut [f32; 3]| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                widgets::color_swatch(ui, "color", color, 120.0);
            });
        },
        color,
    );
    h.ctx.set_fonts(theme::font_definitions());
    theme::apply(&h.ctx, theme::Brightness::default(), theme::Accent::default());
    h.run_steps(2);
    let swatch = egui::pos2(800.0 - 60.0, 9.0);
    h.drag_at(swatch);
    h.drop_at(swatch);
    h.run_steps(3);
    h
}

/// The picker's R value.
fn r_value(h: &Harness<'_, [f32; 3]>) -> egui::Rect {
    h.get_all_by_role(Role::SpinButton).next().unwrap().rect()
}

/// Click at `p` and let the picker apply it.
fn click(h: &mut Harness<'_, [f32; 3]>, p: egui::Pos2) {
    h.drag_at(p);
    h.drop_at(p);
    h.run_steps(3);
}

fn replace_color_text(h: &mut Harness<'_, [f32; 3]>, event: egui::Event) {
    h.get_by_role_and_label(Role::TextInput, "Hex").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(event);
    h.run_steps(3);
}

fn assert_srgb(h: &Harness<'_, [f32; 3]>, expected: [u8; 3]) {
    let actual = h.state().map(|v| (texturecraft_engine::texel::linear_to_srgb(v) * 255.0).round() as u8);
    assert_eq!(actual, expected);
}

/// Find the rendered gradient, so tests do not assume the height of the rows above it.
fn gradient_rect(h: &Harness<'_, [f32; 3]>, square: bool) -> egui::Rect {
    h.output()
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Mesh(mesh) => {
                let rect = mesh.calc_bounds();
                (rect.width() > 200.0 && (rect.width() == rect.height()) == square).then_some(rect)
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn hex_colors_can_be_typed_or_pasted() {
    let mut h = open_picker([1.0; 3]);
    for (event, expected) in [
        (egui::Event::Text("#FF0000".into()), [255, 0, 0]),
        (egui::Event::Paste("  #1a80cF  ".into()), [26, 128, 207]),
        (egui::Event::Paste("336699".into()), [51, 102, 153]),
    ] {
        replace_color_text(&mut h, event);
        assert_srgb(&h, expected);
    }
}

#[test]
fn rgb_values_can_be_pasted_including_the_copy_buttons_format() {
    let mut h = open_picker([1.0; 3]);
    for (text, expected) in [("255,0,0", [255, 0, 0]), (" 32, 128, 240 ", [32, 128, 240])] {
        replace_color_text(&mut h, egui::Event::Paste(text.into()));
        assert_srgb(&h, expected);
    }
    h.get_by_label("Copy RGB values").click();
    h.step();
    let copied = h
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .unwrap();
    *h.state_mut() = [0.0; 3];
    h.run_steps(3);
    replace_color_text(&mut h, egui::Event::Paste(copied));
    assert_srgb(&h, [32, 128, 240]);
}

#[test]
fn incomplete_or_invalid_color_text_does_not_change_the_color() {
    let mut h = open_picker([1.0, 0.0, 0.0]);
    for text in ["", "#12", "#gg0000", "#+f+f+f", "#12345678", "256,0,0", "-1,0,0", "1.5,0,0", "1,2", "1,2,3,4", "色0000"] {
        replace_color_text(&mut h, egui::Event::Paste(text.into()));
        assert_srgb(&h, [255, 0, 0]);
    }
    replace_color_text(&mut h, egui::Event::Text("#12".into()));
    h.event(egui::Event::Text("3456".into()));
    h.run_steps(3);
    assert_srgb(&h, [18, 52, 86]);
}

#[test]
fn the_hex_field_tracks_external_changes_and_normalizes_on_blur() {
    let mut h = open_picker([1.0, 0.0, 0.0]);
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Hex").value().as_deref(), Some("#FF0000"));
    replace_color_text(&mut h, egui::Event::Paste("0,128,255".into()));
    let away = r_value(&h).center();
    click(&mut h, away);
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Hex").value().as_deref(), Some("#0080FF"));
    *h.state_mut() = [1.0; 3];
    h.run_steps(3);
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Hex").value().as_deref(), Some("#FFFFFF"));
}

#[test]
fn paste_applies_to_the_open_picker_without_focusing_a_field() {
    let mut h = open_picker([1.0; 3]);
    for (text, expected) in [("#FF0000", [255, 0, 0]), ("0,128,255", [0, 128, 255]), ("#bad!", [0, 128, 255])] {
        assert!(!h.ctx.text_edit_focused(), "no text field is focused");
        h.event(egui::Event::Paste(text.into()));
        h.run_steps(3);
        assert_srgb(&h, expected);
    }
}

#[test]
fn paste_replaces_the_whole_color_while_an_rgb_channel_is_being_edited() {
    let mut h = open_picker([1.0; 3]);
    let red = r_value(&h).center();
    click(&mut h, red);
    assert!(h.ctx.text_edit_focused());
    h.event(egui::Event::Paste("#2080F0".into()));
    h.run_steps(3);
    assert_srgb(&h, [32, 128, 240]);
}

#[test]
fn an_open_picker_takes_paste_before_the_apps_layer_shortcuts() {
    use texturecraft_engine::core::Channel;
    use texturecraft_ui_egui::TextureCraftApp;

    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    texturecraft_engine::execute(&mut session, "layer.copy", &serde_json::Value::Null).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h.get_by_role_and_label(Role::Button, "Material").click();
    h.run_steps(3);
    let label = h.get_by_label("Base color").rect();
    let swatch = egui::pos2(label.right() + 30.0, label.center().y);
    h.drag_at(swatch);
    h.drop_at(swatch);
    h.run_steps(3);
    assert!(h.query_by_label("Copy RGB values").is_some());
    let layers = h.state().session.doc.project.layers.len();
    for text in ["#FF0000", "0,128,255", "not a color"] {
        h.event(egui::Event::Paste(text.into()));
        h.run_steps(3);
        assert_eq!(h.state().session.doc.project.layers.len(), layers, "paste belongs to the picker");
    }
    let image = std::sync::Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::WHITE));
    h.event(egui::Event::PasteImage(image));
    h.run_steps(3);
    assert_eq!(h.state().session.doc.project.layers.len(), layers, "an image paste must not paste a layer behind the picker");
    let color = h.state().session.tool.values[&Channel::BaseColor];
    let srgb = color.map(|v| (texturecraft_engine::texel::linear_to_srgb(v) * 255.0).round() as u8);
    assert_eq!(srgb, [0, 128, 255]);

    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label("Copy RGB values").is_none());
    h.event(egui::Event::Paste("Layer".into()));
    h.run_steps(3);
    assert_eq!(h.state().session.doc.project.layers.len(), layers + 1, "closing the picker restores layer paste");
}

#[test]
fn the_popup_stays_put_while_the_color_changes() {
    let mut h = open_picker([1.0; 3]);
    let bright = r_value(&h).left();
    *h.state_mut() = [0.001; 3];
    h.run_steps(3);
    assert_eq!(r_value(&h).left(), bright, "the picker does not move when R, G and B lose digits");
}

#[test]
fn the_square_and_the_hue_strip_pick_the_color() {
    let mut h = open_picker([1.0; 3]);
    let square = gradient_rect(&h, true);
    click(&mut h, square.left_bottom() + egui::vec2(2.0, -6.0));
    assert!(h.state().iter().all(|c| *c < 0.01), "black: {:?}", h.state());
    // Full saturation and value, then a hue a third of the way along: green.
    click(&mut h, square.right_top() + egui::vec2(-1.0, 1.0));
    let green = gradient_rect(&h, false).lerp_inside(egui::vec2(1.0 / 3.0, 0.5));
    click(&mut h, green);
    let [r, g, b] = *h.state();
    assert!(g > 0.9 && r < 0.05 && b < 0.05, "green: {:?}", h.state());
}

#[test]
fn the_copy_button_copies_the_rgb_values() {
    let mut h = open_picker([1.0, 0.0, 0.0]);
    h.get_by_label("Copy RGB values").click();
    h.step();
    let copied = h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "255, 0, 0"));
    assert!(copied, "{:?}", h.output().platform_output.commands);
}

#[test]
fn shorthand_hex_and_rgb_function_text_can_be_pasted_without_focus() {
    let mut h = open_picker([1.0; 3]);
    for (text, expected) in [("#f0a", [255, 0, 170]), ("rgb(10, 20, 30)", [10, 20, 30]), ("1 2 3", [1, 2, 3])] {
        h.event(egui::Event::Paste(text.into()));
        h.run_steps(3);
        assert_srgb(&h, expected);
    }
}

#[test]
fn escape_discards_a_pending_hex_edit_and_keeps_the_applied_color() {
    let mut h = open_picker([0.0, 0.0, 1.0]);
    replace_color_text(&mut h, egui::Event::Text("#FF0000".into()));
    assert_srgb(&h, [255, 0, 0]);
    replace_color_text(&mut h, egui::Event::Text("#0F0".into()));
    assert_srgb(&h, [255, 0, 0]);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_srgb(&h, [255, 0, 0]);
}

#[test]
fn short_hex_is_committed_on_enter_or_blur_but_not_during_typing() {
    let mut h = open_picker([1.0, 0.0, 0.0]);
    replace_color_text(&mut h, egui::Event::Text("#00F".into()));
    assert_srgb(&h, [255, 0, 0]);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_srgb(&h, [0, 0, 255]);
    replace_color_text(&mut h, egui::Event::Text("#0F0".into()));
    assert_srgb(&h, [0, 0, 255]);
    let away = r_value(&h).center();
    click(&mut h, away);
    assert_srgb(&h, [0, 255, 0]);
}

#[test]
fn the_hex_copy_button_copies_a_complete_color() {
    let mut h = open_picker([1.0, 0.0, 0.0]);
    h.get_by_label("Copy Hex code").click();
    h.step();
    let copied = h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "#FF0000"));
    assert!(copied);
}

#[test]
fn leaving_the_hex_field_for_the_hue_strip_keeps_the_new_color() {
    let mut h = open_picker([1.0, 0.0, 0.0]);
    h.get_by_role_and_label(Role::TextInput, "Hex").click();
    h.run_steps(3);
    let green = gradient_rect(&h, false).lerp_inside(egui::vec2(1.0 / 3.0, 0.5));
    click(&mut h, green);
    assert_srgb(&h, [0, 255, 0]);
}

#[test]
fn committing_an_unchanged_gray_color_keeps_the_remembered_hue() {
    let mut h = open_picker([0.0, 1.0, 0.0]);
    let square = gradient_rect(&h, true);
    let white = square.left_top();
    click(&mut h, white);
    assert_srgb(&h, [255; 3]);
    replace_color_text(&mut h, egui::Event::Text("#FFFFFF".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    h.event(egui::Event::Paste("#FFFFFF".into()));
    h.run_steps(3);
    click(&mut h, square.right_top() - egui::vec2(0.1, 0.0));
    assert_srgb(&h, [0, 255, 0]);
}

#[test]
fn unfinished_hex_input_survives_switching_away_from_the_window() {
    let mut h = open_picker([1.0, 0.0, 0.0]);
    replace_color_text(&mut h, egui::Event::Text("#00F".into()));
    h.input_mut().focused = false;
    h.event(egui::Event::WindowFocused(false));
    h.run_steps(3);
    assert!(!h.ctx.input(|input| input.focused));
    assert_eq!(h.get_by_role_and_label(Role::TextInput, "Hex").value().as_deref(), Some("#00F"));
    h.input_mut().focused = true;
    h.event(egui::Event::WindowFocused(true));
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_srgb(&h, [0, 0, 255]);
}
