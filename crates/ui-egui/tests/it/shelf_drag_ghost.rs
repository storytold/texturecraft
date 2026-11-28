//! A dragged asset or layer shows at the pointer with a badge saying whether the place under it takes it

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::materials::MATERIALS;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::shelf_drag_ghost::{ShelfDragVerdict, shown};

/// Distance between the view points searched for a set or the background
const SEARCH_STEP: f32 = 8.0;

/// How far the pointer moves off the tile before it counts as a drag
const DRAG_START: f32 = 12.0;

/// The sample project at 1600 x 960, the shelf on Materials
fn sample_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    harness
}

/// The first point of the 3D view showing texture set `set`, or the background when `None`
fn point_over(harness: &Harness<'_, TextureCraftApp>, set: Option<usize>) -> Pos2 {
    let app = harness.state();
    let rect = app.viewport_rect;
    let steps = |length: f32| (1..(length / SEARCH_STEP) as usize).map(|step| step as f32 * SEARCH_STEP);

    steps(rect.height())
        .flat_map(|y| steps(rect.width()).map(move |x| (x, y)))
        .find(|&(x, y)| app.session.texture_set_at(Vec2::new(x, y), app.session.viewport) == set)
        .map(|(x, y)| rect.min + egui::vec2(x, y))
        .unwrap()
}

#[test]
fn the_badge_says_whether_the_place_under_the_pointer_takes_the_asset_without_covering_the_pointer() {
    // The grid's first plain material (by name), so its tile is in view
    let material = MATERIALS.iter().filter(|material| !material.smart).min_by_key(|material| material.label.to_lowercase()).unwrap();
    let mut harness = sample_app();
    let (set, background) = (point_over(&harness, Some(0)), point_over(&harness, None));
    let tile = harness.get_all_by_label(material.label).next().unwrap().rect().center();
    assert!(shown(&harness.ctx).is_none(), "nothing follows the pointer before a drag");

    harness.event(Event::PointerButton { pos: tile, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    harness.hover_at(tile + vec2(DRAG_START, DRAG_START));
    harness.run_steps(2);

    let mut seen = Vec::new();
    for target in [set, background] {
        harness.hover_at(target);
        harness.run_steps(2);
        let ghost = shown(&harness.ctx).expect("the dragged asset follows the pointer");
        assert!(ghost.badge.min.x > target.x && ghost.badge.min.y > target.y, "the badge leaves the pointer's tip clear");
        seen.push(ghost.verdict);
    }
    assert_eq!(seen, [ShelfDragVerdict::Valid, ShelfDragVerdict::NotValid], "a texture set takes a material, the background doesn't");

    harness.event(Event::PointerButton { pos: background, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);

    assert!(shown(&harness.ctx).is_none(), "the ghost goes when the drag ends");
}

#[test]
fn a_layer_dragged_in_the_layer_stack_gets_the_same_ghost_and_badge() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for name in ["Ghost low", "Ghost high"] {
        texturecraft_engine::execute(&mut session, "layer.add.paint", &serde_json::json!({ "name": name })).unwrap();
    }
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 1200.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(4);
    let background = point_over(&harness, None);
    let (high, low) = (harness.get_by_label("Ghost high").rect().center(), harness.get_by_label("Ghost low").rect().center());

    harness.event(Event::PointerButton { pos: high, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(2);
    harness.hover_at(high + vec2(DRAG_START, DRAG_START));
    harness.run_steps(2);

    let mut seen = Vec::new();
    for target in [low, background] {
        harness.hover_at(target);
        harness.run_steps(2);
        seen.push(shown(&harness.ctx).expect("the dragged layer follows the pointer").verdict);
    }
    assert_eq!(seen, [ShelfDragVerdict::Valid, ShelfDragVerdict::NotValid], "another row takes the layer, the 3D view doesn't");

    harness.event(Event::PointerButton { pos: background, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(2);

    assert!(shown(&harness.ctx).is_none(), "the ghost goes when the drag ends");
}
