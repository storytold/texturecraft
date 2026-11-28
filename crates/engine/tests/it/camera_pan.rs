//! Panning moves the orbit pivot to the part of the mesh panned to, not a point at the old depth

use serde_json::json;
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::{Vec2, Vec3};
use texturecraft_engine::{Session, execute};

/// A wide back plate at z = -2 and a small plate in front of it at z = 2, right of the middle
const PLATES_OBJ: &str = "o Plates\nusemtl Plates\n\
v -3 -1 -2\nv 3 -1 -2\nv 3 1 -2\nv -3 1 -2\n\
v 0.5 -1 2\nv 3 -1 2\nv 3 1 2\nv 0.5 1 2\n\
vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\n\
f 1/1 2/2 3/3\nf 1/1 3/3 4/4\nf 5/1 6/2 7/3\nf 5/1 7/3 8/4\n";

/// Looking straight down -Z at the origin from 10 units away
fn plates() -> Session {
    plates_of(PLATES_OBJ)
}

fn plates_of(obj: &str) -> Session {
    let mut session = Session::from_mesh_bytes("plates.obj", obj.as_bytes(), 256, "plates").unwrap();
    session.viewport = Vec2::new(800.0, 600.0);
    execute(&mut session, "camera.set", &json!({"yaw": 0.0, "pitch": 0.0, "distance": 10.0, "target": [0.0, 0.0, 0.0], "ortho": false})).unwrap();
    session
}

/// A pan of `x` world units to the right at the starting depth, in viewport heights
fn pan_right(session: &mut Session, x: f32) {
    let camera = session.camera;
    let height = camera.distance * (camera.fov.to_radians() * 0.5).tan() * 2.0;
    execute(session, "camera.pan", &json!({"dx": -x / height, "dy": 0.0})).unwrap();
}

#[test]
fn panning_onto_a_nearer_part_moves_the_pivot_onto_it() {
    let mut session = plates();
    let eye = session.camera.eye();
    let mut plain = session.camera;
    let height = plain.distance * (plain.fov.to_radians() * 0.5).tan() * 2.0;
    plain.pan(-1.75 / height, 0.0);

    pan_right(&mut session, 1.75);

    let target = session.camera.target;
    assert!((target.z - 2.0).abs() < 1e-3, "the pivot sits on the front plate, not at the back depth: {target:?}");
    assert!(target.x > 0.5, "and on the part panned to: {target:?}");
    assert!((session.camera.eye() - plain.eye()).length() < 1e-3, "the eye goes where a plain pan puts it, so the view doesn't jump");
    assert!((session.camera.eye() - eye).length() > 0.1, "the pan still moved the view");

    let center = session.pick(session.viewport * 0.5, session.viewport).unwrap().pos;
    execute(&mut session, "camera.orbit", &json!({"dx": 40.0, "dy": 15.0})).unwrap();
    let after = session.pick(session.viewport * 0.5, session.viewport).unwrap().pos;
    assert!((after - center).length() < 1e-2, "orbiting turns around the panned-to point, which stays in the middle: {center:?} → {after:?}");
}

#[test]
fn panning_off_the_mesh_keeps_the_pivot_depth() {
    let mut session = plates();
    let depth = session.camera.target.z;

    pan_right(&mut session, 20.0);

    assert!((session.camera.target.z - depth).abs() < 1e-4, "nothing under the view center: the pivot keeps its depth");
    assert!(session.camera.target.x > 5.0, "the pan itself still happened: {:?}", session.camera.target);
}

/// The plates with the front one in a texture set of its own
const SPLIT_PLATES_OBJ: &str = "o Plates\n\
v -3 -1 -2\nv 3 -1 -2\nv 3 1 -2\nv -3 1 -2\n\
v 0.5 -1 2\nv 3 -1 2\nv 3 1 2\nv 0.5 1 2\n\
vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\n\
usemtl Back\nf 1/1 2/2 3/3\nf 1/1 3/3 4/4\nusemtl Front\nf 5/1 6/2 7/3\nf 5/1 7/3 8/4\n";

#[test]
fn a_hidden_texture_set_never_takes_the_pivot() {
    let mut session = plates_of(SPLIT_PLATES_OBJ);
    let front = session.doc.project.texture_sets.iter().position(|set| set.name == "Front").unwrap();
    execute(&mut session, "textureSet.visible", &json!({"ts": front, "visible": false})).unwrap();

    pan_right(&mut session, 1.75);

    let target = session.camera.target;
    assert!((target.z + 2.0).abs() < 1e-3, "the pivot lands on the back plate seen through the hidden front one: {target:?}");
}

#[test]
fn an_orthographic_pan_keeps_what_is_in_front_of_the_new_pivot_in_view() {
    let mut session = plates();
    execute(&mut session, "camera.set", &json!({"ortho": true, "distance": 3.0})).unwrap();
    let on_front = Vec3::new(0.8, 0.0, 2.0);

    // The view center moves onto the back plate, with the front one's edge still in view
    pan_right(&mut session, -0.5);

    assert!(session.camera.target.z < -0.5, "the pivot moves toward the back plate: {:?}", session.camera.target);
    let ndc = session.camera.project(on_front, session.viewport.x / session.viewport.y).unwrap();
    let pixel = Vec2::new((ndc.x * 0.5 + 0.5) * session.viewport.x, (0.5 - ndc.y * 0.5) * session.viewport.y);
    assert!((0.0..=1.0).contains(&ndc.z), "the front plate stays between the clipping planes: depth {}", ndc.z);
    assert!(ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0, "and inside the viewport: {ndc:?}");
    let center = session.pick(session.viewport * 0.5, session.viewport).unwrap();
    assert!((center.pos.z + 2.0).abs() < 1e-3, "the back plate is still what fills the view center: {:?}", center.pos);
    let hit = session.pick(pixel, session.viewport).unwrap();
    assert!((hit.pos.z - 2.0).abs() < 1e-3, "and is what the brush finds there: {:?}", hit.pos);
}

/// Texels of the back plate a screen-space dab at `pixel` paints red
fn screen_space_dab(session: &mut Session, pixel: [f32; 2]) -> usize {
    let back = session.doc.project.texture_sets.iter().position(|set| set.name == "Back").unwrap();
    execute(session, "textureSet.select", &json!({"ts": back})).unwrap();
    session.update();
    execute(session, "layer.add.paint", &json!({})).unwrap();
    execute(session, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(session, "tool.brush", &json!({"size": 5, "hardness": 100, "screenSpace": true})).unwrap();
    execute(session, "tool.stroke", &json!({"points": [pixel]})).unwrap();
    session.update();
    let composite = &session.caches[back].composite;
    (0..256u32)
        .flat_map(|y| (0..256u32).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let color = composite.value(Channel::BaseColor, x, y);
            color[0] - color[1] > 0.5
        })
        .count()
}

#[test]
fn a_screen_space_brush_keeps_its_size_on_screen_when_a_pan_seats_the_pivot() {
    let mut seated = plates_of(SPLIT_PLATES_OBJ);
    let mut plain = plates_of(SPLIT_PLATES_OBJ);
    let height = plain.camera.distance * (plain.camera.fov.to_radians() * 0.5).tan() * 2.0;
    plain.camera.pan(-1.75 / height, 0.0);
    pan_right(&mut seated, 1.75);
    assert!((seated.camera.eye() - plain.camera.eye()).length() < 1e-3, "the same view, only the pivot differs");
    assert!(seated.camera.distance < plain.camera.distance - 1.0);

    // Left of the middle, on the back plate in both
    let (painted_plain, painted_seated) = (screen_space_dab(&mut plain, [200.0, 300.0]), screen_space_dab(&mut seated, [200.0, 300.0]));
    assert!(painted_plain > 0);
    let ratio = painted_seated as f32 / painted_plain as f32;
    assert!((ratio - 1.0).abs() < 0.05, "the dab covers the same texels: {painted_plain} vs {painted_seated}");
}
