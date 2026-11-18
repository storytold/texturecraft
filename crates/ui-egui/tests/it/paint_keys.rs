//! The painting toggle keys (L symmetry, D lazy mouse, A resume previous stroke, N ignore stencil; X inverts the mask value),
//! their rebinding, resume continuing from the last stroke end, and the Hide stencil when painting option

use egui::Key;
use egui::{Event, Modifiers, PointerButton, Pos2, pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
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

fn press(harness: &mut Harness<'_, TextureCraftApp>, key: Key) {
    harness.key_press(key);
    harness.run_steps(3);
}

#[test]
fn the_keys_toggle_symmetry_lazy_mouse_resume_and_ignore_stencil() {
    let mut harness = view_3d_app();
    let symmetry = harness.state().session.tool.brush.symmetry.enabled;
    assert_eq!(harness.state().session.tool.brush.lazy, 0.0);

    press(&mut harness, Key::L);
    assert_eq!(harness.state().session.tool.brush.symmetry.enabled, !symmetry);
    press(&mut harness, Key::L);
    assert_eq!(harness.state().session.tool.brush.symmetry.enabled, symmetry);

    press(&mut harness, Key::D);
    assert!(harness.state().session.tool.brush.lazy > 0.0);
    press(&mut harness, Key::D);
    assert_eq!(harness.state().session.tool.brush.lazy, 0.0);

    press(&mut harness, Key::A);
    assert!(harness.state().session.tool.resume_stroke);
    press(&mut harness, Key::A);
    assert!(!harness.state().session.tool.resume_stroke);

    press(&mut harness, Key::N);
    assert!(harness.state().session.tool.ignore_stencil);
    press(&mut harness, Key::N);
    assert!(!harness.state().session.tool.ignore_stencil);
}

#[test]
fn the_keys_are_listed_in_the_shortcut_overview_and_clash_with_nothing() {
    let bindings = texturecraft_ui_egui::keymap::bindings();
    for (id, key) in [("tool.symmetry", "L"), ("tool.lazyMouse", "D"), ("tool.resumeStroke", "A"), ("tool.ignoreStencil", "N"), ("tool.swapMaskValue", "X")] {
        let binding = bindings.iter().find(|binding| binding.id == id).unwrap_or_else(|| panic!("{id} is bindable"));
        assert_eq!(binding.default, Some(key), "{id}");
        let clash = bindings.iter().find(|other| other.id != id && other.default == Some(key));
        assert!(clash.is_none(), "{key} is also the default of {:?}", clash.map(|other| other.id));
    }
}

#[test]
fn a_rebound_toggle_answers_its_new_key_only() {
    let mut harness = view_3d_app();
    harness.state_mut().ui.keymap.insert("tool.symmetry".into(), "K".into());
    let symmetry = harness.state().session.tool.brush.symmetry.enabled;

    press(&mut harness, Key::L);
    assert_eq!(harness.state().session.tool.brush.symmetry.enabled, symmetry, "L no longer toggles it");
    press(&mut harness, Key::K);
    assert_eq!(harness.state().session.tool.brush.symmetry.enabled, !symmetry);
}

/// After a first click, undone and the camera turned: whether a plain click far away paints at the first click's model point
fn plain_click_reaches_the_old_point(resume: bool) -> bool {
    let (mut harness, _, dab_centre) = after_first_click();
    harness.state_mut().session.undo().unwrap();
    harness.state_mut().session.camera.orbit(ORBIT, 0.0);
    harness.state_mut().session.tool.resume_stroke = resume;
    harness.run_steps(2);
    let end = model_pixel_near(&harness, harness.state().session.viewport * 0.5 + Vec2::new(40.0, 20.0));
    click(&mut harness, end, false);
    let (paint, width) = alpha(&harness);
    paint[dab_centre.y.round() as usize * width as usize + dab_centre.x.round() as usize] > PAINTED_ALPHA
}

#[test]
fn resume_paints_the_line_from_the_old_model_point_without_shift() {
    assert!(!plain_click_reaches_the_old_point(false), "a plain click leaves the old point alone");
    assert!(plain_click_reaches_the_old_point(true), "with resume on the line starts at the old model point");
}

#[test]
fn the_hide_stencil_option_is_in_the_projection_tool_and_on_by_default() {
    let mut harness = view_3d_app();
    harness.state_mut().session.tool.tool = Tool::Projection;
    harness.run_steps(3);
    assert!(harness.state().session.tool.hide_stencil_while_painting);

    harness.get_by_label("Hide stencil when painting").click();
    harness.run_steps(3);
    assert!(!harness.state().session.tool.hide_stencil_while_painting);
    harness.get_by_label("Hide stencil when painting").click();
    harness.run_steps(3);
    assert!(harness.state().session.tool.hide_stencil_while_painting);
}

/// The stencil image quads drawn in the last frame: four-corner image meshes
fn stencil_quads(harness: &Harness<'_, TextureCraftApp>) -> usize {
    harness
        .output()
        .shapes
        .iter()
        .filter(|shape| matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.vertices.len() == 4 && mesh.indices.len() == 6 && mesh.texture_id != egui::TextureId::Managed(0)))
        .count()
}

#[test]
fn the_stencil_is_not_drawn_during_a_stroke_unless_the_option_is_off() {
    let mut harness = view_3d_app();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = harness.state_mut().session.import_resource_bytes("logo.png", png).unwrap();
    harness.state_mut().session.tool.tool = Tool::Projection;
    texturecraft_engine::execute(&mut harness.state_mut().session, "tool.stencil", &serde_json::json!({"image": name})).unwrap();
    harness.run_steps(3);

    let start = model_pixel_near(&harness, harness.state().session.viewport * 0.5);
    let local = Vec2::new(start.x - harness.state().viewport_rect.left(), start.y - harness.state().viewport_rect.top());
    let viewport = harness.state().session.viewport;
    harness.state_mut().session.begin_stroke(local, viewport, 1.0).unwrap();
    harness.run_steps(3);
    assert!(harness.state().session.stroke_active());
    let hidden = stencil_quads(&harness);

    harness.state_mut().session.tool.hide_stencil_while_painting = false;
    harness.run_steps(3);
    assert_eq!(stencil_quads(&harness), hidden + 1, "the option off: the stencil image is drawn during the stroke");

    harness.state_mut().session.tool.hide_stencil_while_painting = true;
    harness.state_mut().session.cancel_stroke();
    harness.run_steps(3);
    assert!(!harness.state().session.stroke_active());
}
