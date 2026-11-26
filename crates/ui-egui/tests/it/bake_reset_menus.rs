//! The Cage and Skew Correction ⟲ menus: reset the parameters, clear the paint strokes, or both

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use texturecraft_engine::core::{BakePaintTarget, BakeSettings};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;
use texturecraft_ui_egui::state::Mode;

/// Bake mode with a high poly, Common Settings in front, a painted stroke on `target` and its section's settings changed
fn harness(target: BakePaintTarget) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    session.add_high_poly_bytes("High.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec()).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 1500.0)).build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.mode = Mode::Baking;
    harness.run_steps(4);
    harness.state_mut().ui.dock.focus(Panel::CommonSettings);

    let command = match target {
        BakePaintTarget::Cage => "bake.cage.paint",
        BakePaintTarget::Skew => "bake.skew.paint",
    };
    harness.state_mut().run(command, json!({ "points": [[0.5, 0.5]] }));
    let settings = &mut harness.state_mut().session.doc.project.texture_sets[0].bake;
    (settings.cage.max_frontal, settings.skew.edge_distance) = (CHANGED_CAGE_DISTANCE, CHANGED_EDGE_DISTANCE);
    harness.run_steps(3);
    harness
}

/// Values other than the defaults (0.01 and 0.15), inside their sliders' ranges
const CHANGED_CAGE_DISTANCE: f32 = 0.03;
const CHANGED_EDGE_DISTANCE: f32 = 0.5;

/// Open `section`'s ⟲ menu and pick `item`
fn pick(harness: &mut Harness<'static, TextureCraftApp>, section: &str, item: &str) {
    // The Baking Visualization panel has its own Cage section in the 3D view, right of Common Settings
    let label = format!("Reset {section}");
    let buttons = harness.get_all_by_role_and_label(Role::Button, &label);
    buttons.min_by(|first, second| first.rect().left().total_cmp(&second.rect().left())).unwrap().click();
    harness.run_steps(3);
    harness.get_by_label(item).click();
    harness.run_steps(3);
}

/// Whether the first set still has its `target` strokes, and whether its section's parameters are the defaults
fn state(harness: &Harness<'static, TextureCraftApp>, target: BakePaintTarget) -> (bool, bool) {
    let session = &harness.state().session;
    let settings = &session.doc.project.texture_sets[0].bake;
    let defaults = BakeSettings::default();
    let painted = session.doc.bake_paint.contains_key(&(0, target));
    let reset = match target {
        BakePaintTarget::Cage => settings.cage == defaults.cage,
        BakePaintTarget::Skew => settings.skew == defaults.skew,
    };
    (painted, reset)
}

#[test]
fn each_cage_reset_item_resets_the_parameters_clears_the_strokes_or_both() {
    let items = [("Reset Cage Parameters", (true, true)), ("Clear All Cage Paint Strokes", (false, false)), ("Reset and Clear All", (false, true))];
    for (item, expected) in items {
        let mut harness = harness(BakePaintTarget::Cage);
        assert_eq!(state(&harness, BakePaintTarget::Cage), (true, false), "painted, parameters changed");

        pick(&mut harness, "Cage", item);

        assert_eq!(state(&harness, BakePaintTarget::Cage), expected, "{item}: (strokes kept, parameters reset)");
    }
}

#[test]
fn each_skew_reset_item_resets_the_parameters_clears_the_strokes_or_both() {
    let items = [("Reset Skew Correction Parameters", (true, true)), ("Clear All Skew Paint Strokes", (false, false)), ("Reset and Clear All", (false, true))];
    for (item, expected) in items {
        let mut harness = harness(BakePaintTarget::Skew);
        assert_eq!(state(&harness, BakePaintTarget::Skew), (true, false), "painted, parameters changed");

        pick(&mut harness, "Skew Correction", item);

        assert_eq!(state(&harness, BakePaintTarget::Skew), expected, "{item}: (strokes kept, parameters reset)");
    }
}
