//! Hide/ignore excluded geometry (`viewport.hideExcluded`): parts a layer's geometry mask
//! leaves out stop counting for picking and strokes, so paint reaches the parts behind.

use serde_json::{Value, json};
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::{Session, execute};

/// A "Lid" triangle in front of a bigger "Body" triangle, in one texture set, seen from the front.
fn lid_over_body() -> Session {
    let obj = "o Lid\nv 0 0 0.5\nv 1 0 0.5\nv 0 1 0.5\nvt 0 0\nvt 0.5 0\nvt 0 0.5\nf 1/1 2/2 3/3\n\
               o Body\nv 0 0 0\nv 2 0 0\nv 0 2 0\nvt 0.5 0.5\nvt 1 0.5\nvt 0.5 1\nf 4/4 5/5 6/6\n";
    let mut s = Session::from_mesh_bytes("lid.obj", obj.as_bytes(), 256, "lid").unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    (s.camera.yaw, s.camera.pitch) = (0.0, 0.0);
    let (lo, hi) = s.mesh.bounds();
    s.camera.frame(lo, hi);
    if s.selected_layer().is_none() {
        execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    }
    let layer = s.selected_layer().unwrap().0;
    execute(&mut s, "layer.set", &json!({"layer": layer, "geometryMask": ["Body"]})).unwrap();
    s
}

/// The viewport pixel of the world point (0.2, 0.2), which both parts cover.
fn overlap(s: &Session) -> Vec2 {
    let n = s.camera.project(texturecraft_engine::geom::glam::Vec3::new(0.2, 0.2, 0.0), 1.0).unwrap();
    Vec2::new((n.x + 1.0) * 200.0, (1.0 - n.y) * 200.0)
}

fn texel_is_painted(s: &mut Session, uv: Vec2) -> bool {
    s.update();
    let (w, h) = s.caches[0].size;
    let v = s.caches[0].composite.value(Channel::BaseColor, (uv.x * w as f32) as u32, (uv.y * h as f32) as u32);
    v[0] > 0.85 && v[1] < 0.15
}

#[test]
fn picking_skips_the_parts_the_mask_excludes() {
    let mut s = lid_over_body();
    let at = overlap(&s);
    let lid = s.pick(at, s.viewport).unwrap();
    assert_eq!(s.mesh.submeshes[0].name, "Lid");
    assert!(lid.tri == 0, "off: the lid in front is hit");

    let r = execute(&mut s, "viewport.hideExcluded", &Value::Null).unwrap();
    assert_eq!((r["enabled"].as_bool(), &r["excludedParts"]), (Some(true), &json!(["Lid"])), "{r}");
    assert_eq!(s.excluded_parts(), [true, false]);
    assert_eq!(s.pick(at, s.viewport).unwrap().tri, 1, "on: the body behind is hit");

    execute(&mut s, "viewport.hideExcluded", &json!({"enabled": false})).unwrap();
    assert!(s.excluded_parts().is_empty());
    assert_eq!(s.pick(at, s.viewport).unwrap().tri, 0, "off again: the lid is back");
}

#[test]
fn nothing_is_hidden_when_the_layer_shows_on_every_part() {
    let mut s = lid_over_body();
    let layer = s.selected_layer().unwrap().0;
    execute(&mut s, "layer.set", &json!({"layer": layer, "geometryMask": null})).unwrap();
    execute(&mut s, "viewport.hideExcluded", &Value::Null).unwrap();
    assert!(s.excluded_parts().is_empty());
    assert_eq!(s.pick(overlap(&s), s.viewport).unwrap().tri, 0);
}

#[test]
fn strokes_paint_through_the_excluded_part() {
    for hidden in [false, true] {
        let mut s = lid_over_body();
        execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
        execute(&mut s, "tool.brush", &json!({"size": 20, "hardness": 100})).unwrap();
        let at = overlap(&s);

        // Where the point lands on each part
        let lid_uv = s.pick(at, s.viewport).unwrap().uv;
        execute(&mut s, "viewport.hideExcluded", &json!({"enabled": true})).unwrap();
        let body_uv = s.pick(at, s.viewport).unwrap().uv;
        assert!((lid_uv - body_uv).length() > 0.2, "the parts have their own UVs");
        execute(&mut s, "viewport.hideExcluded", &json!({"enabled": hidden})).unwrap();

        // Without the toggle the stroke lands on the lid in front (shown there: no mask)
        if !hidden {
            let layer = s.selected_layer().unwrap().0;
            execute(&mut s, "layer.set", &json!({"layer": layer, "geometryMask": null})).unwrap();
        }
        s.begin_stroke(at, s.viewport, 1.0).unwrap();
        s.continue_stroke(at + Vec2::new(4.0, 0.0), 1.0);
        s.end_stroke().unwrap();

        let (painted_lid, painted_body) = (texel_is_painted(&mut s, lid_uv), texel_is_painted(&mut s, body_uv));
        assert_eq!((painted_lid, painted_body), (!hidden, hidden), "hidden: {hidden}");
    }
}
