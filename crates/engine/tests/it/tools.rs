//! Clone, smudge, image and library alphas and the quick mask, driven through engine commands the way
//! agents paint.

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, LayerId};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, StackTarget, execute};

const RES: f32 = 256.0;

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

/// Base color of the composite under a viewport point.
fn color_at(s: &Session, x: f32, y: f32) -> [f32; 3] {
    let hit = s.pick(Vec2::new(x, y), s.viewport).unwrap();
    s.caches[0].composite.value(Channel::BaseColor, (hit.uv.x * RES) as u32, (hit.uv.y * RES) as u32)
}

fn red(v: [f32; 3]) -> bool {
    v[0] > 0.85 && v[1] < 0.15
}

fn dab(s: &mut Session, x: f32, y: f32) {
    execute(s, "tool.stroke", &json!({"points": [[x, y]]})).unwrap();
}

fn paint_red(s: &mut Session, size: f32) {
    execute(s, "tool.paint", &Value::Null).unwrap();
    execute(s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(s, "tool.brush", &json!({"size": size, "hardness": 90})).unwrap();
}

#[test]
fn clone_copies_values_from_the_source_offset() {
    let mut s = session();
    paint_red(&mut s, 20.0);
    dab(&mut s, 160.0, 200.0);
    assert!(red(color_at(&s, 160.0, 200.0)));
    assert!(!red(color_at(&s, 240.0, 200.0)));

    execute(&mut s, "tool.clone", &Value::Null).unwrap();
    assert!(execute(&mut s, "tool.stroke", &json!({"points": [[240, 200]]})).is_err(), "no source yet");
    execute(&mut s, "tool.cloneSource", &json!({"x": 160, "y": 200})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 6})).unwrap();
    dab(&mut s, 240.0, 200.0);
    let v = color_at(&s, 240.0, 200.0);
    assert!(red(v), "cloned red: {v:?}");

    // The offset stays for the next strokes: the source moved along with the destination
    let off = s.tool.clone_source.unwrap().offset.unwrap();
    assert!(off[0].abs() > 0.01, "{off:?}");

    // One undo step per clone stroke
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    s.update();
    assert!(!red(color_at(&s, 240.0, 200.0)));
}

/// A red dab at (160, 200) as the clone source, the Clone tool selected with a small brush.
fn clone_ready() -> Session {
    let mut s = session();
    paint_red(&mut s, 20.0);
    dab(&mut s, 160.0, 200.0);
    execute(&mut s, "tool.clone", &Value::Null).unwrap();
    execute(&mut s, "tool.cloneSource", &json!({"x": 160, "y": 200})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 6})).unwrap();
    s
}

#[test]
fn aligned_clone_keeps_the_first_offset_for_later_strokes() {
    let mut s = clone_ready();
    assert_eq!(execute(&mut s, "tool.brush", &Value::Null).unwrap()["cloneAligned"], true, "aligned by default");
    dab(&mut s, 240.0, 200.0);
    assert!(red(color_at(&s, 240.0, 200.0)));

    // The second stroke reads at the same offset, 40 points left of the first source: nothing red there
    dab(&mut s, 200.0, 200.0);
    assert!(!red(color_at(&s, 200.0, 200.0)), "aligned: the source moved along");
    assert!(s.tool.clone_source.unwrap().offset.is_some());
}

#[test]
fn not_aligned_clone_starts_every_stroke_at_the_source() {
    let mut s = clone_ready();
    let r = execute(&mut s, "tool.brush", &json!({"cloneAligned": false})).unwrap();
    assert_eq!(r["cloneAligned"], false);
    dab(&mut s, 240.0, 200.0);
    assert!(red(color_at(&s, 240.0, 200.0)), "first stroke copies the source");
    assert!(s.tool.clone_source.unwrap().offset.is_none(), "no offset is kept");
    dab(&mut s, 200.0, 200.0);
    assert!(red(color_at(&s, 200.0, 200.0)), "the second stroke starts at the source again");
}

#[test]
fn switching_the_clone_mode_forgets_the_offset_of_earlier_strokes() {
    let mut s = clone_ready();
    dab(&mut s, 240.0, 200.0);
    assert!(s.tool.clone_source.unwrap().offset.is_some());
    execute(&mut s, "tool.brush", &json!({"cloneAligned": false})).unwrap();
    execute(&mut s, "tool.brush", &json!({"cloneAligned": true})).unwrap();
    assert!(s.tool.clone_source.unwrap().offset.is_none(), "the next aligned stroke fixes a fresh offset");
}

#[test]
fn the_clone_source_is_forgotten_when_another_texture_set_becomes_active() {
    let mut s = texturecraft_engine::sample_session(256, 64).unwrap();
    execute(&mut s, "tool.clone", &Value::Null).unwrap();
    execute(&mut s, "tool.cloneSource", &json!({"uv": [0.3, 0.6]})).unwrap();

    // Selecting the active set again keeps it
    let active = s.ts();
    execute(&mut s, "textureSet.select", &json!({"ts": active})).unwrap();
    assert!(s.tool.clone_source.is_some());
    execute(&mut s, "textureSet.select", &json!({"ts": 1})).unwrap();
    assert!(s.tool.clone_source.is_none(), "a point of set 0 means nothing in set 1");
    assert!(execute(&mut s, "tool.stroke", &json!({"points": [[200, 200]]})).is_err(), "asks for a source again");

    // The same through selecting a layer that lives in another set
    let other = s.doc.project.texture_sets[0].stack.last().copied().expect("set 0 has a layer");
    execute(&mut s, "tool.cloneSource", &json!({"uv": [0.3, 0.6]})).unwrap();
    execute(&mut s, "layer.select", &json!({"layer": other.0})).unwrap();
    assert_eq!(s.ts(), 0);
    assert!(s.tool.clone_source.is_none());

    execute(&mut s, "tool.cloneSource", &json!({"uv": [0.3, 0.6]})).unwrap();
    execute(&mut s, "layer.select", &json!({"layers": [other.0]})).unwrap();
    assert!(s.tool.clone_source.is_some());
    let layers = s.doc.project.texture_sets[1].stack.clone();
    execute(&mut s, "layer.select", &json!({"layers": layers})).unwrap();
    assert_eq!(s.ts(), 1);
    assert!(s.tool.clone_source.is_none());
}

#[test]
fn smudge_drags_color_along_the_stroke() {
    let mut s = session();
    paint_red(&mut s, 12.0);
    dab(&mut s, 170.0, 200.0);
    let before = color_at(&s, 205.0, 200.0);
    assert!(!red(before), "{before:?}");
    execute(&mut s, "tool.smudge", &Value::Null).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 10, "smudgeStrength": 100, "spacing": 5})).unwrap();
    let pts: Vec<[f32; 2]> = (0..=40).map(|i| [170.0 + i as f32, 200.0]).collect();
    execute(&mut s, "tool.stroke", &json!({"points": pts})).unwrap();
    let after = color_at(&s, 205.0, 200.0);
    assert!(after[0] - before[0] > 0.2 && after[1] < before[1], "red dragged right: {before:?} -> {after:?}");
}

#[test]
fn smudge_leaves_a_channel_alone_that_the_material_has_off() {
    let mut s = session();
    paint_red(&mut s, 12.0);
    dab(&mut s, 170.0, 200.0);
    let before = color_at(&s, 205.0, 200.0);
    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "enabled": false})).unwrap();
    execute(&mut s, "tool.smudge", &Value::Null).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 10, "smudgeStrength": 100, "spacing": 5})).unwrap();
    let pts: Vec<[f32; 2]> = (0..=40).map(|i| [170.0 + i as f32, 200.0]).collect();
    execute(&mut s, "tool.stroke", &json!({"points": pts})).unwrap();
    let after = color_at(&s, 205.0, 200.0);
    assert!((after[0] - before[0]).abs() < 0.02 && (after[1] - before[1]).abs() < 0.02, "base color not dragged: {before:?} -> {after:?}");
}

#[test]
fn image_alpha_shapes_the_stamp() {
    let mut s = session();
    let mut png = Vec::new();
    // Top half white, bottom half black.
    let img = image::RgbaImage::from_fn(16, 16, |_, y| if y < 8 { image::Rgba([255; 4]) } else { image::Rgba([0, 0, 0, 255]) });
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = s.import_resource_bytes("half.png", png).unwrap();
    paint_red(&mut s, 20.0);
    assert!(execute(&mut s, "tool.brush", &json!({"alpha": {"image": "missing.png"}})).is_err());
    let r = execute(&mut s, "tool.brush", &json!({"alpha": {"image": name}})).unwrap();
    assert_eq!(r["alpha"]["image"], json!(name));
    execute(&mut s, "tool.strokeUv", &json!({"points": [[0.5, 0.5]]})).unwrap();
    // In the 2D view +v points down: the image's top half lands above the center.
    let at = |s: &Session, dy: i32| s.caches[0].composite.value(Channel::BaseColor, 128, (128 + dy) as u32);
    assert!(red(at(&s, -12)), "{:?}", at(&s, -12));
    assert!(!red(at(&s, 12)), "{:?}", at(&s, 12));
    // A procedural alpha clears the image.
    let r = execute(&mut s, "tool.brush", &json!({"alpha": "round"})).unwrap();
    assert_eq!(r["alpha"], json!("Round"));
}

/// Offsets (dx, dy) of red composite texels within `r` texels of (cx, cy), with their lengths.
fn red_offsets(s: &Session, cx: i32, cy: i32, r: i32) -> Vec<(i32, i32, f32)> {
    let mut out = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            let (x, y) = (cx + dx, cy + dy);
            if (0..RES as i32).contains(&x) && (0..RES as i32).contains(&y) && red(s.caches[0].composite.value(Channel::BaseColor, x as u32, y as u32)) {
                out.push((dx, dy, ((dx * dx + dy * dy) as f32).sqrt()));
            }
        }
    }
    out
}

#[test]
fn library_alphas_shape_the_stamp() {
    let mut s = session();
    paint_red(&mut s, 20.0);
    let list = execute(&mut s, "alpha.list", &Value::Null).unwrap();
    assert!(list["library"].as_array().is_some_and(|a| a.len() >= 60), "{list}");
    assert_eq!(list["procedural"][0], json!("Round"));
    assert!(execute(&mut s, "tool.brush", &json!({"alpha": {"library": "nope"}})).is_err());
    let r = execute(&mut s, "tool.brush", &json!({"alpha": {"library": "Ring"}})).unwrap();
    assert_eq!(r["alpha"], json!({"library": "ring"}));

    // 2D view: a ring with a clean middle.
    execute(&mut s, "tool.strokeUv", &json!({"points": [[0.5, 0.5]]})).unwrap();
    let ring = red_offsets(&s, 128, 128, 48);
    let outer = ring.iter().map(|t| t.2).fold(0.0, f32::max);
    assert!(outer > 6.0 && outer < 47.0, "{outer}");
    assert!(ring.iter().all(|t| t.2 > outer * 0.45), "the middle stays clean");
    let mid = (outer * 0.72 / 0.92).round() as i32;
    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        assert!(ring.iter().any(|t| (t.0, t.1) == (dx * mid, dy * mid)), "ring at {:?}", (dx * mid, dy * mid));
    }

    // 3D view: the same hollow stamp on the sphere.
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    s.update();
    dab(&mut s, 200.0, 200.0);
    let hit = s.pick(Vec2::new(200.0, 200.0), s.viewport).unwrap();
    let (cx, cy) = ((hit.uv.x * RES) as i32, (hit.uv.y * RES) as i32);
    let ring = red_offsets(&s, cx, cy, 48);
    assert!(!ring.is_empty(), "painted");
    assert!(ring.iter().all(|t| t.2 > 2.0), "the middle stays clean");

    // A procedural alpha clears the library alpha.
    let r = execute(&mut s, "tool.brush", &json!({"alpha": "square"})).unwrap();
    assert_eq!(r["alpha"], json!("Square"));
    assert!(s.tool.brush.alpha_library.is_none());
}

#[test]
fn quick_mask_limits_painting() {
    let mut session = session();
    paint_red(&mut session, 20.0);
    execute(&mut session, "viewport.quickMask", &json!({"enabled": true})).unwrap();

    let state = execute(&mut session, "viewport.quickMask.edit", &Value::Null).unwrap();
    let undo = session.history_labels().len();
    dab(&mut session, 150.0, 200.0);

    assert_eq!(state["editing"], json!(true));
    assert!(!red(color_at(&session, 150.0, 200.0)), "mask strokes do not paint the layer");
    assert_eq!(session.history_labels().len(), undo, "mask strokes stay out of the history");
    let overlay_size: u32 = 64;
    let rgba_bytes = (overlay_size * overlay_size * 4) as usize;
    assert!(session.quick_mask_rgba(0, overlay_size).is_some_and(|pixels| pixels.len() == rgba_bytes), "an RGBA8 overlay of the requested size");

    execute(&mut session, "viewport.quickMask.edit", &Value::Null).unwrap();
    execute(&mut session, "tool.brush", &json!({"size": 6})).unwrap();
    dab(&mut session, 150.0, 200.0);
    dab(&mut session, 250.0, 200.0);

    assert!(red(color_at(&session, 150.0, 200.0)), "inside the mask");
    assert!(!red(color_at(&session, 250.0, 200.0)), "outside the mask");

    execute(&mut session, "viewport.quickMask.invert", &Value::Null).unwrap();
    dab(&mut session, 250.0, 200.0);

    assert!(red(color_at(&session, 250.0, 200.0)), "inverted: outside is open");

    execute(&mut session, "viewport.quickMask", &json!({"enabled": false})).unwrap();
    execute(&mut session, "tool.material", &json!({"channel": "baseColor", "color": [0.0, 1.0, 0.0]})).unwrap();
    dab(&mut session, 150.0, 200.0);

    assert!(color_at(&session, 150.0, 200.0)[1] > 0.85, "disabled: everything is open");
}

/// How much redder than green a texel must be to count as painted (faded edges included)
const PAINTED_TINT: f32 = 0.05;

/// Separate painted runs down texture column `x` (base color tinted red)
fn painted_runs(session: &Session, x: u32) -> usize {
    let painted = |y: u32| {
        let color = session.caches[0].composite.value(Channel::BaseColor, x, y);
        color[0] - color[1] > PAINTED_TINT
    };
    (0..RES as u32).filter(|&y| painted(y) && (y == 0 || !painted(y - 1))).count()
}

#[test]
fn a_stroke_to_the_edge_of_the_mesh_fades_without_breaking_into_dots() {
    let mut session = session();
    paint_red(&mut session, 4.0);
    // From the middle of the view to past the ball's right edge, above center
    let points: Vec<Value> = (0..40).map(|step| json!([200.0 + step as f32 * 6.0, 165.0])).collect();
    execute(&mut session, "tool.stroke", &json!({"points": points})).unwrap();
    session.update();

    // The stroke crosses each texture column once, also where the surface turns away from the camera
    let broken: Vec<u32> = (0..RES as u32).filter(|&x| painted_runs(&session, x) > 1).collect();
    assert!(broken.is_empty(), "columns with gaps: {broken:?}");
}

#[test]
fn swap_mask_value_inverts_only_while_a_mask_is_the_target() {
    let mut session = session();
    execute(&mut session, "layer.add.paint", &Value::Null).unwrap();
    let swap = |session: &mut Session| execute(session, "tool.swapMaskValue", &Value::Null);
    let effect_target = |result: Value| StackTarget::Effect(LayerId(result["effect"].as_u64().unwrap()));

    assert!(swap(&mut session).is_err(), "painting the layer content: nothing to invert");

    let content_effect = effect_target(execute(&mut session, "effect.add.paint", &json!({"target": "content"})).unwrap());
    session.target = content_effect;
    assert!(swap(&mut session).is_err(), "a content effect is not a mask");

    execute(&mut session, "mask.add.white", &Value::Null).unwrap();
    session.target = StackTarget::Mask;
    session.tool.mask_value = 0.2;

    swap(&mut session).unwrap();

    assert_eq!(session.tool.mask_value, 0.8);

    session.tool.mask_value = 1.0;

    swap(&mut session).unwrap();

    assert_eq!(session.tool.mask_value, 0.0);

    swap(&mut session).unwrap();

    assert_eq!(session.tool.mask_value, 1.0, "pressing it again goes back");

    session.target = effect_target(execute(&mut session, "effect.add.paint", &json!({"target": "mask"})).unwrap());

    swap(&mut session).unwrap();

    assert_eq!(session.tool.mask_value, 0.0, "a mask effect paints the mask too");
}

#[test]
fn stroke_after_undoing_mask_is_paint_stroke() {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:random", None, 256).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    execute(&mut s, "layer.add.fill", &json!({"color": [0.2, 0.5, 0.3], "roughness": 0.7})).unwrap();
    execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    s.update();
    execute(&mut s, "tool.brush", &json!({"size": 20, "hardness": 50, "opacity": 0.8})).unwrap();
    execute(&mut s, "tool.paint", &Value::Null).unwrap();
    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": [0.9, 0.3, 0.9]})).unwrap();

    execute(&mut s, "tool.stroke", &json!({"points": [[130, 130], [170, 150]]})).unwrap();
    s.update();
    execute(&mut s, "mask.add.white", &Value::Null).unwrap();
    s.update();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    s.update();
    execute(&mut s, "tool.stroke", &json!({"points": [[167, 183], [207, 203]]})).unwrap();
    s.update();

    let undone = execute(&mut s, "edit.undo", &Value::Null).unwrap();
    let layer = s.doc.project.layer(s.selected_layer().unwrap()).unwrap().name.clone();
    assert_eq!(undone["undone"], format!("Paint on {layer}"), "the stroke after undoing the mask was recorded as {}", undone["undone"]);
    execute(&mut s, "edit.redo", &Value::Null).unwrap();
    s.update();

    let settled = s.caches[0].composite.clone();
    s.caches[0].below.clear();
    s.caches[0].invalidate_all();
    s.update();
    assert_eq!(settled, s.caches[0].composite, "the composite after the stroke matches a full recomposite");
}

#[test]
fn undoing_after_painting_on_instance_source_mask_keeps_mask_target() {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:random", None, 256).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    execute(&mut s, "mask.add.white", &Value::Null).unwrap();
    execute(&mut s, "layer.instance", &Value::Null).unwrap();
    s.update();
    s.target = StackTarget::Mask;
    execute(&mut s, "tool.brush", &json!({"size": 20, "hardness": 50, "opacity": 0.8})).unwrap();
    execute(&mut s, "tool.stroke", &json!({"points": [[170, 180], [210, 200]]})).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(s.target, StackTarget::Mask);

    execute(&mut s, "tool.stroke", &json!({"points": [[180, 180], [210, 200]]})).unwrap();
    let undone = execute(&mut s, "edit.undo", &Value::Null).unwrap();
    let layer = s.doc.project.layer(s.selected_layer().unwrap()).unwrap().name.clone();
    assert_eq!(undone["undone"], format!("Paint mask of {layer}"));
}

/// The name of the selected layer
fn selected_name(s: &Session) -> String {
    s.doc.project.layer(s.selected_layer().unwrap()).unwrap().name.clone()
}

/// The label of the newest history step
fn last_label(s: &Session) -> String {
    s.history_labels().pop().unwrap()
}

#[test]
fn strokes_are_named_by_tool_and_layer() {
    let mut s = session();
    execute(&mut s, "layer.rename", &json!({"name": "Rust"})).unwrap();
    paint_red(&mut s, 20.0);
    dab(&mut s, 160.0, 200.0);
    assert_eq!(last_label(&s), "Paint on Rust");

    execute(&mut s, "tool.clone", &Value::Null).unwrap();
    execute(&mut s, "tool.cloneSource", &json!({"x": 160, "y": 200})).unwrap();
    dab(&mut s, 240.0, 200.0);
    assert_eq!(last_label(&s), "Clone on Rust");

    execute(&mut s, "tool.smudge", &Value::Null).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 10, "smudgeStrength": 100, "spacing": 5})).unwrap();
    execute(&mut s, "tool.stroke", &json!({"points": [[150, 200], [200, 200], [250, 200]]})).unwrap();
    assert_eq!(last_label(&s), "Smudge on Rust");

    execute(&mut s, "tool.eraser", &Value::Null).unwrap();
    dab(&mut s, 160.0, 200.0);
    assert_eq!(last_label(&s), "Erase on Rust");

    // Another layer gives another name
    execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    execute(&mut s, "layer.rename", &json!({"name": "Dust"})).unwrap();
    execute(&mut s, "tool.paint", &Value::Null).unwrap();
    dab(&mut s, 160.0, 200.0);
    assert_eq!(last_label(&s), format!("Paint on {}", selected_name(&s)));
    assert_eq!(selected_name(&s), "Dust");
}

#[test]
fn the_label_template_of_every_tool_is_listed_for_translation() {
    use texturecraft_engine::{HISTORY_TEMPLATES, Tool, history_label, stroke_template};
    for tool in Tool::ALL {
        for on_mask in [false, true] {
            let template = stroke_template(tool, on_mask);
            assert!(HISTORY_TEMPLATES.contains(&template), "{template} is not in HISTORY_TEMPLATES");
            assert!(history_label(template, "Rust").contains("Rust"));
        }
    }
    assert_eq!(history_label("Paint on {layer}", "A {b}"), "Paint on A {b}");
}

/// Texels of base color red along a viewport row
fn red_width(s: &Session, y: f32) -> usize {
    (100..300).filter(|x| s.pick(Vec2::new(*x as f32, y), s.viewport).is_some() && red(color_at(s, *x as f32, y))).count()
}

#[test]
fn brush_alpha_parameters_are_set_read_back_and_narrow_the_stamp() {
    let mut full_session = session();
    paint_red(&mut full_session, 30.0);
    dab(&mut full_session, 200.0, 200.0);
    let full = red_width(&full_session, 200.0);
    assert!(full > 20, "{full}");

    let mut s = session();
    paint_red(&mut s, 30.0);

    let brush = execute(&mut s, "tool.brush", &json!({"alphaSizeX": 50, "alphaRoundness": 0, "alphaContrast": 2.5, "alphaInvert": false})).unwrap();
    assert_eq!((brush["alphaSizeX"].as_f64(), brush["alphaSizeY"].as_f64(), brush["alphaRoundness"].as_f64()), (Some(50.0), Some(100.0), Some(0.0)));
    assert_eq!((brush["alphaContrast"].as_f64(), brush["alphaInvert"].as_bool()), (Some(2.5), Some(false)));
    execute(&mut s, "tool.brush", &json!({"alphaRoundness": 100, "alphaContrast": 1})).unwrap();
    dab(&mut s, 200.0, 200.0);
    let narrow = red_width(&s, 200.0);
    assert!(narrow > 5 && narrow * 10 < full * 7, "half the stamp width: {narrow} of {full}");

    // Out-of-range values are clamped, not rejected
    let brush = execute(&mut s, "tool.brush", &json!({"alphaSizeX": 0, "alphaContrast": 99})).unwrap();
    assert_eq!((brush["alphaSizeX"].as_f64(), brush["alphaContrast"].as_f64()), (Some(5.0), Some(8.0)));
}

#[test]
fn brushes_saved_without_alpha_parameters_load_unchanged() {
    use texturecraft_engine::paint::{AlphaParams, BrushSettings};

    let mut saved = serde_json::to_value(BrushSettings::default()).unwrap();
    assert!(saved.as_object_mut().unwrap().remove("alpha_params").is_some());
    let loaded: BrushSettings = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(loaded, BrushSettings::default());

    saved["alpha_params"] = json!({"size_x": 0.5, "invert": true});
    let partial: BrushSettings = serde_json::from_value(saved).unwrap();
    assert_eq!(partial.alpha_params, AlphaParams { size_x: 0.5, invert: true, ..AlphaParams::default() });
}

#[test]
fn solo_keeps_the_document_source_for_clone_and_the_material_picker() {
    let mut s = clone_ready();
    let layer = s.selected_layer().unwrap();
    let empty = execute(&mut s, "layer.add.paint", &json!({})).unwrap()["layer"].as_u64().unwrap();
    execute(&mut s, "view.solo", &json!({"layer": empty})).unwrap();
    execute(&mut s, "layer.select", &json!({"layer": layer.0})).unwrap();
    s.update();
    assert!(!red(color_at(&s, 160.0, 200.0)));

    let picked = s.color_pick(Vec2::new(160.0, 200.0), s.viewport).unwrap();
    assert!(red(picked[&Channel::BaseColor]));
    dab(&mut s, 240.0, 200.0);
    execute(&mut s, "view.solo", &json!({"layer": null})).unwrap();
    s.update();
    assert!(red(color_at(&s, 240.0, 200.0)));
}
