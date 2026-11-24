//! The filter slot in Properties swaps a filter through its picker, and Add filter adds one

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use texturecraft_engine::core::{Filter, LayerId};
use texturecraft_engine::{StackTarget, execute};
use texturecraft_ui_egui::TextureCraftApp;

use crate::filter_support::{WINDOW_HEIGHT, WINDOW_WIDTH, app_with, click_at, filter_of};

/// The app with an Invert filter on a fill layer's content, selected so Properties shows its slot
fn app_with_invert() -> (Harness<'static, TextureCraftApp>, LayerId) {
    app_with("invert", "content", WINDOW_HEIGHT)
}

/// Clicks the Invert slot, which opens its picker
fn open_picker(harness: &mut Harness<'_, TextureCraftApp>) {
    let at = harness.get_by_role_and_label(Role::ComboBox, "Invert").rect().center();
    click_at(harness, at);
    assert!(egui::Popup::is_any_open(&harness.ctx), "clicking the slot opens the picker");
}

#[test]
fn picking_from_the_slot_swaps_the_filter() {
    let (mut harness, effect) = app_with_invert();
    open_picker(&mut harness);

    // The picker's search field has the focus as it opens
    harness.event(egui::Event::Text("sharp".into()));
    harness.run_steps(3);

    assert_eq!(harness.query_all_by_label("Blur").count(), 0, "the search hides filters that don't match");

    let at = harness.get_by_label("Sharpen").rect().center();
    click_at(&mut harness, at);

    assert!(matches!(filter_of(&harness, effect), Filter::Sharpen { .. }), "{:?}", filter_of(&harness, effect));
    assert!(!egui::Popup::is_any_open(&harness.ctx), "a pick closes the picker");
}

#[test]
fn picking_blur_slope_installs_its_source_controls() {
    let (mut harness, effect) = app_with_invert();
    open_picker(&mut harness);
    harness.event(egui::Event::Text("blur slope".into()));
    harness.run_steps(3);

    let option = harness.get_by_label("Blur Slope").rect().center();
    click_at(&mut harness, option);

    assert!(matches!(filter_of(&harness, effect), Filter::SourceSlopeBlur(_)));
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Default Noise").is_some());
}

#[test]
fn escape_closes_the_picker_without_changes() {
    let (mut harness, effect) = app_with_invert();
    open_picker(&mut harness);

    // Without the search field's focus, the painting shortcuts see Escape first
    harness.ctx.memory_mut(|memory| memory.stop_text_input());
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    assert!(!egui::Popup::is_any_open(&harness.ctx));
    assert!(matches!(filter_of(&harness, effect), Filter::Invert));
}

/// Opens the wand's Add effect menu and clicks Filter
fn add_filter_from_the_wand(harness: &mut Harness<'_, TextureCraftApp>) {
    let at = harness.get_by_role_and_label(Role::Button, "Add effect").rect().center();
    click_at(harness, at);

    let at = harness.get_by_role_and_label(Role::Button, "Filter").rect().center();
    click_at(harness, at);
}

#[test]
fn add_filter_adds_a_filter_selects_it_and_opens_its_picker() {
    let (mut harness, _) = app_with_invert();
    add_filter_from_the_wand(&mut harness);

    let StackTarget::Effect(added) = harness.state().session.target else { panic!("the new filter is selected") };
    assert!(matches!(filter_of(&harness, added), Filter::Empty), "{:?}", filter_of(&harness, added));
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Filter").is_some(), "Properties shows the new filter's slot");
    assert!(egui::Popup::is_any_open(&harness.ctx), "the new filter's picker is open to choose the real filter");

    // The search field has the focus, and a search brings Sharpen into view
    harness.event(egui::Event::Text("sharp".into()));
    harness.run_steps(3);
    let at = harness.get_by_label("Sharpen").rect().center();
    click_at(&mut harness, at);

    assert!(matches!(filter_of(&harness, added), Filter::Sharpen { .. }), "{:?}", filter_of(&harness, added));
}

#[test]
fn escape_after_add_filter_keeps_the_empty_filter() {
    let (mut harness, _) = app_with_invert();
    add_filter_from_the_wand(&mut harness);

    assert!(harness.query_by_label("Sharpen").is_some(), "Add filter opened the picker");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);

    let StackTarget::Effect(added) = harness.state().session.target else { panic!("the new filter stays selected") };
    assert!(!egui::Popup::is_any_open(&harness.ctx));
    assert!(matches!(filter_of(&harness, added), Filter::Empty), "{:?}", filter_of(&harness, added));

    harness.state_mut().run("edit.undo", Value::Null);

    assert!(harness.state().session.doc.project.effect(added).is_none(), "one undo removes the empty filter");
}

#[test]
fn add_filter_on_a_mask_adds_it_to_the_mask() {
    let (mut harness, _) = app_with_invert();
    let layer = harness.state().session.selected_layer().unwrap();

    // A new mask becomes the selected target, so the wand adds to it
    harness.state_mut().run("mask.add.white", json!({"layer": layer.0}));
    harness.run_steps(3);

    add_filter_from_the_wand(&mut harness);

    let StackTarget::Effect(added) = harness.state().session.target else { panic!("the new filter is selected") };
    let mask = harness.state().session.doc.project.layer(layer).unwrap().mask.clone().unwrap();

    assert!(mask.effects.iter().any(|effect| effect.id == added), "the filter went on the mask");
    assert!(matches!(filter_of(&harness, added), Filter::Empty), "{:?}", filter_of(&harness, added));
    assert!(harness.query_by_label("Sharpen").is_some(), "the mask filter's picker is open");
    assert!(harness.query_by_label("Normal Invert").is_none(), "the picker leaves out content-only filters");
    assert!(harness.query_by_label("Grayscale Conversion").is_some(), "masks take Grayscale Conversion");
}

#[test]
fn add_filter_reopens_closed_properties_to_show_its_picker() {
    let (mut harness, _) = app_with_invert();
    harness.state_mut().ui.dock.close(texturecraft_ui_egui::docking::Panel::Properties);
    harness.run_steps(3);

    assert!(harness.query_by_role_and_label(Role::ComboBox, "Invert").is_none());

    add_filter_from_the_wand(&mut harness);

    assert!(harness.query_by_role_and_label(Role::ComboBox, "Filter").is_some(), "Add filter reveals Properties");
    assert!(harness.query_by_label("Sharpen").is_some(), "the picker opens immediately");
}

#[test]
fn keyboard_opens_the_slot_and_outside_click_closes_it() {
    let (mut harness, effect) = app_with_invert();
    for key in [egui::Key::Enter, egui::Key::Space] {
        harness.get_by_role_and_label(Role::ComboBox, "Invert").focus();
        harness.key_press(key);
        harness.run_steps(3);

        assert!(egui::Popup::is_any_open(&harness.ctx));

        let position = harness.state().viewport_rect.center();
        harness.hover_at(position);
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            harness.run_steps(1);
        }
        harness.run_steps(3);

        assert!(!egui::Popup::is_any_open(&harness.ctx));
        assert!(matches!(filter_of(&harness, effect), Filter::Invert));
    }
}

/// The snapshots' window height: a usual desktop window, the picker opening over the views
const SNAPSHOT_HEIGHT: f32 = 960.0;

#[test]
#[ignore = "requires an offscreen GPU adapter and TC_UI_SNAPSHOTS"]
fn filter_picker_snapshots_and_frame_timings() {
    let output = std::path::PathBuf::from(std::env::var_os("TC_UI_SNAPSHOTS").expect("set TC_UI_SNAPSHOTS"));
    std::fs::create_dir_all(&output).unwrap();
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    execute(&mut session, "layer.add.fill", &Value::Null).unwrap();
    execute(&mut session, "effect.add.filter", &json!({"target": "content", "filter": "invert"})).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(WINDOW_WIDTH, SNAPSHOT_HEIGHT)).wgpu().build_eframe(|cc| {
        eprintln!("selected graphics adapter: {:?}", cc.wgpu_render_state.as_ref().unwrap().adapter.get_info());
        TextureCraftApp::new(cc, session)
    });
    harness.run_steps(20);
    let start = std::time::Instant::now();
    open_picker(&mut harness);
    eprintln!("picker open and first 3 UI frames: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
    harness.run_steps(20);
    for (brightness, name) in
        [(texturecraft_ui_egui::theme::Brightness::MediumDark, "filter-picker"), (texturecraft_ui_egui::theme::Brightness::Light, "filter-picker-light")]
    {
        harness.state_mut().ui.prefs.brightness = brightness;
        harness.run_steps(3);
        harness.render().unwrap().save(output.join(format!("{name}.png"))).unwrap();
    }
    let start = std::time::Instant::now();
    harness.run_steps(120);
    eprintln!("warm open picker: {:.3} ms/UI frame (120 frames, excluding GPU readback)", start.elapsed().as_secs_f64() * 1000.0 / 120.0);

    harness.event(egui::Event::Text("sharp".into()));
    harness.run_steps(3);
    harness.render().unwrap().save(output.join("filter-picker-search.png")).unwrap();
    let at = harness.get_by_label("Sharpen").rect().center();
    click_at(&mut harness, at);
    harness.render().unwrap().save(output.join("filter-picker-selected.png")).unwrap();
}
