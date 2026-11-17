//! Shift+click in the 3D view paints a straight line from the model point where the last stroke ended, also after the camera moved

use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use serde_json::json;
use texturecraft_engine::Tool;
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::state::ViewLayout;

/// How far the camera turns between the stroke and the Shift+click (radians)
const ORBIT: f32 = 0.5;

/// Alpha a texel needs to count as painted
const PAINTED_ALPHA: f32 = 0.05;

/// Search step and reach when looking for a pixel that shows the model (viewport pixels)
const SEARCH_STEP: f32 = 6.0;
const SEARCH_REACH: f32 = 300.0;

/// The sample project with only the 3D view and the Paint tool
fn view_3d_app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.layout = ViewLayout::Only3D;
    harness.state_mut().session.tool.tool = Tool::Paint;
    harness.run_steps(4);
    harness
}

/// The screen position, in the window, of the model pixel nearest `near` (viewport pixels from the view's corner)
fn model_pixel_near(harness: &Harness<'_, TextureCraftApp>, near: Vec2) -> Pos2 {
    let app = harness.state();
    let steps = (SEARCH_REACH / SEARCH_STEP) as i32;
    let mut candidates: Vec<Vec2> = (-steps..=steps).flat_map(|x| (-steps..=steps).map(move |y| near + Vec2::new(x as f32, y as f32) * SEARCH_STEP)).collect();
    candidates.sort_by(|a, b| a.distance(near).total_cmp(&b.distance(near)));
    let hit = candidates.into_iter().find(|point| app.session.pick(*point, app.session.viewport).is_some()).expect("the model is in view");
    pos2(app.viewport_rect.left() + hit.x, app.viewport_rect.top() + hit.y)
}

/// A primary click at `pos`, with Shift held when `shift` is set
fn click(harness: &mut Harness<'_, TextureCraftApp>, pos: Pos2, shift: bool) {
    let modifiers = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
    harness.event(Event::ModifiersChanged(modifiers));
    harness.hover_at(pos);
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers });
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers });
    harness.run_steps(2);
    harness.event(Event::ModifiersChanged(Modifiers::NONE));
    harness.run_steps(1);
}

/// The selected layer's base color alpha, row by row, with the texture width
fn alpha(harness: &Harness<'_, TextureCraftApp>) -> (Vec<f32>, u32) {
    let session = &harness.state().session;
    let (width, height) = session.doc.project.texture_sets[session.ts()].size();
    let layer = session.selected_layer().expect("a selected layer");
    let Some(image) = session.doc.pixels.layer(layer, Channel::BaseColor) else { return (vec![0.0; (width * height) as usize], width) };
    let texels = (0..height).flat_map(|y| (0..width).map(move |x| (x, y)));
    (texels.map(|(x, y)| image.get(x, y)[3] as f32 / 65535.0).collect(), width)
}

/// The alpha-weighted centre of `paint`, in texels
fn centre(paint: &[f32], width: u32) -> Vec2 {
    let (sum, total) = paint.iter().enumerate().fold((Vec2::ZERO, 0.0), |(sum, total), (index, value)| {
        let texel = Vec2::new((index as u32 % width) as f32, (index as u32 / width) as f32);
        (sum + texel * *value, total + value)
    });
    assert!(total > 0.0, "something was painted");
    sum / total
}

/// The stored line anchor
fn anchor(harness: &Harness<'_, TextureCraftApp>) -> Option<texturecraft_ui_egui::uv_paint_point::SurfacePaintPoint> {
    harness.state().last_paint_point
}

/// A fresh app and a click on the model; returns it with the click position and the dab's centre in texels
fn after_first_click() -> (Harness<'static, TextureCraftApp>, Pos2, Vec2) {
    let mut harness = view_3d_app();
    let first = model_pixel_near(&harness, harness.state().session.viewport * 0.5);
    let (blank, width) = alpha(&harness);
    assert!(blank.iter().all(|alpha| *alpha == 0.0), "the layer starts empty");
    let app = harness.state();
    let local = Vec2::new(first.x - app.viewport_rect.left(), first.y - app.viewport_rect.top());
    assert_eq!(app.session.texture_set_at(local, app.session.viewport), Some(app.session.ts()), "the click lands on the active texture set");
    click(&mut harness, first, false);
    let (dab, _) = alpha(&harness);
    let dab_centre = centre(&dab, width);
    (harness, first, dab_centre)
}

#[test]
fn the_line_starts_at_the_old_model_point_after_orbiting() {
    let (mut harness, _, dab_centre) = after_first_click();
    harness.state_mut().session.undo().unwrap();
    let (blank, width) = alpha(&harness);
    assert!(blank.iter().all(|alpha| *alpha == 0.0), "undo removed the first dab");

    harness.state_mut().session.camera.orbit(ORBIT, 0.0);
    harness.run_steps(2);
    let end = model_pixel_near(&harness, harness.state().session.viewport * 0.5 + Vec2::new(40.0, 20.0));
    click(&mut harness, end, true);

    let (line, _) = alpha(&harness);
    let index = dab_centre.y.round() as usize * width as usize + dab_centre.x.round() as usize;
    assert!(line[index] > PAINTED_ALPHA, "the line is painted at the old model point {dab_centre}");
}

#[test]
fn the_anchor_is_the_model_point_of_the_stroke_end() {
    let (harness, first, _) = after_first_click();
    let app = harness.state();
    let local = Vec2::new(first.x - app.viewport_rect.left(), first.y - app.viewport_rect.top());
    let expected = app.session.pick(local, app.session.viewport).unwrap().pos;
    let stored = anchor(&harness).and_then(|point| point.position_for(&app.session)).expect("an anchor after a stroke");
    assert!(stored.distance(expected) < 1e-4, "{stored} vs {expected}");
}

#[test]
fn switching_the_texture_set_drops_the_anchor() {
    let (mut harness, _, _) = after_first_click();
    harness.state_mut().run("textureSet.select", json!({"ts": 1}));
    harness.run_steps(2);
    let session = &harness.state().session;
    assert!(anchor(&harness).and_then(|point| point.position_for(session)).is_none());
}

#[test]
fn opening_another_project_drops_the_anchor() {
    let (mut harness, _, _) = after_first_click();
    harness.state_mut().run("project.new", json!({"mesh": "sphere", "resolution": 256})).unwrap();
    harness.run_steps(2);
    let session = &harness.state().session;
    assert!(anchor(&harness).and_then(|point| point.position_for(session)).is_none());
}
