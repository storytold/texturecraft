//! The History panel: a click on a step goes back or forward to it, the current step is marked and kept in view

use egui::accesskit::Toggled;
use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::geom::glam::Vec3;
use texturecraft_engine::{Session, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::Panel;

/// Show document history so row clicks exercise the same command dispatch as the application
fn history_harness(session: Session) -> Harness<'static, TextureCraftApp> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.state_mut().ui.dock.focus(Panel::History);
    harness.run_steps(4);
    harness
}

/// Undo or redo one step from the keyboard, as the Edit menu's shortcuts do
fn press(harness: &mut Harness<'_, TextureCraftApp>, key: Key) {
    harness.key_press_modifiers(Modifiers::COMMAND, key);
    harness.run_steps(3);
}

/// The History rows shown as the current step: name and top edge
fn current_rows(harness: &Harness<'_, TextureCraftApp>) -> Vec<(String, f32)> {
    let session = &harness.state().session;
    let undone = session.redo_labels().into_iter().map(|label| format!("{label} (undone)"));
    let names: Vec<String> = session.history_labels().into_iter().chain(undone).chain(["Project opened".to_owned()]).collect();
    harness
        .get_all_by_role(egui::accesskit::Role::Button)
        .filter(|node| node.accesskit_node().toggled() == Some(Toggled::True))
        .filter_map(|node| Some((node.accesskit_node().label()?, node.rect().top())))
        .filter(|(name, _)| names.contains(name))
        .collect()
}

/// Three document rows provide an intermediate target that detects both short and excessive jumps
fn renamed_session() -> Session {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for name in ["Step A", "Step B", "Step C"] {
        execute(&mut session, "layer.rename", &json!({"name": name})).unwrap();
    }
    session
}

/// Canceling a preview can be its own undo or part of undoing the first placed point
#[test]
fn history_jump_accounts_for_drafts_with_and_without_an_empty_checkpoint() {
    for placed in [false, true] {
        let mut session = renamed_session();
        let layer = session.selected_layer().unwrap();
        let effect_count = session.doc.project.layer(layer).unwrap().content_effects.len();
        if placed {
            execute(&mut session, "tool.select", &json!({"tool": "path"})).unwrap();
            for point in [Vec3::new(0.2, 0.5, 0.0), Vec3::new(0.5, 0.3, 0.0), Vec3::new(0.8, 0.5, 0.0)] {
                session.append_path_point(point, true);
                session.path_live().unwrap();
            }
        } else {
            execute(&mut session, "tool.path", &json!({"uvPoints": [[0.2, 0.5], [0.8, 0.5]]})).unwrap();
            session.move_path_point(0, Vec3::new(0.2, 0.3, 0.0), true);
            session.path_live().unwrap();
        }
        session.path_checkpoint();
        let mut harness = history_harness(session);

        // The list runs oldest first, so the first "Rename layer" row is the oldest step
        harness.get_all_by_label("Rename layer").next().unwrap().click();
        harness.run_steps(3);

        assert_eq!(harness.state().session.doc.project.layer(layer).unwrap().name, "Step A");
        assert_eq!(harness.state().session.history_labels().len(), 1);
        assert_eq!(harness.state().session.redo_labels().len(), 2);
        assert!(harness.state().session.tool.path.points.is_empty());
        assert_eq!(harness.state().session.doc.project.layer(layer).unwrap().content_effects.len(), effect_count);
    }
}

/// A row jump traverses point history, while Ctrl+Z and Ctrl+Y still take one point step
#[test]
fn history_jump_accounts_for_stored_point_edits_without_changing_single_steps() {
    let mut session = renamed_session();
    let layer = session.selected_layer().unwrap();
    let report = execute(&mut session, "tool.path", &json!({"uvPoints": [[0.2, 0.5], [0.8, 0.5]], "commit": true})).unwrap();
    execute(&mut session, "path.edit", &json!({"effect": report["committed"]["effect"], "index": 0})).unwrap();
    let original = session.tool.path.points.clone();
    session.move_path_point(0, Vec3::new(0.2, 0.3, 0.0), true);
    session.path_live().unwrap();
    let moved = session.tool.path.points.clone();
    session.move_path_point(0, original[0], true);
    session.path_live().unwrap();
    session.path_checkpoint();
    let done = session.history_labels().len();
    let mut harness = history_harness(session);

    press(&mut harness, Key::Z);
    assert_eq!(harness.state().session.tool.path.points, moved);
    assert_eq!(harness.state().session.history_labels().len(), done);
    press(&mut harness, Key::Y);
    assert_eq!(harness.state().session.tool.path.points, original);
    assert_eq!(harness.state().session.history_labels().len(), done);

    harness.get_by_label("Edit path 1").click();
    harness.run_steps(3);
    assert_eq!(harness.state().session.tool.path.points, original);
    assert_eq!(harness.state().session.history_labels().len(), done);

    // The list runs oldest first, so the first "Rename layer" row is the oldest step
    harness.get_all_by_label("Rename layer").next().unwrap().click();
    harness.run_steps(3);

    assert_eq!(harness.state().session.doc.project.layer(layer).unwrap().name, "Step A");
    assert_eq!(harness.state().session.history_labels().len(), 1);
    assert_eq!(harness.state().session.redo_labels().len(), done - 1);
    assert!(harness.state().session.tool.path.points.is_empty());
}

/// Incomplete filled curves keep point redo separate from the document redo rows
#[test]
fn history_redo_jump_accounts_for_point_steps_without_changing_single_steps() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = session.selected_layer().unwrap();
    let report =
        execute(&mut session, "tool.path", &json!({"uvPoints": [[0.2, 0.5], [0.5, 0.3], [0.8, 0.5]], "closed": true, "fill": true, "commit": true})).unwrap();
    for name in ["Step A", "Step B", "Step C"] {
        execute(&mut session, "layer.rename", &json!({"name": name})).unwrap();
    }
    let done = session.history_labels().len();
    execute(&mut session, "edit.undo", &json!({"steps": 3})).unwrap();
    execute(&mut session, "path.edit", &json!({"effect": report["committed"]["effect"], "index": 0})).unwrap();
    let original = session.tool.path.points.clone();

    // Partial filled shapes do not repaint; their undo/redo leaves document redo available
    for points in [json!([[0.2, 0.5], [0.8, 0.5]]), json!([[0.2, 0.5]]), json!([[0.2, 0.5], [0.8, 0.5]])] {
        session.path_checkpoint();
        execute(&mut session, "tool.path", &json!({"uvPoints": points})).unwrap();
    }
    for _ in 0..3 {
        session.undo().unwrap();
    }
    assert_eq!(session.redo_labels().len(), 3);
    let mut harness = history_harness(session);

    press(&mut harness, Key::Y);
    assert_eq!(harness.state().session.tool.path.points.len(), 2);
    assert_eq!(harness.state().session.history_labels().len(), done - 3);
    press(&mut harness, Key::Z);
    assert_eq!(harness.state().session.tool.path.points, original);
    assert_eq!(harness.state().session.history_labels().len(), done - 3);

    // The last undone row is the newest step, so clicking it redoes up to the end
    harness.get_all_by_label("Rename layer (undone)").last().unwrap().click();
    harness.run_steps(3);

    assert_eq!(harness.state().session.doc.project.layer(layer).unwrap().name, "Step C");
    assert_eq!(harness.state().session.history_labels().len(), done);
    assert!(harness.state().session.redo_labels().is_empty());
    assert!(harness.state().session.tool.path.points.is_empty());
}

/// Returning to the current document row cancels a draft without undoing a document step
#[test]
fn clicking_the_current_history_row_cancels_a_draft() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = session.selected_layer().unwrap();
    let effect_count = session.doc.project.layer(layer).unwrap().content_effects.len();
    execute(&mut session, "tool.path", &json!({"uvPoints": [[0.2, 0.5], [0.8, 0.5]]})).unwrap();
    let mut harness = history_harness(session);

    harness.get_by_label("Project opened").click();
    harness.run_steps(3);

    assert!(harness.state().session.tool.path.points.is_empty());
    assert!(harness.state().session.history_labels().is_empty());
    assert!(harness.state().session.redo_labels().is_empty());
    assert_eq!(harness.state().session.doc.project.layer(layer).unwrap().content_effects.len(), effect_count);
}

#[test]
fn clicking_a_history_step_returns_to_it() {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    let layer = session.selected_layer().unwrap();
    for name in ["Step A", "Step B", "Step C"] {
        texturecraft_engine::execute(&mut session, "layer.rename", &json!({"layer": layer.0, "name": name})).unwrap();
    }
    let done = session.history_labels().len();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.dock.focus(Panel::History);
    h.run_steps(4);
    let name = |h: &Harness<'_, TextureCraftApp>| h.state().session.doc.project.layers[&layer].name.clone();
    // The first state: everything undone, and listed as undone.
    h.get_by_label("Project opened").click();
    h.run_steps(3);
    assert!(!h.state().session.can_undo());
    assert_eq!(h.state().session.redo_labels().len(), done);
    // The first undone row is the next step: one redo.
    h.get_all_by_label("Rename layer (undone)").next().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().session.history_labels().len(), 1);
    assert_eq!(h.state().session.redo_labels().len(), done - 1);
    // The last undone row: all the way forward again.
    h.get_all_by_label("Rename layer (undone)").last().unwrap().click();
    h.run_steps(3);
    assert_eq!(name(&h), "Step C");
    assert!(!h.state().session.can_redo());
}

/// The panel has no Undo and Redo buttons; exactly one row reads as the current step, it moves with Ctrl+Z, and undone rows stay clickable
#[test]
fn only_the_current_step_is_marked_selected() {
    let mut harness = history_harness(renamed_session());

    // Undo and redo live in the Edit menu and on their shortcuts, not in a button row under the list
    assert!(harness.query_by_label(&format!("{} Undo", egui_phosphor::regular::ARROW_COUNTER_CLOCKWISE)).is_none());
    assert!(harness.query_by_label(&format!("{} Redo", egui_phosphor::regular::ARROW_CLOCKWISE)).is_none());

    let before = current_rows(&harness);
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].0, "Rename layer");

    press(&mut harness, Key::Z);
    let after = current_rows(&harness);
    assert_eq!(after.len(), 1);
    assert!(after[0].1 < before[0].1, "the outline moves up a row");

    harness.get_by_label("Rename layer (undone)").click();
    harness.run_steps(3);
    assert!(harness.state().session.redo_labels().is_empty());
}

/// Enough steps that the History cannot show them all at once
const STEPS_PAST_THE_PANEL: usize = 100;

/// A session `steps` renames long, so its History list is taller than the panel
fn long_session(steps: usize) -> Session {
    let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
    for step in 0..steps {
        execute(&mut session, "layer.rename", &json!({"name": format!("Step {step}")})).unwrap();
    }
    session
}

/// Whether the current step's row named `name` is on screen
fn current_shown(harness: &Harness<'_, TextureCraftApp>, name: &str) -> bool {
    let screen = harness.ctx.content_rect();
    current_rows(harness).iter().any(|(row, top)| row == name && screen.y_range().contains(*top))
}

/// Scroll the History list back to its first rows with the mouse wheel
fn scroll_to_top(harness: &mut Harness<'_, TextureCraftApp>) {
    // Exactly one row is the current step; its top finds the row under the wheel
    let (_, top) = current_rows(harness)[0];
    let over = harness.get_all_by_label("Rename layer").find(|node| node.rect().top() == top).unwrap().rect().center();
    harness.hover_at(over);
    harness.event(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, 100_000.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: Modifiers::NONE,
    });
    harness.run_steps(12);
}

/// A History list longer than the panel opens at its newest step, not at "Project opened"
#[test]
fn a_long_history_shows_the_newest_step() {
    let harness = history_harness(long_session(STEPS_PAST_THE_PANEL));

    assert!(current_shown(&harness, "Rename layer"));
    assert!(harness.query_by_label("Project opened").is_none_or(|row| !harness.ctx.content_rect().y_range().contains(row.rect().top())));
}

/// A list the user scrolled up stays there when a tool changes but the history doesn't
#[test]
fn scrolling_up_is_not_pulled_back() {
    let mut harness = history_harness(long_session(STEPS_PAST_THE_PANEL));
    scroll_to_top(&mut harness);
    assert!(!current_shown(&harness, "Rename layer"));

    execute(&mut harness.state_mut().session, "tool.select", &json!({"tool": "path"})).unwrap();
    harness.run_steps(4);

    assert!(!current_shown(&harness, "Rename layer"));
}

/// Once the history is full, a new step keeps the step count; its new name still brings it into view
#[test]
fn the_newest_step_shows_past_the_history_cap() {
    let steps = texturecraft_engine::session::MAX_HISTORY + 60;
    let mut harness = history_harness(long_session(steps));
    let capped = harness.state().session.history_labels().len();
    assert!(capped < steps, "the history is full");
    scroll_to_top(&mut harness);
    assert!(!current_shown(&harness, "Rename layer"));

    execute(&mut harness.state_mut().session, "layer.add.paint", &json!({})).unwrap();
    harness.run_steps(4);

    let newest = harness.state().session.history_labels().last().cloned().unwrap();
    assert_eq!(harness.state().session.history_labels().len(), capped);
    assert!(current_shown(&harness, &newest));
}

/// A History list that moves to another container (rail popup to floating window) still shows the current step
#[test]
fn an_undocked_history_shows_the_newest_step() {
    let session = long_session(STEPS_PAST_THE_PANEL);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|context| TextureCraftApp::new(context, session));
    harness.run_steps(4);
    let icon = harness.get_all_by_role(egui::accesskit::Role::Button).find(|button| {
        let label = button.accesskit_node().label().unwrap_or_default();
        label == "History" || label.starts_with("History (")
    });
    icon.expect("the History icon on the rail").click();
    harness.run_steps(4);
    assert!(current_shown(&harness, "Rename layer"));

    harness.get_by_label("Undock").click();
    harness.run_steps(4);

    assert!(current_shown(&harness, "Rename layer"));
}

/// Shift+Tab walks from the current step up to the first row, past the rows on screen when the panel opened
#[test]
fn the_keyboard_reaches_rows_scrolled_out_of_view() {
    let mut harness = history_harness(long_session(STEPS_PAST_THE_PANEL));
    let (_, top) = current_rows(&harness)[0];
    harness.get_all_by_label("Rename layer").find(|node| node.rect().top() == top).unwrap().focus();
    harness.run_steps(2);

    let first_row_focused = |harness: &Harness<'_, TextureCraftApp>| {
        let screen = harness.ctx.content_rect();
        harness.query_by_label("Project opened").is_some_and(|row| row.accesskit_node().is_focused() && screen.y_range().contains(row.rect().top()))
    };
    // One press per step is enough to reach the first row
    for _ in 0..harness.state().session.history_labels().len() + 1 {
        if first_row_focused(&harness) {
            break;
        }
        harness.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
        harness.run_steps(2);
    }

    assert!(first_row_focused(&harness));
}
