//! Shift+F frames the active texture set, F the whole mesh; a floor plane must not decide the view

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use texturecraft_engine::Session;
use texturecraft_ui_egui::TextureCraftApp;

const SCENE_OBJ: &str = "o Hydrant\nusemtl Hydrant\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n\
o Floor\nusemtl Floor\nv -100 -1 -100\nv 100 -1 -100\nv 0 -1 100\nvt 0 0\nvt 1 0\nvt 0 1\nf 4/4 5/5 6/6\n";

fn harness() -> Harness<'static, TextureCraftApp> {
    let session = Session::from_mesh_bytes("scene.obj", SCENE_OBJ.as_bytes(), 256, "scene").unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.run_steps(4);
    h
}

fn press(h: &mut Harness<'_, TextureCraftApp>, key: Key, modifiers: Modifiers) {
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.run_steps(2);
}

#[test]
fn shift_f_frames_the_painted_object_and_f_the_whole_scene() {
    let mut h = harness();
    let whole = h.state().session.camera;

    press(&mut h, Key::F, Modifiers::SHIFT);
    let framed = h.state().session.camera;
    assert!((framed.target - texturecraft_engine::geom::glam::Vec3::new(0.5, 0.5, 0.0)).length() < 1e-4, "{:?}", framed.target);
    assert!(framed.distance < whole.distance * 0.05, "{} vs {}", framed.distance, whole.distance);

    press(&mut h, Key::F, Modifiers::NONE);
    let back = h.state().session.camera;
    assert!((back.target - whole.target).length() < 1e-4 && (back.distance - whole.distance).abs() < 1e-3, "{back:?} vs {whole:?}");
}
