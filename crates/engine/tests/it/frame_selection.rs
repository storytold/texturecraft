//! Frame texture set (Shift+F) and Show entire mesh (F): a floor plane must not decide the camera

use serde_json::{Value, json};
use texturecraft_engine::geom::glam::Vec3;
use texturecraft_engine::{Session, execute};

/// A small "Hydrant" (set 0) near the origin and a huge "Floor" (set 1) below and around it
const SCENE_OBJ: &str = "o Hydrant\nusemtl Hydrant\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n\
o Floor\nusemtl Floor\nv -100 -1 -100\nv 100 -1 -100\nv 0 -1 100\nvt 0 0\nvt 1 0\nvt 0 1\nf 4/4 5/5 6/6\n";

fn scene() -> Session {
    let session = Session::from_mesh_bytes("scene.obj", SCENE_OBJ.as_bytes(), 256, "scene").unwrap();
    let sets: Vec<_> = session.doc.project.texture_sets.iter().map(|set| session.mesh.materials[set.material].as_str()).collect();
    assert_eq!(sets, ["Hydrant", "Floor"], "the sets follow the parts");
    session
}

fn target(session: &Session) -> Vec3 {
    session.camera.target
}

fn near(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < 1e-4
}

#[test]
fn frame_texture_set_frames_only_the_active_set() {
    let mut session = scene();
    let all = session.camera.distance;

    execute(&mut session, "viewport.frameSelection", &Value::Null).unwrap();

    assert!(near(target(&session), Vec3::new(0.5, 0.5, 0.0)), "the camera looks at the hydrant, not the floor: {:?}", target(&session));
    assert!(session.camera.distance < all * 0.05, "the hydrant is about 1 unit across, the whole scene about 280: {} vs {all}", session.camera.distance);
}

#[test]
fn frame_texture_set_follows_the_selected_set() {
    let mut session = scene();

    execute(&mut session, "textureSet.select", &json!({"ts": 1})).unwrap();
    execute(&mut session, "viewport.frameSelection", &Value::Null).unwrap();

    assert!(near(target(&session), Vec3::new(0.0, -1.0, 0.0)), "{:?}", target(&session));
    assert!(session.camera.distance > 100.0, "the floor is wide: {}", session.camera.distance);
}

#[test]
fn orbit_turns_around_the_framed_center() {
    let mut session = scene();
    execute(&mut session, "viewport.frameSelection", &Value::Null).unwrap();
    let (center, distance) = (target(&session), session.camera.distance);

    for (dx, dy) in [(90.0, 0.0), (0.0, 30.0), (-45.0, -10.0)] {
        execute(&mut session, "camera.orbit", &json!({"dx": dx, "dy": dy})).unwrap();
        assert!(near(target(&session), center), "orbiting keeps the pivot");
        assert!(((session.camera.eye() - center).length() - distance).abs() < 1e-3, "and the eye stays that far from the framed center");
    }
}

#[test]
fn show_entire_mesh_restores_the_whole_scene() {
    let mut session = scene();
    let whole = (target(&session), session.camera.distance);
    execute(&mut session, "viewport.frameSelection", &Value::Null).unwrap();
    assert!(!near(target(&session), whole.0));

    execute(&mut session, "viewport.frame", &Value::Null).unwrap();

    assert!(near(target(&session), whole.0), "{:?}", target(&session));
    assert!((session.camera.distance - whole.1).abs() < 1e-3);
}

#[test]
fn viewport_frame_takes_a_target() {
    let mut session = scene();
    let whole = session.camera.distance;

    let selection = execute(&mut session, "viewport.frame", &json!({"target": "selection"})).unwrap();
    assert!(session.camera.distance < whole * 0.05);
    assert_eq!(selection["target"], json!([0.5, 0.5, 0.0]));

    execute(&mut session, "viewport.frame", &json!({"target": "all"})).unwrap();
    assert!((session.camera.distance - whole).abs() < 1e-3);

    assert!(execute(&mut session, "viewport.frame", &json!({"target": "floor"})).is_err());
}

#[test]
fn a_set_without_triangles_frames_the_whole_mesh() {
    let mut session = scene();
    let whole = (target(&session), session.camera.distance);
    session.doc.project.texture_sets[0].material = 7;

    execute(&mut session, "viewport.frameSelection", &Value::Null).unwrap();

    assert!(near(target(&session), whole.0) && (session.camera.distance - whole.1).abs() < 1e-3);
}

#[test]
fn a_saved_overview_camera_is_restored_after_framing_a_set() {
    let mut session = scene();
    execute(&mut session, "camera.add", &json!({"name": "Overview"})).unwrap();
    let overview = session.camera.distance;
    execute(&mut session, "viewport.frameSelection", &Value::Null).unwrap();

    execute(&mut session, "camera.select", &json!({"name": "Overview"})).unwrap();

    assert!((session.camera.distance - overview).abs() < 1e-3, "{} vs {overview}", session.camera.distance);
}
