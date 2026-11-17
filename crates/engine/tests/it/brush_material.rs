//! A simple shelf material can be the brush's material (`tool.material {material}`): no layer, and the
//! next stroke paints it. Shelf drops place new layers with `below` and `top`.

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::materials::{MATERIALS, brush_values};
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

fn stack(s: &Session) -> Vec<LayerId> {
    s.doc.project.texture_set(0).unwrap().stack.clone()
}

#[test]
fn every_simple_material_has_brush_values_and_no_smart_material_has() {
    for material in MATERIALS {
        let values = brush_values(material.id);
        assert_eq!(values.is_some(), !material.smart, "{}", material.id);
        if let Some(values) = values {
            let channels: Vec<Channel> = values.iter().map(|(channel, _)| *channel).collect();
            for channel in [Channel::BaseColor, Channel::Metallic, Channel::Roughness] {
                assert!(channels.contains(&channel), "{} has no {channel:?}", material.id);
            }
        }
    }
    assert!(brush_values("nope").is_none());
}

#[test]
fn a_material_set_on_the_brush_adds_no_layer_and_no_history() {
    let mut s = session();
    let (layers, history) = (stack(&s), s.history_labels());

    execute(&mut s, "tool.material", &json!({"material": "copper_hammered"})).unwrap();

    assert_eq!(stack(&s), layers);
    assert_eq!(s.history_labels(), history);
    for (channel, value) in brush_values("copper_hammered").unwrap() {
        assert_eq!(s.tool.values[&channel], value, "{channel:?}");
    }
    assert!(s.tool.values[&Channel::Height][0] < 0.0, "the material's height comes along");
    assert!(s.tool.enabled.contains(&Channel::Roughness), "the channel toggles stay as they were");
}

#[test]
fn a_material_without_height_resets_the_height_of_the_previous_one() {
    let mut s = session();
    execute(&mut s, "tool.material", &json!({"material": "copper_hammered"})).unwrap();
    assert!(s.tool.values[&Channel::Height][0] < 0.0);

    execute(&mut s, "tool.material", &json!({"material": "copper"})).unwrap();

    assert_eq!(s.tool.values[&Channel::Height], Channel::Height.default_value());
}

#[test]
fn a_smart_material_or_an_unknown_one_cannot_be_the_brush_material() {
    let mut s = session();
    let before = s.tool.values.clone();

    for id in ["iron_rusted", "nope"] {
        assert!(execute(&mut s, "tool.material", &json!({"material": id})).is_err(), "{id}");
    }
    assert_eq!(s.tool.values, before);
}

/// Texel of the sphere point under screen position `(x, y)`
fn texel(s: &Session, x: f32, y: f32) -> (u32, u32) {
    let hit = s.pick(Vec2::new(x, y), s.viewport).unwrap();
    ((hit.uv.x * 256.0) as u32, (hit.uv.y * 256.0) as u32)
}

/// A session with `material` as the brush material, brush `size` (0..100), the stroke along y = 200 over `xs`
fn painted(material: &str, size: u32, xs: [f32; 2]) -> Session {
    let mut s = session();
    paint(&mut s, material, size, xs);
    s
}

fn paint(s: &mut Session, material: &str, size: u32, xs: [f32; 2]) {
    execute(s, "tool.material", &json!({"material": material})).unwrap();
    execute(s, "tool.brush", &json!({"size": size, "hardness": 100})).unwrap();
    execute(s, "tool.stroke", &json!({"points": [[xs[0], 200], [xs[1], 200]]})).unwrap();
    s.update();
}

/// What a fill layer of `material` (projection scale `scale`) shows over the sphere
fn fill_of(material: &str, scale: f32) -> Session {
    let mut s = session();
    let layer = LayerId(execute(&mut s, "material.apply", &json!({"material": material})).unwrap()["layer"].as_u64().unwrap());
    execute(&mut s, "layer.fill.projection", &json!({"layer": layer.0, "projection": "triplanar", "scale": scale})).unwrap();
    s.update();
    s
}

fn at(s: &Session, channel: Channel, (x, y): (u32, u32)) -> f32 {
    s.caches[0].composite.value(channel, x, y)[0]
}

#[test]
fn a_stroke_after_picking_a_material_paints_its_pattern_like_a_fill_of_it() {
    // Brush size 40 is the pattern's natural size: the same as a fill layer with the material
    let (stroke, fill) = (painted("cobblestone", 40, [150.0, 250.0]), fill_of("cobblestone", 1.0));
    let samples: Vec<f32> = (0..24).map(|i| 170.0 + 3.0 * i as f32).collect();

    let mut colours = Vec::new();
    for x in samples {
        let texel = texel(&stroke, x, 200.0);
        for channel in [Channel::BaseColor, Channel::Roughness, Channel::Height] {
            let (painted, expected) = (at(&stroke, channel, texel), at(&fill, channel, texel));
            assert!((painted - expected).abs() < 0.02, "{channel:?} at x = {x}: {painted} vs {expected}");
        }
        colours.push(at(&stroke, Channel::BaseColor, texel));
    }
    let (low, high) = colours.iter().fold((f32::MAX, f32::MIN), |(low, high), v| (low.min(*v), high.max(*v)));
    assert!(high - low > 0.05, "the colour varies along the stroke: {colours:?}");
}

#[test]
fn overlapping_strokes_agree_where_they_overlap() {
    let mut s = painted("cobblestone", 40, [150.0, 215.0]);
    let samples: Vec<(u32, u32)> = (0..8).map(|i| texel(&s, 190.0 + 3.0 * i as f32, 200.0)).collect();
    let first: Vec<f32> = samples.iter().map(|t| at(&s, Channel::BaseColor, *t)).collect();

    paint(&mut s, "cobblestone", 40, [185.0, 250.0]);

    for (i, t) in samples.iter().enumerate() {
        assert!((at(&s, Channel::BaseColor, *t) - first[i]).abs() < 0.005, "sample {i}");
    }
}

#[test]
fn the_pattern_follows_the_brush_size() {
    // A quarter of the natural brush size is a pattern four times as fine
    let (stroke, fill) = (painted("cobblestone", 10, [194.0, 206.0]), fill_of("cobblestone", 4.0));
    let natural = fill_of("cobblestone", 1.0);
    let (mut fine, mut coarse) = (0.0_f32, 0.0_f32);
    for x in [197.0, 199.0, 201.0, 203.0] {
        let texel = texel(&stroke, x, 200.0);
        let painted = at(&stroke, Channel::BaseColor, texel);
        fine = fine.max((painted - at(&fill, Channel::BaseColor, texel)).abs());
        coarse = coarse.max((painted - at(&natural, Channel::BaseColor, texel)).abs());
    }
    assert!(fine < 0.02, "matches the pattern at 4x: {fine}");
    assert!(coarse > 0.02, "differs from the natural size: {coarse}");
}

#[test]
fn editing_a_brush_value_ends_the_pattern() {
    let mut s = session();
    execute(&mut s, "tool.material", &json!({"material": "cobblestone"})).unwrap();
    assert_eq!(s.tool.material.as_deref(), Some("cobblestone"));

    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();

    assert_eq!(s.tool.material, None);
}

/// Add copper with `place` params (above, below, top, index)
fn apply(s: &mut Session, place: Value) -> LayerId {
    let mut params = json!({"material": "copper"});
    params.as_object_mut().unwrap().extend(place.as_object().unwrap().clone());
    LayerId(execute(s, "material.apply", &params).unwrap()["layer"].as_u64().unwrap())
}

#[test]
fn a_material_dropped_on_a_row_goes_directly_below_it_and_one_dropped_on_the_view_goes_on_top() {
    let mut s = session();
    let paint = stack(&s)[0];
    let upper = apply(&mut s, json!({}));
    let lower = apply(&mut s, json!({"index": 0}));
    assert_eq!(stack(&s), vec![lower, paint, upper]);

    let below_paint = apply(&mut s, json!({"below": paint.0}));
    assert_eq!(stack(&s), vec![lower, below_paint, paint, upper]);

    let below_bottom = apply(&mut s, json!({"below": lower.0}));
    assert_eq!(stack(&s)[0], below_bottom, "below the bottom row is the bottom");

    s.selected.insert(0, lower);
    let top = apply(&mut s, json!({"top": true}));
    assert_eq!(stack(&s).last(), Some(&top), "the top, wherever the selection is");
    assert_eq!(s.selected_layer(), Some(top));
    s.doc.project.validate().unwrap();
}
