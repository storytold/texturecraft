//! The 3D and 2D views as Tab stops: they keep the focus Tab gives them, Tab on one hides and shows the UI, Shift+Tab
//! moves on from it, and the keys of the viewport work as with nothing focused.

use craft_ui::runtime::focus::{FocusTarget, request};
use egui::{Event, Id, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use serde_json::json;
use texturecraft_engine::{Session, Tool, execute};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;
use texturecraft_ui_egui::viewport::{view_2d_id, view_3d_id};

type AppHarness = Harness<'static, TextureCraftApp>;

fn harness(session: Session, layout: ViewLayout) -> AppHarness {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.state_mut().ui.layout = layout;
    h.run_steps(4);
    h
}

fn sample() -> Session {
    texturecraft_engine::sample_session(256, 64).unwrap()
}

fn focused(h: &AppHarness) -> Option<Id> {
    h.ctx.memory(|memory| memory.focused())
}

/// Focus `view` as Tab does
fn focus(h: &mut AppHarness, view: Id) {
    request(&h.ctx, FocusTarget::Id(view));
    h.run_steps(2);
    assert_eq!(focused(h), Some(view));
}

fn press(h: &mut AppHarness, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(3);
}

fn hidden(h: &AppHarness) -> bool {
    h.state().ui.hide_ui
}

/// The views each layout draws
const DRAWN: [(ViewLayout, bool, bool); 3] = [(ViewLayout::Only3D, true, false), (ViewLayout::Only2D, false, true), (ViewLayout::Split, true, true)];

#[test]
fn shift_tab_walks_over_the_views_and_tab_on_one_toggles_the_ui_once() {
    let mut h = harness(sample(), ViewLayout::Split);
    let mut walked = Vec::new();
    while focused(&h) != Some(view_3d_id()) {
        press(&mut h, Modifiers::SHIFT, Key::Tab);
        assert!(focused(&h).is_some(), "Shift+Tab {} keeps a focus", walked.len());
        walked.push(focused(&h).unwrap());
        assert!(walked.len() < 400, "Shift+Tab reaches the 3D view");
    }
    assert!(walked.iter().position(|stop| *stop == view_2d_id()).is_some_and(|at| at > 0), "it passes the stops after the views, then the 2D view");

    press(&mut h, Modifiers::SHIFT, Key::Tab);
    let before = focused(&h).expect("Shift+Tab moves on from the view");
    assert!(!walked.contains(&before));
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert_eq!(focused(&h), Some(view_3d_id()), "Tab from the stop before the view stops at it");
    assert!(!hidden(&h));

    press(&mut h, Modifiers::NONE, Key::Tab);
    assert!(hidden(&h), "Tab on the focused view hides the UI");
    assert_eq!(focused(&h), Some(view_3d_id()), "which keeps the focus");
    press(&mut h, Modifiers::NONE, Key::Tab);
    assert!(!hidden(&h), "the next Tab shows it");
    press(&mut h, Modifiers::SHIFT, Key::Tab);
    assert_eq!(focused(&h), Some(before), "Shift+Tab leaves the view backward");
}

#[test]
fn every_drawn_view_holds_the_focus_and_toggles_the_ui_with_and_without_a_project() {
    for (layout, in_3d, in_2d) in DRAWN {
        for project in [true, false] {
            for view in [view_3d_id(), view_2d_id()].into_iter().zip([in_3d, in_2d]).filter_map(|(view, drawn)| drawn.then_some(view)) {
                let mut h = harness(if project { sample() } else { Session::closed().unwrap() }, layout);
                for focus_view in [false, true] {
                    if focus_view {
                        focus(&mut h, view);
                    }
                    for shown_next in [false, true] {
                        press(&mut h, Modifiers::NONE, Key::Tab);
                        assert_eq!(hidden(&h), !shown_next, "project {project}, focused {focus_view}");
                        assert_eq!(focused(&h), focus_view.then_some(view), "project {project}");
                    }
                }
            }
        }
    }
}

#[test]
fn a_click_in_either_view_leaves_tab_to_the_ui_toggle() {
    for (clicked, with_focus) in [(view_3d_id(), view_3d_id()), (view_2d_id(), view_2d_id()), (view_2d_id(), view_3d_id())] {
        let mut h = harness(sample(), ViewLayout::Split);
        focus(&mut h, with_focus);
        let at = h.ctx.read_response(clicked).unwrap().rect.center();
        for pressed in [true, false] {
            h.hover_at(at);
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(3);
        }
        press(&mut h, Modifiers::NONE, Key::Tab);
        assert!(hidden(&h));
        press(&mut h, Modifiers::NONE, Key::Tab);
        assert!(!hidden(&h));
    }
}

#[test]
fn shortcuts_run_with_a_view_focused() {
    for view in [view_3d_id(), view_2d_id()] {
        let mut h = harness(sample(), ViewLayout::Split);
        let name = h.state().session.doc.project.layers[&h.state().session.selected_layer().unwrap()].name.clone();
        let row = h.get_by_label(&name).rect().center();
        focus(&mut h, view);
        let layers = |h: &AppHarness| h.state().session.doc.project.layers.len();
        let before = layers(&h);
        h.event(Event::Copy);
        h.run_steps(3);
        assert!(h.state().session.clipboard.is_some(), "Ctrl+C copies the layer");
        h.event(Event::Paste("Layer".into()));
        h.run_steps(3);
        assert!(layers(&h) > before, "Ctrl+V pastes it");
        press(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(layers(&h), before, "Ctrl+Z takes it back");

        let size = h.state().session.tool.brush.size;
        press(&mut h, Modifiers::NONE, Key::CloseBracket);
        assert!(h.state().session.tool.brush.size > size, "] grows the brush");

        h.hover_at(row);
        h.run_steps(1);
        press(&mut h, Modifiers::COMMAND, Key::A);
        let session = &h.state().session;
        let every_row = texturecraft_engine::layer_selection::stack_rows(&session.doc.project, 0, true);
        assert_eq!(session.selected_layers().len(), every_row.len(), "Ctrl+A over the Layers panel selects every row");
        assert_eq!(focused(&h), Some(view));
    }
}

#[test]
fn the_path_keys_work_with_the_view_focused() {
    let mut session = sample();
    execute(&mut session, "layer.add.paint", &json!(null)).unwrap();
    execute(&mut session, "tool.select", &json!({"tool": "path"})).unwrap();
    let mut h = harness(session, ViewLayout::Only2D);
    let place = |h: &mut AppHarness| {
        execute(&mut h.state_mut().session, "tool.path", &json!({"uvPoints": [[0.2, 0.5], [0.5, 0.5], [0.8, 0.5]], "commit": false})).unwrap();
        h.run_steps(3);
    };
    let points = |h: &AppHarness| h.state().session.tool.path.points.len();
    focus(&mut h, view_2d_id());

    place(&mut h);
    press(&mut h, Modifiers::NONE, Key::Backspace);
    assert_eq!(points(&h), 2, "Backspace removes one point");
    let (effect, index) = h.state().session.tool.path.editing.unwrap();
    press(&mut h, Modifiers::NONE, Key::Enter);
    assert!(points(&h) == 0 && h.state().session.paths(effect).is_some_and(|paths| paths.get(index).is_some()), "Enter applies the path");

    place(&mut h);
    assert_eq!(points(&h), 3);
    press(&mut h, Modifiers::NONE, Key::Escape);
    assert_eq!(points(&h), 0, "Escape cancels the next one");
    assert_eq!(focused(&h), Some(view_2d_id()));
}

#[test]
fn held_keys_work_with_the_view_focused() {
    let mut session = sample();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let image = session.import_resource_bytes("logo.png", png).unwrap();
    execute(&mut session, "tool.stencil", &json!({"image": image})).unwrap();
    session.tool.tool = Tool::Projection;
    let mut h = harness(session, ViewLayout::Only3D);
    focus(&mut h, view_3d_id());

    let scale = |h: &AppHarness| h.state().session.tool.stencil.as_ref().unwrap().scale;
    let (before, yaw) = (scale(&h), h.state().session.camera.yaw);
    hold_and_right_drag(&mut h, Key::S);
    assert_ne!(scale(&h), before, "S + right drag scales the stencil");
    assert_eq!(h.state().session.camera.yaw, yaw, "instead of turning the camera");
    assert_eq!(focused(&h), Some(view_3d_id()));
}

fn hold_and_right_drag(h: &mut AppHarness, key: Key) {
    let from = h.state().viewport_rect.center();
    let button = |h: &mut AppHarness, pos: Pos2, pressed| {
        h.event(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
    };
    h.event(Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.hover_at(from);
    button(h, from, true);
    h.run_steps(2);
    for step in 1..=4 {
        h.hover_at(from + egui::vec2(15.0 * step as f32, 0.0));
        h.run_steps(1);
    }
    button(h, from + egui::vec2(60.0, 0.0), false);
    h.event(Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn escape_and_enter_answer_a_question_with_a_view_focused() {
    for (key, reset) in [(Key::Escape, false), (Key::Enter, true)] {
        let mut h = harness(sample(), ViewLayout::Only3D);
        h.state_mut().ui.windows.confirm_reset_ui = true;
        h.run_steps(3);
        focus(&mut h, view_3d_id());
        press(&mut h, Modifiers::NONE, key);
        assert!(!h.state().ui.windows.confirm_reset_ui, "{key:?} answers the question");
        assert_eq!(h.state().ui.layout == ViewLayout::Split, reset, "only Enter resets the layout");
    }
}
