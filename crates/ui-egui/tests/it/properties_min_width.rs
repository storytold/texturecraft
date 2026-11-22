//! The Properties panel never gets narrower than its rows need: the column it docks in stops shrinking there

use craft_ui::docking::Node;
use craft_ui::layout::SplitSize;
use egui::{Event, Modifiers, PointerButton, Pos2, Role, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::{Dock, PROPERTIES_MIN_WIDTH};

use crate::{BAKE_RESOLUTION, DRAG_STEP_FRAMES, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION, panel_close_button};

const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);

/// A right column comfortably wider than the minimum, where a drag starts
const WIDE_COLUMN: f32 = 420.0;

/// Half the gutter between the views and the right column, where its divider is grabbed
const DIVIDER_OFFSET: f32 = 3.0;

/// How far under the Properties header the divider is grabbed: beside the panel's rows, clear of the header bar
const DIVIDER_GRAB_BELOW_HEADER: f32 = 100.0;

/// How far right of the window's edge a drag aims, past any width the column could stop at
const WINDOW_RIGHT_INSET: f32 = 4.0;

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The header names below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    harness
}

/// Gives the right column (Layers over Properties) `width` points, as a layout saved at that width would
fn set_column_width(harness: &mut Harness<'_, TextureCraftApp>, width: f32) {
    let mut workspace = harness.state().ui.dock.workspace().clone();
    let Some(Node::Split { second, .. }) = workspace.layout.root.as_mut() else { panic!("the assets column and the rest") };
    let Node::Split { size, .. } = second.as_mut() else { panic!("the 3D view and the right column") };

    *size = SplitSize::FixedSecond(width);
    harness.state_mut().ui.dock = Dock::from_value(serde_json::to_value(workspace).unwrap());
    harness.run_steps(SETTLE_FRAMES);
}

/// The Properties header's title, which starts at the column's left edge
fn properties_title(harness: &Harness<'_, TextureCraftApp>) -> egui::Rect {
    harness
        .get_all_by_role(Role::Tab)
        .find(|tab| tab.accesskit_node().label().is_some_and(|label| label.starts_with("Properties - ")))
        .map(|tab| tab.rect())
        .expect("the Properties header")
}

/// Width of the Properties header bar, from its title's left edge to its close button's right edge
fn properties_header_width(harness: &Harness<'_, TextureCraftApp>) -> f32 {
    let title = properties_title(harness);
    panel_close_button(harness, title).max.x - title.min.x
}

/// Moves of a divider drag, so the dock follows the pointer as it does by hand
const DRAG_MOVES: usize = 4;

/// Drags with the primary button from `from` to `to` in [`DRAG_MOVES`] moves
fn drag(harness: &mut Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2) {
    harness.hover_at(from);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(DRAG_STEP_FRAMES);

    for step in 1..=DRAG_MOVES {
        harness.hover_at(from + (to - from) * (step as f32 / DRAG_MOVES as f32));
        harness.run_steps(1);
    }

    harness.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn dragging_the_right_column_narrow_stops_at_the_properties_minimum_width() {
    let mut harness = app();
    set_column_width(&mut harness, PROPERTIES_MIN_WIDTH);
    let at_minimum = properties_header_width(&harness);

    set_column_width(&mut harness, WIDE_COLUMN);
    let before = properties_title(&harness).min.x;

    assert!(properties_header_width(&harness) > at_minimum, "a wide column gives Properties more than its minimum");

    let y = properties_title(&harness).max.y + DIVIDER_GRAB_BELOW_HEADER;
    drag(&mut harness, pos2(before - DIVIDER_OFFSET, y), pos2(WINDOW_SIZE.x - WINDOW_RIGHT_INSET, y));

    assert!(properties_title(&harness).min.x > before, "the divider moved right");

    let width = properties_header_width(&harness);
    assert!((width - at_minimum).abs() < 0.5, "Properties stopped at its minimum: header {width}, {at_minimum} at the minimum");
}

#[test]
fn a_layout_saved_narrower_draws_properties_at_its_minimum_width() {
    let mut harness = app();
    set_column_width(&mut harness, PROPERTIES_MIN_WIDTH);
    let at_minimum = properties_header_width(&harness);

    set_column_width(&mut harness, PROPERTIES_MIN_WIDTH * 0.5);
    let width = properties_header_width(&harness);

    assert!((width - at_minimum).abs() < 0.5, "a narrow saved column opens at the minimum: header {width}, {at_minimum} at the minimum");
}

#[test]
fn visiting_and_deleting_layers_does_not_accumulate_scroll_areas() {
    let mut harness = app();
    let count = |harness: &Harness<'_, TextureCraftApp>| harness.ctx.data(|data| data.count::<egui::scroll_area::State>());
    let initial = count(&harness);

    let mut layers = Vec::new();
    for index in 0..12 {
        let session = &mut harness.state_mut().session;
        let layer = texturecraft_engine::execute(session, "layer.add.group", &serde_json::json!({"name": format!("Folder {index}")})).unwrap()["layer"]
            .as_u64()
            .unwrap();
        texturecraft_engine::execute(session, "layer.select", &serde_json::json!({"layer": layer})).unwrap();
        harness.run_steps(1);
        layers.push(layer);
    }
    let after_visiting = count(&harness);

    let session = &mut harness.state_mut().session;
    texturecraft_engine::execute(session, "layer.remove", &serde_json::json!({"layers": layers})).unwrap();
    harness.run_steps(SETTLE_FRAMES);

    let after_removal = count(&harness);
    eprintln!("scroll states after visiting {after_visiting}, after removal {after_removal}, initially {initial}");
    assert!(
        after_visiting <= initial && after_removal <= initial,
        "scroll areas after visiting {after_visiting}, after removal {after_removal}, initially {initial}"
    );
}
