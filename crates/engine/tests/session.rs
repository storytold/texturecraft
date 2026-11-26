use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, MIN_RESOLUTION};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, command_specs, execute};

fn session() -> Session {
    session_at(256)
}

fn session_at(texture_size: u32) -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, texture_size).unwrap();
    s.viewport = texturecraft_engine::geom::glam::Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    s
}

#[test]
fn active_paint_color_survives_saving_and_reopening_a_project() {
    let mut s = session();
    let default_color = s.tool.values[&Channel::BaseColor];
    let red = [1.0, 0.0, 0.0];
    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": red})).unwrap();
    assert_eq!(s.tool.values[&Channel::BaseColor], red);

    let bytes = s.to_project_bytes().unwrap();
    let reopened = Session::from_project_bytes(&bytes).unwrap();
    assert_eq!(reopened.tool.values[&Channel::BaseColor], red);
    assert_eq!(reopened.doc.project, s.doc.project);

    let mut older = s.project_file();
    older.project.paint_color = None;
    let bytes = texturecraft_formats::save_project_bytes(&older).unwrap();
    let reopened = Session::from_project_bytes(&bytes).unwrap();
    assert_eq!(reopened.tool.values[&Channel::BaseColor], default_color);
}

#[test]
fn scripted_stroke_paints_and_undo_restores() {
    let mut s = session();
    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 20, "hardness": 90})).unwrap();
    let r = execute(&mut s, "tool.stroke", &json!({"points": [[180, 200], [220, 200]]})).unwrap();
    assert!(r["stamps"].as_u64().unwrap() > 1);
    // The texel under the screen center is now red.
    let hit = s.pick(texturecraft_engine::geom::glam::Vec2::new(200.0, 200.0), s.viewport).unwrap();
    let res = 256.0;
    let (x, y) = ((hit.uv.x * res) as u32, (hit.uv.y * res) as u32);
    let v = s.caches[0].composite.value(Channel::BaseColor, x, y);
    assert!(v[0] > 0.9 && v[1] < 0.1, "{v:?}");
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    s.update();
    let v = s.caches[0].composite.value(Channel::BaseColor, x, y);
    assert!((v[0] - 0.5).abs() < 0.01, "{v:?}");
    execute(&mut s, "edit.redo", &Value::Null).unwrap();
    s.update();
    assert!(s.caches[0].composite.value(Channel::BaseColor, x, y)[0] > 0.9);
}

#[test]
fn layers_masks_materials_and_bake() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({"color": "#336699", "roughness": 0.2})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    execute(&mut s, "effect.add.generator", &json!({"generator": "position", "level": 0.5})).unwrap();
    execute(&mut s, "bake.run", &json!({"resolution": 128, "rays": 8})).unwrap();
    execute(&mut s, "material.apply", &json!({"material": "steel_painted_worn"})).unwrap();
    execute(&mut s, "layer.group", &Value::Null).unwrap();
    execute(&mut s, "layer.duplicate", &Value::Null).unwrap();
    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    assert!(inspect["layers"].as_array().unwrap().len() >= 3, "{inspect}");
    s.doc.project.validate().unwrap();
    s.update();
    for _ in 0..7 {
        execute(&mut s, "edit.undo", &Value::Null).unwrap();
    }
    assert_eq!(s.doc.project.walk(0).len(), 1, "back to the initial paint layer");
}

#[test]
fn generator_offset_is_settable_and_clamped() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    let r = execute(&mut s, "effect.add.generator", &json!({"generator": "position", "offset": 0.25})).unwrap();
    let eff = r["effect"].as_u64().unwrap();
    let offset = |s: &Session| {
        let layer = s.doc.project.layers.values().find(|l| l.mask.is_some()).unwrap();
        layer
            .mask
            .as_ref()
            .unwrap()
            .effects
            .iter()
            .find_map(|e| match &e.kind {
                texturecraft_engine::core::EffectKind::Generator(g) => Some(g.offset),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(offset(&s), 0.25);
    execute(&mut s, "effect.set", &json!({"effect": eff, "offset": 5.0})).unwrap();
    assert_eq!(offset(&s), 1.0, "clamped to 1");
}

fn generator_offset_of(s: &Session) -> f32 {
    let layer = s.doc.project.layers.values().find(|l| l.mask.is_some()).unwrap();
    layer
        .mask
        .as_ref()
        .unwrap()
        .effects
        .iter()
        .find_map(|e| match &e.kind {
            texturecraft_engine::core::EffectKind::Generator(g) => Some(g.offset),
            _ => None,
        })
        .unwrap()
}

#[test]
fn generator_offset_survives_save_and_reopen_and_undo_restores_it() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    let r = execute(&mut s, "effect.add.generator", &json!({"generator": "position", "offset": 0.25})).unwrap();
    let eff = r["effect"].as_u64().unwrap();
    execute(&mut s, "effect.set", &json!({"effect": eff, "offset": -0.6})).unwrap();
    assert_eq!(generator_offset_of(&s), -0.6);

    let path = std::env::temp_dir().join(format!("tc-generator-offset-{}.tcraft", std::process::id()));
    let path = path.to_str().unwrap();
    execute(&mut s, "file.saveCopy", &json!({"path": path})).unwrap();
    let reopened = texturecraft_engine::file_cmds::open_path(path);
    let _ = std::fs::remove_file(path);
    assert_eq!(generator_offset_of(&reopened.unwrap()), -0.6, "the offset is saved and read back");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(generator_offset_of(&s), 0.25, "undo restores the previous offset");
}

#[test]
fn generator_offset_works_alongside_curvature_mode_and_sizes() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    let r = execute(&mut s, "effect.add.generator", &json!({"generator": "curvature", "mode": "cavities", "offset": 0.5})).unwrap();
    let eff = r["effect"].as_u64().unwrap();
    assert_eq!(generator_offset_of(&s), 0.5);

    execute(&mut s, "effect.set", &json!({"effect": eff, "offset": -0.5, "sizes": [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5]})).unwrap();
    assert_eq!(generator_offset_of(&s), -0.5);
    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(inspect.contains(r#""offset":-0.5"#) && inspect.contains(r#""mode":"Cavities""#), "{inspect}");
    assert!(inspect.contains(r#""sizes":[0.0,0.0,0.0,1.0,0.0,0.0,0.5]"#), "{inspect}");
}

#[test]
fn layer_rename_and_undo() {
    let mut s = session();
    let layer_id = s.selected_layer().unwrap();
    let orig_name = s.doc.project.layer(layer_id).unwrap().name.clone();
    execute(&mut s, "layer.rename", &json!({"name": "Custom Paint"})).unwrap();
    assert_eq!(s.doc.project.layer(layer_id).unwrap().name, "Custom Paint");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert_eq!(s.doc.project.layer(layer_id).unwrap().name, orig_name);

    execute(&mut s, "edit.redo", &Value::Null).unwrap();
    assert_eq!(s.doc.project.layer(layer_id).unwrap().name, "Custom Paint");
}

#[test]
fn undo_and_redo_jump_several_steps() {
    let mut s = session();
    let layer = s.selected_layer().unwrap();
    let name = |s: &Session| s.doc.project.layer(layer).unwrap().name.clone();
    let first = name(&s);
    for n in ["A", "B", "C"] {
        execute(&mut s, "layer.rename", &json!({"name": n})).unwrap();
    }
    let base = s.history_labels().len() - 3;
    // Two back to "A": the undone steps wait to be redone, next first.
    assert_eq!(execute(&mut s, "edit.undo", &json!({"steps": 2})).unwrap(), json!({"undone": "Rename layer", "steps": 2}));
    assert_eq!(name(&s), "A");
    assert_eq!(s.redo_labels().len(), 2);
    assert_eq!(execute(&mut s, "edit.history", &Value::Null).unwrap()["redo"].as_array().unwrap().len(), 2);
    // Forward past the end stops at "C"; back past the start stops at the first state.
    assert_eq!(execute(&mut s, "edit.redo", &json!({"steps": 99})).unwrap()["steps"], 2);
    assert_eq!(name(&s), "C");
    let r = execute(&mut s, "edit.undo", &json!({"steps": 1_000_000})).unwrap();
    assert_eq!(r["steps"].as_u64().unwrap() as usize, base + 3);
    assert_eq!(name(&s), first);
    assert!(execute(&mut s, "edit.undo", &json!({"steps": 2})).is_err(), "nothing left to undo");
}

#[test]
fn polygon_fill_and_color_picker() {
    let mut s = session();
    execute(&mut s, "tool.material", &json!({"channel": "roughness", "value": 0.9})).unwrap();
    execute(&mut s, "tool.polygonFill", &Value::Null).unwrap();
    execute(&mut s, "tool.fillMode", &json!({"mode": "mesh"})).unwrap();
    execute(&mut s, "tool.click", &json!({"x": 200, "y": 200})).unwrap();
    execute(&mut s, "tool.colorPicker", &Value::Null).unwrap();
    let r = execute(&mut s, "tool.click", &json!({"x": 200, "y": 200})).unwrap();

    assert!((r["roughness"][0].as_f64().unwrap() - 0.9).abs() < 0.01, "{r}");
}

#[test]
fn color_at_reads_the_surface_without_changing_the_brush() {
    let mut s = session();
    execute(&mut s, "tool.material", &json!({"channel": "roughness", "value": 0.9})).unwrap();
    execute(&mut s, "tool.polygonFill", &Value::Null).unwrap();
    execute(&mut s, "tool.fillMode", &json!({"mode": "mesh"})).unwrap();
    execute(&mut s, "tool.click", &json!({"x": 200, "y": 200})).unwrap();
    execute(&mut s, "tool.material", &json!({"channel": "roughness", "value": 0.2})).unwrap();
    let viewport = s.viewport;

    let read = s.color_at(Vec2::new(200.0, 200.0), viewport).unwrap();

    let surface = read.get(&Channel::Roughness).unwrap();

    assert!((surface[0] - 0.9).abs() < 0.01, "{surface:?}");

    let brush = s.tool.values.get(&Channel::Roughness).unwrap();

    assert!((brush[0] - 0.2).abs() < 0.01, "the brush keeps its own value: {brush:?}");
}

#[test]
fn polygon_fill_boxes_survive_hostile_params() {
    let hostile = [
        json!({"x0": 1e30, "y0": -1e30, "x1": f64::MAX, "y1": 0, "viewport": [1e9, 1e9]}),
        json!({"x0": 0, "y0": 0, "x1": 0, "y1": 0, "viewport": [16, 16]}),
        json!({"uv0": [1e30, -1e30], "uv1": [0, 0]}),
        json!({"uv0": [0.5], "uv1": "x", "uv": [f64::MIN, 2]}),
        json!({"hitTest": "nope", "visibleOnly": "yes", "mode": 7}),
    ];
    for visible_only in [true, false] {
        for hit_test in ["intersection", "containedCenter"] {
            let mut session = session();
            execute(&mut session, "tool.polygonFill", &Value::Null).unwrap();
            execute(&mut session, "tool.fillSelection", &json!({"hitTest": hit_test, "visibleOnly": visible_only})).unwrap();
            for params in &hostile {
                let _ = execute(&mut session, "tool.click", params);
                let _ = execute(&mut session, "tool.fillSelection", params);
                session.update();
            }
        }
    }
}

/// Every command must return Ok or Err — never panic — for hostile params.
/// Small texture and bake sizes retain the same command and parameter cases without a full-size bake per case.
#[test]
fn panic_hunt() {
    let hostile = [
        Value::Null,
        json!({}),
        json!({"layer": u64::MAX, "ts": 99, "channel": "nope", "mode": "???", "opacity": f64::MAX}),
        json!({"layer": 1, "index": -5, "delta": i64::MIN, "above": "x", "points": [[f64::MAX, -1e300]], "size": -1e30}),
        json!({"resolution": 0, "maps": ["x"], "rays": u64::MAX, "antialiasing": 3, "color": [1], "value": "nan"}),
        json!({"effect": 0, "x": 1e30, "y": -1e30, "viewport": [0, 0], "mesh": "../../etc", "material": ""}),
    ];
    for spec in command_specs() {
        if spec.id == "project.sample" {
            continue; // slow (bakes); covered elsewhere
        }

        for p in &hostile {
            let mut s = session_at(MIN_RESOLUTION);
            for texture_set in &mut s.doc.project.texture_sets {
                texture_set.bake.common.resolution = texturecraft_engine::core::BakeResolution::R64;
            }
            let _ = execute(&mut s, spec.id, p);
            s.update();
        }
    }

    hunt_filters(&hostile);
}

/// A 4 × 4 gray PNG, the image the hunted decals project
fn gray_png() -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([128, 128, 128, 255])).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// Frame edits, frame and Fill section resets and Transform filter settings on the fills they act on (a planar, a warp, a decal mask and a Transform filter) meet hostile values without panicking; a refused edit changes nothing and adds no undo step
#[test]
fn panic_hunt_frames_and_transforms() {
    let mut s = session();
    let image = s.import_resource_bytes("gray.png", gray_png()).unwrap();
    let planar = execute(&mut s, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap();
    execute(&mut s, "layer.fill.projection", &json!({"layer": planar, "projection": "planar"})).unwrap();
    let warp = execute(&mut s, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap();
    execute(&mut s, "layer.fill.projection", &json!({"layer": warp, "projection": "warp"})).unwrap();
    let decal = execute(&mut s, "layer.add.decal", &json!({"image": image, "channel": "mask", "position": [0, 0, 1], "normal": [0, 0, 1]})).unwrap();
    let mask_effect = decal["maskEffect"].as_u64().unwrap();
    let transform = execute(&mut s, "effect.add.filter", &json!({"filter": "transform", "target": "content"})).unwrap()["effect"].as_u64().unwrap();
    s.update();

    let hostile_values =
        [json!(1e30), json!(-1e30), json!(0), json!(null), json!("x"), json!([]), json!([1e30, -1e30]), json!([0, 0, 0]), json!([-1e30, 1e30, 0, 5])];
    let targets = [json!({"layer": planar}), json!({"layer": warp}), json!({"effect": mask_effect})];
    let keys = ["frameOffset", "frameRotation", "frameScale", "size", "offset", "depth", "rotation", "up"];
    for target in &targets {
        for key in keys {
            for value in &hostile_values {
                let mut params = target.clone();
                params[key] = value.clone();
                hunt_refused_changes_nothing(&mut s, "layer.decal.set", &params);
            }
        }
    }

    for target in &targets {
        for command in ["layer.fill.resetFrame", "layer.fill.resetProjection"] {
            let _ = execute(&mut s, command, target);
            s.update();
        }
    }

    let transform_keys = ["repeatMode", "flipMode", "scaling", "offset", "rotation", "tileSafeRotation"];
    for key in transform_keys {
        for value in &hostile_values {
            let mut params = json!({"effect": transform});
            params[key] = value.clone();
            hunt_refused_changes_nothing(&mut s, "effect.set", &params);
        }
    }
}

/// Run `command` with `params` and render; refused, it must leave the project and the undo history as they were
fn hunt_refused_changes_nothing(s: &mut Session, command: &str, params: &Value) {
    let (before, steps) = (s.doc.project.clone(), s.history_labels().len());
    if execute(s, command, params).is_err() {
        assert!(s.doc.project == before && s.history_labels().len() == steps, "a refused {command} {params} changes nothing");
    }
    s.update();
}

/// The number parameters library filters read, all set at once to one hostile value
const FILTER_NUMBER_KEYS: &[&str] = &[
    "radius",
    "intensity",
    "amount",
    "level",
    "hue",
    "saturation",
    "lightness",
    "contrast",
    "luminosity",
    "levels",
    "min",
    "max",
    "position",
    "range",
    "offset",
    "multiply",
    "scale",
    "angle",
    "width",
    "distance",
    "smoothing",
    "size",
    "spread",
    "opacity",
    "blur",
    "balance",
    "quality",
    "sourceBlur",
    "sourceBalance",
    "sourceContrast",
    "sourceTiling",
    "tolerance",
    "softness",
    "light",
    "ambient",
    "occlusion",
    "azimuth",
    "elevation",
    "horizontalRotation",
    "verticalRotation",
    "exposure",
    "heightIntensity",
    "ambientOcclusion",
    "specularOcclusion",
    "pixelsAmount",
    "curveOffset",
    "curveShape",
    "maskThreshold",
    "distanceMapMultiplier",
    "threshold",
    "axisXOffset",
    "axisYOffset",
    "surfaceSize",
    "heightDepth",
    "normalIntensity",
    "chroma",
    "luma",
    "red",
    "green",
    "blue",
    "alpha",
    "tensorSmoothness",
    "anisotropy",
    "sharpness",
    "smoothness",
    "rotation",
    "scaling",
    "diffuseCavity",
    "specularAo",
    "specularCavity",
    "cavitySmoothness",
    "edgesIntensity",
    "edgesSmoothness",
    "dielectricReflectance",
    "heightToNormal",
];

/// The objects library filters read numbers from (Baked Lighting Stylized's two lights)
const FILTER_LIGHT_KEYS: &[&str] = &["light1", "light2"];

/// Every filter number parameter set to `number`, with seeds, switches and choices at their ends
fn filter_extreme(number: f64) -> Value {
    let mut params = json!({"seed": u64::MAX, "wrap": true, "applyToAlpha": true, "invert": true, "directions": 4, "divider": 1000});
    for key in FILTER_NUMBER_KEYS {
        params[*key] = json!(number);
    }
    for key in FILTER_LIGHT_KEYS {
        params[*key] = json!({"azimuth": number, "elevation": number, "intensity": number});
    }
    params
}

/// Every library filter added on the content and on a mask with `hostile` and extreme parameters, edited to extremes, and rendered; then swapped to the same way
fn hunt_filters(hostile: &[Value]) {
    let extremes: Vec<Value> = [3.0e38, -3.0e38, 0.0].into_iter().map(filter_extreme).collect();

    for def in texturecraft_engine::filter_lib::FILTERS {
        let mut s = session_at(MIN_RESOLUTION);
        execute(&mut s, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1], "roughness": 0.4})).unwrap();
        for target in ["content", "mask"] {
            if target == "mask" {
                execute(&mut s, "mask.add.black", &Value::Null).unwrap();
            }

            for given in hostile.iter().chain(&extremes) {
                let mut params = if given.is_object() { given.clone() } else { json!({}) };
                params["filter"] = json!(def.id);
                params["target"] = json!(target);
                let added = execute(&mut s, "effect.add.filter", &params).ok().and_then(|reply| reply["effect"].as_u64());
                s.update();

                // Extreme values are clamped, not refused, so every filter meets them on the content
                if target == "content" && extremes.contains(given) {
                    assert!(added.is_some(), "{} takes extreme values: {params}", def.id);
                }
                if let Some(effect) = added {
                    for edit in &extremes {
                        let mut edit = edit.clone();
                        edit["effect"] = json!(effect);

                        // Each accepted value reaches the compositor before the next edit replaces it
                        if execute(&mut s, "effect.set", &edit).is_ok() {
                            s.update();
                        }
                    }
                }
            }
        }
    }

    hunt_filter_swaps(hostile, &extremes);
}

/// An empty filter effect on the content and on a mask swapped through `effect.set` to every library filter with each of `hostile` and `extremes`, and rendered
fn hunt_filter_swaps(hostile: &[Value], extremes: &[Value]) {
    let mut s = session_at(MIN_RESOLUTION);
    execute(&mut s, "layer.add.fill", &json!({"color": [0.8, 0.3, 0.1], "roughness": 0.4})).unwrap();
    for target in ["content", "mask"] {
        if target == "mask" {
            execute(&mut s, "mask.add.black", &Value::Null).unwrap();
        }
        let added = execute(&mut s, "effect.add.filter", &json!({"filter": "none", "target": target})).unwrap();
        let effect = added["effect"].as_u64().unwrap();

        for def in texturecraft_engine::filter_lib::FILTERS {
            for given in hostile.iter().chain(extremes) {
                let mut params = if given.is_object() { given.clone() } else { json!({}) };
                params["effect"] = json!(effect);
                params["filter"] = json!(def.id);

                // Emptied first, so each swap builds the filter anew rather than editing it in place
                execute(&mut s, "effect.set", &json!({"effect": effect, "filter": "none"})).unwrap();
                let _ = execute(&mut s, "effect.set", &params);
                s.update();
            }
        }
    }
}

#[test]
fn an_srgb_hex_colour_matches_its_linear_rgb_value() {
    let base_color = |color: Value| {
        let mut s = session();
        execute(&mut s, "layer.add.fill", &json!({ "color": color })).unwrap();
        s.update();
        s.caches[0].composite.value(Channel::BaseColor, 128, 128)
    };
    let hex = base_color(json!("#c8102e"));
    let linear = base_color(json!([0.5776, 0.0052, 0.0273]));
    assert!(hex.iter().zip(&linear).all(|(a, b)| (a - b).abs() < 2e-3), "{hex:?} {linear:?}");
    let digits = base_color(json!([0.7843, 0.0627, 0.1804]));
    assert!(digits[0] - hex[0] > 0.1, "{digits:?} {hex:?}");
}

#[test]
fn copy_paste_flatten_and_resize_keep_the_look() {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
    s.viewport = texturecraft_engine::geom::glam::Vec2::new(400.0, 400.0);
    execute(&mut s, "layer.add.fill", &json!({"color": [0.9, 0.1, 0.1], "roughness": 0.3})).unwrap();
    execute(&mut s, "mask.add.black", &Value::Null).unwrap();
    execute(&mut s, "effect.add.fill", &json!({"procedural": "checker", "scale": 4})).unwrap();
    s.update();
    let before: Vec<[f32; 3]> = (0..256).step_by(9).map(|x| s.caches[0].composite.value(Channel::BaseColor, x, 100)).collect();
    // Flatten keeps the composite identical (within 8-bit-ish tolerance) and makes a paint layer.
    let id = s.selected_layer().unwrap();
    execute(&mut s, "layer.flatten", &json!({"layer": id.0})).unwrap();
    s.update();
    let after: Vec<[f32; 3]> = (0..256).step_by(9).map(|x| s.caches[0].composite.value(Channel::BaseColor, x, 100)).collect();
    for (a, b) in before.iter().zip(&after) {
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() < 2e-3, "{a:?} {b:?}");
        }
    }
    assert!(matches!(s.doc.project.layer(id).unwrap().kind, texturecraft_engine::core::LayerKind::Paint));
    // Copy into the other texture set (different resolution).
    execute(&mut s, "textureSet.resolution", &json!({"ts": 1, "resolution": 128})).unwrap();
    execute(&mut s, "layer.copy", &Value::Null).unwrap();
    execute(&mut s, "textureSet.select", &json!({"ts": 1})).unwrap();
    let r = execute(&mut s, "layer.paste", &Value::Null).unwrap();
    let pasted = texturecraft_engine::core::LayerId(r["layer"].as_u64().unwrap());
    assert_eq!(s.doc.pixels.layer(pasted, Channel::BaseColor).unwrap().width(), 128);
    // Resizing resamples instead of clearing.
    execute(&mut s, "textureSet.resolution", &json!({"ts": 0, "resolution": 512})).unwrap();
    assert_eq!(s.doc.pixels.layer(id, Channel::BaseColor).unwrap().width(), 512);
    s.update();
    let big = s.caches[0].composite.value(Channel::BaseColor, 2 * 9 * 5, 200);
    let small = before[5];
    assert!((big[0] - small[0]).abs() < 0.2, "{big:?} {small:?}");
    execute(&mut s, "layer.instantiateAll", &json!({"layer": id.0})).unwrap();
    s.doc.project.validate().unwrap();
}

#[test]
fn image_resources_fill_layers_and_round_trip() {
    let mut s = session();
    let mut png = Vec::new();
    let img = image::RgbaImage::from_fn(8, 8, |_, _| image::Rgba([255, 0, 0, 255]));
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = s.import_resource_bytes("C:/x/red bricks.png", png).unwrap();
    assert_eq!(name, "red_bricks.png");
    let r = execute(&mut s, "layer.add.image", &json!({"name": name})).unwrap();
    assert!(r["layer"].as_u64().is_some());
    s.update();
    let v = s.caches[0].composite.value(Channel::BaseColor, 128, 128);
    assert!(v[0] > 0.95 && v[1] < 0.05, "{v:?}");
    let bytes = s.to_project_bytes().unwrap();
    let back = Session::from_project_bytes(&bytes).unwrap();
    assert_eq!(back.doc.resources.len(), 1);
    assert_eq!(back.images.len(), 1);
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(s.images.is_empty(), "undoing the import forgets the decoded image");
    assert!(s.import_resource_bytes("bad.png", vec![1, 2, 3]).is_err());
}

#[test]
fn image_resource_lookup_accepts_extension_and_stem() {
    let mut s = session();

    // No resources imported yet
    let err = execute(&mut s, "layer.add.image", &json!({"name": "missing"})).unwrap_err();
    assert!(err.to_string().contains("no resource `missing` (no resources imported: use resource.import)"));

    let mut png = Vec::new();
    let img = image::RgbaImage::from_fn(4, 4, |_, _| image::Rgba([0, 255, 0, 255]));
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.import_resource_bytes("tile.png", png.clone()).unwrap();

    // Lookup by stem
    let r = execute(&mut s, "layer.add.image", &json!({"name": "tile"})).unwrap();
    assert!(r["layer"].as_u64().is_some());

    // Lookup by full name
    let r2 = execute(&mut s, "layer.add.image", &json!({"name": "tile.png"})).unwrap();
    assert!(r2["layer"].as_u64().is_some());

    // Missing resource lists available
    let err = execute(&mut s, "layer.add.image", &json!({"name": "wood"})).unwrap_err();
    assert!(err.to_string().contains("no resource `wood` (available: tile.png)"));

    // Ambiguous stem resolution
    s.import_resource_bytes("tile.jpg", png).unwrap();
    let err = execute(&mut s, "layer.add.image", &json!({"name": "tile"})).unwrap_err();
    assert!(err.to_string().contains("ambiguous resource `tile`"));
}

#[test]
fn image_resource_lookup_ignores_linked_packages() {
    use std::sync::Arc;
    use texturecraft_engine::core::{LayerId, LayerKind, Source};

    let mut s = session();
    s.doc.resources.insert("wood.mcraft".into(), Arc::new(b"package bytes".to_vec()));
    s.sync_images();
    assert!(!s.images.contains_key("wood.mcraft"));

    let err = execute(&mut s, "layer.add.image", &json!({"name": "wood"})).unwrap_err();
    assert!(err.to_string().contains("no resources imported"), "{err}");

    let err = execute(&mut s, "layer.add.image", &json!({"name": "wood.mcraft"})).unwrap_err();
    assert!(err.to_string().contains("no resource `wood.mcraft` (no resources imported"), "{err}");

    let mut png = Vec::new();
    let img = image::RgbaImage::from_fn(4, 4, |_, _| image::Rgba([0, 255, 0, 255]));
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.import_resource_bytes("wood.png", png).unwrap();

    let err = execute(&mut s, "layer.add.image", &json!({"name": "missing"})).unwrap_err();
    assert!(err.to_string().contains("no resource `missing` (available: wood.png)"), "{err}");

    let res = execute(&mut s, "layer.add.image", &json!({"name": "wood"})).unwrap();
    let id = LayerId(res["layer"].as_u64().unwrap());
    let LayerKind::Fill(fill) = &s.doc.project.layer(id).unwrap().kind else { panic!("expected fill layer") };
    assert_eq!(fill.sources.get(&Channel::BaseColor), Some(&Source::image("wood.png")));
}

#[test]
fn brush_presets_apply_save_and_load() {
    let mut s = session();
    let list = execute(&mut s, "brush.preset.list", &Value::Null).unwrap();
    assert!(list.as_array().unwrap().len() >= 12);
    execute(&mut s, "tool.brush", &json!({"size": 30})).unwrap();
    let b = execute(&mut s, "brush.preset.apply", &json!({"preset": "Scratches"})).unwrap();
    assert_eq!((b["alpha"]["library"].as_str(), b["followPath"].as_bool()), (Some("scratches"), Some(true)));
    assert!((b["size"].as_f64().unwrap() - 30.0).abs() < 1e-3, "presets keep the size");
    assert!(execute(&mut s, "tool.stroke", &json!({"points": [[160, 200], [240, 200]]})).unwrap()["stamps"].as_u64().unwrap() > 0);
    // Saved and loaded back onto another brush, the stroke returns and the size stays.
    let path = std::env::temp_dir().join(format!("tc-brush-{}.tcbrush", std::process::id()));
    let path = path.to_str().unwrap();
    execute(&mut s, "brush.preset.save", &json!({"path": path})).unwrap();
    execute(&mut s, "brush.preset.apply", &json!({"preset": "basic_soft"})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 10})).unwrap();
    let b = execute(&mut s, "brush.preset.load", &json!({"path": path})).unwrap();
    assert_eq!(b["alpha"]["library"], "scratches");
    assert!((b["size"].as_f64().unwrap() - 10.0).abs() < 1e-3);
    std::fs::write(path, b"{}").unwrap();
    assert!(execute(&mut s, "brush.preset.load", &json!({"path": path})).is_err());
    let _ = std::fs::remove_file(path);
    for bad in [json!({}), json!({"preset": "nope"})] {
        assert!(execute(&mut s, "brush.preset.apply", &bad).is_err());
    }
}

#[test]
fn text_becomes_a_brush_alpha_and_a_decal() {
    let mut s = session();
    let fonts = execute(&mut s, "text.fonts", &Value::Null).unwrap();
    assert_eq!(fonts["builtIn"][0], "Ubuntu Light");
    let r = execute(&mut s, "resource.text", &json!({"text": "TEXT\nCRAFT", "font": "ubuntu light", "size": 64})).unwrap();
    let name = r["name"].as_str().unwrap().to_string();
    assert_eq!(name, "Text_TEXT.png");
    assert!(r["width"].as_u64().unwrap() > r["height"].as_u64().unwrap() / 2, "two short lines");
    // White with the glyphs in alpha.
    let img = s.images[&name].clone();
    assert!(img.rgba.iter().any(|t| t[3] > 0.99) && img.rgba.iter().any(|t| t[3] == 0.0) && img.rgba.iter().all(|t| t[0] == 1.0));
    // As the brush alpha it paints (keeping its proportions), and it places as a decal.
    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 40, "alpha": {"image": name}})).unwrap();
    assert!(execute(&mut s, "tool.click", &json!({"x": 200, "y": 200})).is_ok());
    assert!(execute(&mut s, "layer.add.decal", &json!({"image": name})).unwrap()["layer"].as_u64().is_some());
    for bad in [json!({}), json!({"text": "  "}), json!({"text": "x", "font": "No Such Font"})] {
        assert!(execute(&mut s, "resource.text", &bad).is_err(), "{bad}");
    }
}

#[test]
fn projection_tool_paints_the_stencil_colors() {
    let mut s = session();
    let mut png = Vec::new();
    let img = image::RgbaImage::from_fn(16, 16, |x, _| if x < 8 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 255, 0, 255]) });
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let name = s.import_resource_bytes("split.png", png).unwrap();
    execute(&mut s, "tool.projection", &Value::Null).unwrap();
    // Without a stencil the tool explains what's missing.
    assert!(execute(&mut s, "tool.stroke", &json!({"points": [[150, 200]]})).is_err());
    execute(&mut s, "tool.stencil", &json!({"image": name, "scale": 1.0, "tile": true})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 6, "hardness": 90})).unwrap();
    for x in [150.0f32, 250.0] {
        execute(&mut s, "tool.stroke", &json!({"points": [[x, 200.0]]})).unwrap();
    }
    let vp = s.viewport;
    let at = |s: &Session, x: f32| {
        let hit = s.pick(texturecraft_engine::geom::glam::Vec2::new(x, 200.0), vp).unwrap();
        s.caches[0].composite.value(Channel::BaseColor, (hit.uv.x * 256.0) as u32, (hit.uv.y * 256.0) as u32)
    };
    let left = at(&s, 150.0);
    let right = at(&s, 250.0);
    assert!(left[0] > 0.8 && left[1] < 0.2, "left {left:?}");
    assert!(right[1] > 0.8 && right[0] < 0.2, "right {right:?}");
}

#[test]
fn smart_materials_save_and_load_with_pixels() {
    let mut s = Session::new_project("t", primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
    s.viewport = texturecraft_engine::geom::glam::Vec2::new(400.0, 400.0);
    let group = texturecraft_engine::materials::apply(&mut s, "steel_painted_worn").unwrap();
    let before = s.doc.project.walk(0).len();
    let bytes = s.smart_material_bytes(group, "My Steel").unwrap();
    execute(&mut s, "textureSet.select", &json!({"ts": 1})).unwrap();
    let id = s.add_smart_material_bytes(&bytes).unwrap();
    assert_eq!(s.doc.project.layer(id).unwrap().name, "My Steel");
    let walked = s.doc.project.walk(1).len();
    assert_eq!(walked, 1 + (before - 1), "the folder and its children arrive");
    assert!(s.add_smart_material_bytes(b"junk").is_err());
    s.doc.project.validate().unwrap();
}

/// Strokes drawn frame by frame (paint, erase, a mask) commit their staged tiles, which then
/// settle at low priority to exactly what a full recomposite gives; undo restores.
#[test]
fn committed_strokes_settle_to_the_full_composite() {
    use texturecraft_engine::geom::glam::Vec2;
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 2048).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    execute(&mut s, "layer.add.fill", &json!({"color": [0.2, 0.5, 0.3], "roughness": 0.7})).unwrap();
    execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 25, "hardness": 70, "opacity": 0.8})).unwrap();
    s.update();
    let full = |s: &mut Session| {
        s.caches[0].invalidate_all();
        s.update();
        s.caches[0].composite.clone()
    };
    let original = full(&mut s);
    for step in ["paint", "eraser", "mask"] {
        match step {
            "eraser" => drop(execute(&mut s, "tool.select", &json!({"tool": "eraser"})).unwrap()),
            "mask" => {
                execute(&mut s, "tool.select", &json!({"tool": "paint"})).unwrap();
                execute(&mut s, "mask.add.white", &Value::Null).unwrap();
                s.target = texturecraft_engine::StackTarget::Mask;
                s.tool.mask_value = 0.2;
            }
            _ => {}
        }
        s.update();
        s.begin_stroke(Vec2::new(120.0, 200.0), s.viewport, 1.0).unwrap();
        for i in 1..30 {
            s.continue_stroke(Vec2::new(120.0 + i as f32 * 6.0, 200.0 + (i as f32 * 0.4).sin() * 40.0), 1.0);
            s.update();
        }
        s.end_stroke().unwrap();
        // Nothing was left dirty: the committed tiles only settle, a slice per budgeted update.
        assert!(s.caches[0].dirty.is_empty(), "{step}");
        let settling = s.caches[0].settle.len();
        assert!(settling > 32, "{step}: {settling} tiles settle");
        let t = std::cell::Cell::new(0.0);
        let clock = || {
            t.set(t.get() + 2.0);
            t.get()
        };
        s.update_for(30.0, clock);
        let left = s.pending_tiles();
        assert!(left > 0 && left < settling, "{step}: {left} of {settling} left after one slice");
        s.update();
        let settled = s.caches[0].composite.clone();
        assert_eq!(settled, full(&mut s), "{step}: settled tiles match a full recomposite");
    }
    // Mask stroke, white mask, eraser and paint strokes.
    for _ in 0..4 {
        execute(&mut s, "edit.undo", &Value::Null).unwrap();
    }
    assert_eq!(full(&mut s), original);
}

/// A resize prepared from a snapshot and applied later matches the command, keeps edits made
/// while it ran, and undoes in one step.
#[test]
fn background_resizes_match_the_command_and_keep_later_edits() {
    use texturecraft_engine::bake::Progress;
    let stroke = |s: &mut Session, y: u32| {
        execute(s, "tool.stroke", &json!({"points": [[150, y], [250, y]]})).unwrap();
    };
    let (mut a, mut b) = (session(), session());
    for s in [&mut a, &mut b] {
        execute(s, "tool.brush", &json!({"size": 10})).unwrap();
        stroke(s, 200);
    }
    execute(&mut a, "textureSet.resolution", &json!({"resolution": 512})).unwrap();
    let task = b.resize_task(0, 512).unwrap();
    let progress = Progress::new();
    let out = task.run(&progress).unwrap();
    assert_eq!(progress.fraction(), 1.0);
    assert_eq!(out.target(), (0, 512));
    b.apply_resize(out).unwrap();
    assert_eq!(a.doc.pixels.layers, b.doc.pixels.layers);
    a.update();
    b.update();
    assert_eq!(a.caches[0].composite, b.caches[0].composite);
    assert_eq!(b.caches[0].size, (512, 512));
    // A stroke painted while the resize ran is kept (the resize runs again on apply).
    let out = b.resize_task(0, 1024).unwrap().run(&Progress::new()).unwrap();
    stroke(&mut b, 260);
    let painted = |s: &Session| s.doc.pixels.layers.values().map(|i| i.allocated_count()).sum::<usize>();
    let before = painted(&b);
    b.apply_resize(out).unwrap();
    assert_eq!(b.doc.project.texture_sets[0].resolution, 1024);
    stroke(&mut a, 260);
    execute(&mut a, "textureSet.resolution", &json!({"resolution": 1024})).unwrap();
    assert_eq!(a.doc.pixels.layers, b.doc.pixels.layers);
    assert!(painted(&b) > before);
    assert_eq!(b.undo_label(), Some("Texture set size"));
    execute(&mut b, "edit.undo", &Value::Null).unwrap();
    assert_eq!((b.doc.project.texture_sets[0].resolution, painted(&b)), (512, before));
    b.update();
    assert_eq!(b.caches[0].size, (512, 512));
}

/// A 2048 sphere set with a fill layer and an empty paint layer, a small brush, and the
/// oracle: the composite after recompositing every covered tile from scratch.
fn painted_set() -> Session {
    use texturecraft_engine::geom::glam::Vec2;
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 2048).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    execute(&mut s, "layer.add.fill", &json!({"color": [0.2, 0.5, 0.3], "roughness": 0.7})).unwrap();
    execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 12, "hardness": 70, "opacity": 0.8})).unwrap();
    // Seed the image so later strokes change existing tiles
    execute(&mut s, "tool.stroke", &json!({"points": [[150, 160], [170, 170]]})).unwrap();
    s.update();
    s
}

fn full(s: &mut Session) -> texturecraft_engine::compose::Composite {
    s.caches[0].invalidate_all();
    s.update();
    s.caches[0].composite.clone()
}

/// Undo and redo of `change` queue the right tiles: `Some(max)` fewer than `max` of them, matching
/// a full recomposite once updated; `None` every covered tile.
fn check_undo_redo(s: &mut Session, change: impl FnOnce(&mut Session), limit: Option<usize>) {
    change(s);
    s.update();
    for cmd in ["edit.undo", "edit.redo"] {
        execute(s, cmd, &Value::Null).unwrap();
        let (dirty, covered) = (s.caches[0].dirty.len(), s.caches[0].covered_tiles.len());
        match limit {
            Some(max) => assert!(0 < dirty && dirty < max, "{cmd}: {dirty} of {covered} tiles dirty"),
            None => assert_eq!(dirty, covered, "{cmd}"),
        }
        s.update();
        let settled = s.caches[0].composite.clone();
        assert_eq!(settled, full(s), "{cmd}: matches a full recomposite");
    }
}

#[test]
fn undo_recomposites_only_changed_tiles() {
    let mut s = painted_set();
    let stroke = |s: &mut Session| drop(execute(s, "tool.stroke", &json!({"points": [[150, 200], [190, 210]]})).unwrap());
    // The stroke covers a handful of the set's 1024 tiles
    check_undo_redo(&mut s, stroke, Some(64));
}

#[test]
fn undo_with_a_blur_above_recomposites_everything() {
    use texturecraft_engine::geom::glam::Vec2;
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 2048).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    execute(&mut s, "layer.add.fill", &json!({"color": [0.2, 0.5, 0.3], "roughness": 0.7})).unwrap();
    execute(&mut s, "mask.add.white", &Value::Null).unwrap();
    execute(&mut s, "effect.add.filter", &json!({"filter": "blur", "radius": 8})).unwrap();
    execute(&mut s, "layer.add.paint", &Value::Null).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 12, "hardness": 70, "opacity": 0.8})).unwrap();
    execute(&mut s, "tool.stroke", &json!({"points": [[150, 160], [170, 170]]})).unwrap();
    s.update();
    let stroke = |s: &mut Session| drop(execute(s, "tool.stroke", &json!({"points": [[150, 200], [190, 210]]})).unwrap());
    check_undo_redo(&mut s, stroke, None);
}

#[test]
fn polygon_fill_recomposites_only_changed_tiles() {
    let mut s = painted_set();
    execute(&mut s, "tool.material", &json!({"channel": "roughness", "value": 0.1})).unwrap();
    execute(&mut s, "tool.stroke", &json!({"points": [[150, 240], [170, 250]]})).unwrap();
    execute(&mut s, "tool.polygonFill", &Value::Null).unwrap();
    s.update();
    let tris = s.polygon_fill(texturecraft_engine::geom::glam::Vec2::new(200.0, 200.0), s.viewport).unwrap();
    assert!(tris > 0);
    let (dirty, covered) = (s.caches[0].dirty.len(), s.caches[0].covered_tiles.len());
    assert!(0 < dirty && dirty < covered / 4, "{dirty} of {covered} tiles dirty after one triangle");
    s.update();
    let settled = s.caches[0].composite.clone();
    assert_eq!(settled, full(&mut s));
    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    assert!(s.caches[0].dirty.len() < covered / 4);
    s.update();
    let settled = s.caches[0].composite.clone();
    assert_eq!(settled, full(&mut s));
}

#[test]
fn undo_of_a_layer_change_recomposites_everything() {
    let mut s = painted_set();
    let change = |s: &mut Session| drop(execute(s, "layer.blend", &json!({"channel": "all", "opacity": 0.5})).unwrap());
    check_undo_redo(&mut s, change, None);
}

#[test]
fn curvature_generators_take_a_mode_and_detail_sizes() {
    let mut state = session();
    execute(&mut state, "layer.add.fill", &json!({})).unwrap();
    execute(&mut state, "mask.add.black", &Value::Null).unwrap();
    let added = execute(&mut state, "effect.add.generator", &json!({"generator": "curvature", "mode": "cavities"})).unwrap();
    let effect = added["effect"].clone();

    execute(&mut state, "effect.set", &json!({"effect": effect, "sizes": [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5]})).unwrap();
    let inspect = execute(&mut state, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(inspect.contains(r#""mode":"Cavities""#) && inspect.contains(r#""sizes":[0.0,0.0,0.0,1.0,0.0,0.0,0.5]"#), "{inspect}");

    let too_few = execute(&mut state, "effect.set", &json!({"effect": effect, "sizes": [1.0, 0.5]}));
    assert!(too_few.is_err(), "sizes takes one weight per detail size");
    let not_numbers = execute(&mut state, "effect.set", &json!({"effect": effect, "sizes": [1.0, "a", 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]}));
    assert!(not_numbers.is_err(), "a size that isn't a number is rejected, not skipped");
    let unknown = execute(&mut state, "effect.set", &json!({"effect": effect, "mode": "ridges"}));
    assert!(unknown.is_err(), "unknown modes are rejected");
    let wrong_type = execute(&mut state, "effect.set", &json!({"effect": effect, "mode": 1, "level": 0.1}));
    assert!(wrong_type.is_err(), "a mode must be a string and an invalid edit must be rejected");
    let dirt = execute(&mut state, "effect.add.generator", &json!({"generator": "dirt", "mode": "edges"}));
    assert!(dirt.is_err(), "only curvature generators have a mode");
    assert_eq!(execute(&mut state, "project.inspect", &Value::Null).unwrap().to_string(), inspect, "failed commands leave the project unchanged");
}

#[test]
fn generators_take_blur_balance_and_contrast() {
    let mut state = session();
    execute(&mut state, "layer.add.fill", &json!({})).unwrap();
    execute(&mut state, "mask.add.black", &Value::Null).unwrap();
    let added = execute(&mut state, "effect.add.generator", &json!({"generator": "ambientOcclusion", "blur": 0.25})).unwrap();
    let effect = added["effect"].clone();

    execute(&mut state, "effect.set", &json!({"effect": effect, "balance": 0.75, "contrast": 2.0})).unwrap();
    let inspect = execute(&mut state, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(inspect.contains(r#""blur":0.25"#) && inspect.contains(r#""balance":0.75"#), "{inspect}");
    assert!(inspect.contains(r#""contrast":1.0"#), "contrast is clamped to 0..1: {inspect}");

    execute(&mut state, "effect.set", &json!({"effect": effect, "resolution": "custom", "customResolution": 512, "resolutionFactor": -9})).unwrap();
    let inspect = execute(&mut state, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(inspect.contains(r#""resolution":{"custom":512,"factor":-3,"mode":"Custom"}"#), "the factor is clamped to 1/8: {inspect}");
    assert!(execute(&mut state, "effect.set", &json!({"effect": effect, "resolution": "huge"})).is_err(), "unknown resolutions are rejected");
}

#[test]
fn a_generator_input_reads_a_uniform_value_or_its_mesh_map_again() {
    let mut state = session();
    execute(&mut state, "layer.add.fill", &json!({})).unwrap();
    execute(&mut state, "mask.add.black", &Value::Null).unwrap();
    let effect = execute(&mut state, "effect.add.generator", &json!({"generator": "dirt"})).unwrap()["effect"].clone();

    execute(&mut state, "effect.set", &json!({"effect": effect, "input": "ambientOcclusion", "inputValue": 0.25})).unwrap();
    let inspect = execute(&mut state, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(inspect.contains(r#""inputs":{"ambientOcclusion":{"uniform":[0.25,0.25,0.25]}}"#), "{inspect}");

    let wrong_input = execute(&mut state, "effect.set", &json!({"effect": effect, "input": "thickness", "inputValue": 0.5}));
    assert!(wrong_input.is_err(), "dirt reads ambient occlusion and curvature, not thickness");
    let missing_image = execute(&mut state, "effect.set", &json!({"effect": effect, "input": "ambientOcclusion", "inputImage": "nothing.png"}));
    assert!(missing_image.is_err(), "the image must be a resource of the project");
    let two_sources = execute(&mut state, "effect.set", &json!({"effect": effect, "input": "ambientOcclusion", "inputValue": 0.5, "inputBaked": true}));
    assert!(two_sources.is_err(), "an input takes one source");

    execute(&mut state, "effect.set", &json!({"effect": effect, "input": "ambientOcclusion", "inputBaked": true})).unwrap();
    let inspect = execute(&mut state, "project.inspect", &Value::Null).unwrap().to_string();
    assert!(!inspect.contains(r#""inputs""#), "the mesh map is read again: {inspect}");
}

#[test]
fn changing_generator_kind_keeps_the_effect_identity_and_settings() {
    let mut state = session();
    execute(&mut state, "layer.add.fill", &json!({})).unwrap();
    execute(&mut state, "mask.add.black", &Value::Null).unwrap();
    let effect = execute(&mut state, "effect.add.generator", &json!({"generator": "dirt"})).unwrap()["effect"].as_u64().unwrap();
    execute(&mut state, "effect.set", &json!({"effect": effect, "name": "Keep me", "opacity": 0.4, "level": 0.2})).unwrap();
    execute(&mut state, "effect.set", &json!({"effect": effect, "input": "ambientOcclusion", "inputValue": 0.25})).unwrap();
    let later = execute(&mut state, "effect.add.generator", &json!({"generator": "position"})).unwrap()["effect"].as_u64().unwrap();

    execute(&mut state, "effect.set", &json!({"effect": effect, "generator": "ambientOcclusion"})).unwrap();

    let (owner, edited) = state.doc.project.effect(texturecraft_core::LayerId(effect)).unwrap();
    assert!(matches!(&edited.kind, texturecraft_core::EffectKind::Generator(generator)
        if generator.kind == texturecraft_core::GeneratorKind::AmbientOcclusion
            && generator.level == 0.2
            && matches!(generator.inputs.get(&texturecraft_core::BakedMap::AmbientOcclusion), Some(texturecraft_core::Source::Uniform([0.25, 0.25, 0.25])))));
    assert_eq!(edited.name, "Keep me");
    assert_eq!(edited.opacity, 0.4);
    let layer = state.doc.project.layer(owner).unwrap();
    let ids: Vec<u64> = layer.mask.as_ref().unwrap().effects.iter().map(|item| item.id.0).collect();
    assert_eq!(ids, vec![effect, later]);
}

#[test]
fn unparseable_color_param_returns_error() {
    let mut s = session();
    let err = execute(&mut s, "layer.add.fill", &json!({"color": "#fff"})).unwrap_err();
    assert_eq!(err.to_string(), "`layer.add.fill`: `color` must be a color ([r, g, b] linear or \"#rrggbb\" sRGB)");

    let err = execute(&mut s, "layer.fill.set", &json!({"channel": "baseColor", "color": "#123"})).unwrap_err();
    assert_eq!(err.to_string(), "`layer.fill.set`: `color` must be a color ([r, g, b] linear or \"#rrggbb\" sRGB)");

    let err = execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": "invalid"})).unwrap_err();
    assert_eq!(err.to_string(), "`tool.material`: `color` must be a color ([r, g, b] linear or \"#rrggbb\" sRGB)");

    let err = execute(&mut s, "mask.add.colorSelection", &json!({"color": "#xyz"})).unwrap_err();
    assert_eq!(err.to_string(), "`mask.add.colorSelection`: `color` must be a color ([r, g, b] linear or \"#rrggbb\" sRGB)");

    let err = execute(&mut s, "display.set", &json!({"scatterColor": "#12345"})).unwrap_err();
    assert_eq!(err.to_string(), "`display.set`: `scatterColor` must be a color ([r, g, b] linear or \"#rrggbb\" sRGB)");

    let rev_before = s.revision;
    let exposure_before = s.doc.project.display.env_exposure;
    let err = execute(&mut s, "display.set", &json!({"exposure": 2.0, "scatterColor": "invalid"})).unwrap_err();
    assert_eq!(err.to_string(), "`display.set`: `scatterColor` must be a color ([r, g, b] linear or \"#rrggbb\" sRGB)");
    assert_eq!(s.doc.project.display.env_exposure, exposure_before);
    assert_eq!(s.revision, rev_before);

    let mask_before = s.tool.mask_value;
    let err = execute(&mut s, "tool.material", &json!({"maskValue": 0.25, "channel": "baseColor", "color": "invalid"})).unwrap_err();
    assert_eq!(err.to_string(), "`tool.material`: `color` must be a color ([r, g, b] linear or \"#rrggbb\" sRGB)");
    assert_eq!(s.tool.mask_value, mask_before);
    assert_eq!(s.revision, rev_before);

    let err = execute(&mut s, "tool.material", &json!({"color": "invalid"})).unwrap_err();
    assert_eq!(err.to_string(), "`tool.material`: `color` must be a color ([r, g, b] linear or \"#rrggbb\" sRGB)");
    assert_eq!(s.revision, rev_before);

    let err = execute(&mut s, "layer.add.fill", &json!({"color": [1e40, 0.0, 0.0]})).unwrap_err();
    assert_eq!(err.to_string(), "`layer.add.fill`: `color` must be a color ([r, g, b] linear or \"#rrggbb\" sRGB)");
    assert_eq!(s.revision, rev_before);

    let err = execute(&mut s, "layer.fill.set", &json!({"channel": "baseColor", "procedural": "noise", "color": "red"})).unwrap_err();
    assert_eq!(err.to_string(), "`layer.fill.set`: `color` must be a color ([r, g, b] linear or \"#rrggbb\" sRGB)");
    assert_eq!(s.revision, rev_before);

    assert!(execute(&mut s, "tool.cloneSource", &json!({"uv": [0.2, 0.8, 0.4]})).is_err());
    execute(&mut s, "tool.polygonFill", &Value::Null).unwrap();
    let err = execute(&mut s, "tool.click", &json!({"uv": [0.2, 0.8, 0.4]})).unwrap_err();
    assert_eq!(err.to_string(), "`tool.click`: a 2D view point needs `uv` as [u, v]");
}

#[test]
fn resetting_a_stencil_puts_back_its_placement_only() {
    let mut stencil = texturecraft_engine::StencilState::new("logo.png");
    stencil.center = [0.4, -0.2];
    stencil.scale = 2.0;
    stencil.rotation = 30.0;
    stencil.opacity = 0.9;
    stencil.tile = true;
    stencil.reset_placement();
    assert_eq!((stencil.center, stencil.scale, stencil.rotation, stencil.opacity, stencil.tile), ([0.0, 0.0], 0.8, 0.0, 0.9, true));
}

#[test]
fn blending_set_gives_each_channel_its_own_mode_and_opacity_in_one_undo_step() {
    let mut s = session();
    let layer = execute(&mut s, "layer.add.paint", &json!({})).unwrap()["layer"].as_u64().unwrap();
    let read = |s: &Session| s.doc.project.layers[&texturecraft_engine::core::LayerId(layer)].channels.clone();
    let before = read(&s);
    execute(
        &mut s,
        "layer.blending.set",
        &json!({"layer": layer, "channels": {"baseColor": {"mode": "Multiply", "opacity": 0.25}, "roughness": {"opacity": 0.5}}}),
    )
    .unwrap();
    let after = read(&s);
    assert_eq!((after[&Channel::BaseColor].mode, after[&Channel::BaseColor].opacity), (texturecraft_engine::core::BlendMode::Multiply, 0.25));
    assert_eq!((after[&Channel::Roughness].mode, after[&Channel::Roughness].opacity), (before[&Channel::Roughness].mode, 0.5));
    assert_eq!(after[&Channel::Metallic], before[&Channel::Metallic], "channels not listed stay as they were");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert_eq!(read(&s), before, "one undo step restores every listed channel");

    execute(&mut s, "edit.redo", &Value::Null).unwrap();

    // A channel the layer has disabled stays disabled
    execute(&mut s, "layer.blend", &json!({"layer": layer, "channel": "metallic", "enabled": false})).unwrap();
    execute(&mut s, "layer.blending.set", &json!({"layer": layer, "channels": {"metallic": {"opacity": 0.1}}})).unwrap();

    assert!(!read(&s).contains_key(&Channel::Metallic));

    let stable = read(&s);
    for bad in [
        json!({"layer": layer}),
        json!({"layer": layer, "channels": {"nope": {}}}),
        json!({"layer": layer, "channels": {"baseColor": {"mode": "Multiply"}, "nope": {}}}),
        json!({"layer": layer, "channels": {"baseColor": {"mode": "???"}}}),
    ] {
        assert!(execute(&mut s, "layer.blending.set", &bad).is_err(), "{bad}");
        assert_eq!(read(&s), stable, "an error changes nothing");
    }
    execute(&mut s, "layer.blending.set", &json!({"layer": layer, "channels": {"baseColor": {"opacity": 2.0}, "roughness": {"opacity": -1.0}}})).unwrap();

    assert_eq!((read(&s)[&Channel::BaseColor].opacity, read(&s)[&Channel::Roughness].opacity), (1.0, 0.0), "opacity is clamped");
}

#[test]
fn interleaved_sessions_render_depth_from_their_own_mesh() {
    use texturecraft_engine::geom::{Mesh, glam::Vec3};

    let make_mesh = |front_edge| {
        let mut positions = vec![
            Vec3::new(0.0, -1.0, 0.5),
            Vec3::new(front_edge, -1.0, 0.5),
            Vec3::new(front_edge, 1.0, 0.5),
            Vec3::new(0.0, 1.0, 0.5),
            Vec3::new(-1.0, -1.0, -0.5),
            Vec3::new(1.0, -1.0, -0.5),
            Vec3::new(1.0, 1.0, -0.5),
            Vec3::new(-1.0, 1.0, -0.5),
        ];
        // Padding makes the old sampled key miss the front edge
        positions.resize(200, Vec3::ZERO);
        let mut uvs = vec![
            Vec2::new(0.5, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.5, 1.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(0.5, 0.0),
            Vec2::new(0.5, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        uvs.resize(positions.len(), Vec2::ZERO);
        let normals = vec![Vec3::Z; positions.len()];
        Mesh::from_parts(positions, Some(normals), uvs, vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7], "Two squares").unwrap()
    };

    let mut covered = Session::new_project("Covered", make_mesh(1.0), "squares", None, 128).unwrap();
    let mut exposed = Session::new_project("Exposed", make_mesh(0.5), "squares", None, 128).unwrap();
    let sample_rear_surface = |session: &Session| session.caches[0].composite.value(Channel::BaseColor, 56, 32);

    for session in [&mut covered, &mut exposed] {
        execute(session, "layer.add.fill", &json!({"color": [1.0, 0.0, 0.0]})).unwrap();
        execute(
            session,
            "layer.fill.projection",
            &json!({
                "projection": "planar", "axis": 2, "depthCulling": true, "depthCullingHardness": 1.0,
            }),
        )
        .unwrap();
        session.update();
    }

    let hidden = sample_rear_surface(&covered);
    let visible = sample_rear_surface(&exposed);
    assert!((hidden[0] - hidden[1]).abs() < 0.05, "the rear surface is occluded: {hidden:?}");
    assert!(visible[0] > visible[1] + 0.5, "the rear surface is exposed: {visible:?}");
}
