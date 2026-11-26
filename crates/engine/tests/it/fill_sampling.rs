//! How fills read their image sources (filtering), through engine commands

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, Fill, Filtering, LayerId, LayerKind, Projection, Sampling, ScaleMode};
use texturecraft_engine::geom::NO_TRI;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
    s.update();
    s
}

fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y))).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// A fill layer whose base color is a one-texel black and white checker, tiled `tiling` times
fn checker_fill(s: &mut Session, tiling: f32) -> LayerId {
    let name = s.import_resource_bytes("checker.png", png(64, 64, |x, y| if (x + y) % 2 == 0 { [255; 4] } else { [0, 0, 0, 255] })).unwrap();
    let layer = LayerId(execute(s, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    execute(s, "layer.fill.set", &json!({"layer": layer.0, "channel": "baseColor", "image": name})).unwrap();
    execute(s, "layer.fill.projection", &json!({"layer": layer.0, "scale": tiling})).unwrap();
    layer
}

/// Lowest and highest base color red across the covered texels of the set's middle band
fn spread(s: &mut Session) -> (f32, f32) {
    s.update();
    let cache = &s.caches[0];
    let (w, h) = cache.size;
    let mut range = (f32::MAX, f32::MIN);
    for y in h * 3 / 8..h * 5 / 8 {
        for x in w / 4..w * 3 / 4 {
            if cache.gbuf.tri[(y * w + x) as usize] != NO_TRI {
                let red = cache.composite.value(Channel::BaseColor, x, y)[0];
                range = (range.0.min(red), range.1.max(red));
            }
        }
    }
    range
}

fn filtering(s: &mut Session, layer: LayerId, id: &str) {
    execute(s, "layer.fill.projection", &json!({"layer": layer.0, "filtering": id})).unwrap();
}

/// A checker shrunk to two image texels per texel: HQ averages it, Sharp aliases it, Nearest keeps black and white
#[test]
fn filtering_decides_how_a_shrunk_image_reads() {
    let mut s = session();
    let layer = checker_fill(&mut s, 7.3);
    let (lo, hi) = spread(&mut s);

    assert!(hi - lo < 0.2, "a new fill reads with Bilinear | HQ: an even gray, got {lo}..{hi}");

    filtering(&mut s, layer, "bilinearSharp");
    let (lo, hi) = spread(&mut s);
    assert!(hi - lo > 0.5, "Bilinear | Sharp aliases a shrunk checker: {lo}..{hi}");

    filtering(&mut s, layer, "nearest");
    s.update();
    let cache = &s.caches[0];
    let (w, h) = cache.size;
    let pure = (h * 3 / 8..h * 5 / 8)
        .flat_map(|y| (w / 4..w * 3 / 4).map(move |x| (x, y)))
        .filter(|&(x, y)| cache.gbuf.tri[(y * w + x) as usize] != NO_TRI)
        .all(|(x, y)| {
            let red = cache.composite.value(Channel::BaseColor, x, y)[0];
            !(0.01..=0.99).contains(&red)
        });

    assert!(pure, "Nearest reads one image texel: black or white only");

    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    let row = inspect["layers"].as_array().unwrap().iter().find(|l| l["id"] == layer.0).unwrap().clone();

    assert_eq!(row["fill"]["filtering"], json!("nearest"));
    assert!(execute(&mut s, "layer.fill.projection", &json!({"layer": layer.0, "filtering": "trilinear"})).is_err());
}

/// Fills saved before Filtering existed read images as before (plain bilinear); new fills start on Bilinear | HQ
#[test]
fn fills_saved_before_filtering_keep_their_look() {
    let mut s = session();
    let layer = checker_fill(&mut s, 1.0);
    let LayerKind::Fill(fill) = &s.doc.project.layer(layer).unwrap().kind else { panic!("a fill layer") };

    assert_eq!(fill.sampling.filtering, Filtering::BilinearHq);

    let mut saved = serde_json::to_value(fill).unwrap();
    saved.as_object_mut().unwrap().remove("sampling");
    let loaded: Fill = serde_json::from_value(saved).unwrap();

    assert_eq!(loaded.sampling, Sampling::SAVED_BEFORE);
    assert_eq!(loaded.sampling.filtering, Filtering::BilinearSharp);
}

/// Share of the middle band's covered texels where the white fill shows over the black layer below
fn shown(s: &mut Session) -> f32 {
    s.update();
    let cache = &s.caches[0];
    let (w, h) = cache.size;
    let (mut on, mut all) = (0usize, 0usize);
    for y in h * 3 / 8..h * 5 / 8 {
        for x in 0..w {
            if cache.gbuf.tri[(y * w + x) as usize] != NO_TRI {
                all += 1;
                on += usize::from(cache.composite.value(Channel::BaseColor, x, y)[0] > 0.5);
            }
        }
    }
    on as f32 / all.max(1) as f32
}

/// UV Wrap on a white fill tiled 4 × 2: Repeat covers the band, None one tile, one-axis repeats in between
#[test]
fn uv_wrap_repeats_the_tile_or_stops_at_its_edge() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({"color": [0.0, 0.0, 0.0]})).unwrap();
    let layer = LayerId(execute(&mut s, "layer.add.fill", &json!({"color": [1.0, 1.0, 1.0]})).unwrap()["layer"].as_u64().unwrap());
    let set = |s: &mut Session, p: Value| {
        let mut p = p;
        p["layer"] = json!(layer.0);
        execute(s, "layer.fill.projection", &p)
    };
    set(&mut s, json!({"scale": [4, 2]})).unwrap();
    let near = |got: f32, want: f32| (got - want).abs() < 0.05;
    let repeat = shown(&mut s);

    assert!(near(repeat, 1.0), "Repeat covers the band: {repeat}");

    set(&mut s, json!({"wrap": "none"})).unwrap();
    let none = shown(&mut s);
    assert!(near(none, 0.125), "None: a quarter across, half down: {none}");

    set(&mut s, json!({"wrap": "repeatHorizontally"})).unwrap();
    let across = shown(&mut s);
    assert!(near(across, 0.5), "repeating across: the whole width, half down: {across}");

    set(&mut s, json!({"wrap": "repeatVertically"})).unwrap();
    let down = shown(&mut s);
    assert!(near(down, 0.25), "repeating down: a quarter across, the whole height: {down}");

    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    let row = inspect["layers"].as_array().unwrap().iter().find(|l| l["id"] == layer.0).unwrap().clone();

    assert_eq!(row["fill"]["wrap"], json!("repeatVertically"));
    assert!(set(&mut s, json!({"wrap": "mirror"})).is_err());

    // Tri-planar has no tile: it covers whatever the wrap says
    set(&mut s, json!({"projection": "triplanar", "wrap": "none"})).unwrap();
    let triplanar = shown(&mut s);
    assert!(near(triplanar, 1.0), "{triplanar}");
}

/// 3D projections take UV transformations; older saves look as they did; world scale and offset stay separate
#[test]
fn three_d_projections_take_uv_transformations() {
    use texturecraft_engine::core::{Projection, UvTransform};
    let mut s = session();
    let layer = LayerId(execute(&mut s, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    let set = |s: &mut Session, p: Value| {
        let mut p = p;
        p["layer"] = json!(layer.0);
        execute(s, "layer.fill.projection", &p)
    };
    execute(
        &mut s,
        "layer.fill.set",
        &json!({"layer": layer.0, "channel": "baseColor", "procedural": "Checker", "scale": 4, "low": [0, 0, 0], "high": [1, 1, 1]}),
    )
    .unwrap();
    set(&mut s, json!({"projection": "triplanar"})).unwrap();
    let projection = |s: &Session| match &s.doc.project.layer(layer).unwrap().kind {
        LayerKind::Fill(f) => f.projection.clone(),
        _ => panic!("a fill layer"),
    };
    let snapshot = |s: &mut Session| {
        s.update();
        let cache = &s.caches[0];
        let (w, h) = cache.size;
        (0..w * h).map(|i| cache.composite.value(Channel::BaseColor, i % w, i / w)[0]).collect::<Vec<f32>>()
    };
    let plain = snapshot(&mut s);

    set(&mut s, json!({"tiling": [2, 3], "rotation": 45, "uvOffset": [0.25, 0.5]})).unwrap();
    let Projection::Triplanar { uv, scale, .. } = projection(&s) else { panic!("tri-planar") };

    assert_eq!(uv, UvTransform { tiling: [2.0, 3.0], rotation: 45.0, offset: [0.25, 0.5] });
    assert_eq!(scale, 1.0, "the world scale is its own param");
    assert_ne!(snapshot(&mut s), plain, "the UV transformations move the pattern");

    // A fill saved before 3D projections had UV transformations loads them as nothing
    let mut saved = serde_json::to_value(projection(&s)).unwrap();
    saved["Triplanar"].as_object_mut().unwrap().remove("uv");
    let loaded: Projection = serde_json::from_value(saved).unwrap();

    assert!(matches!(loaded, Projection::Triplanar { uv, .. } if uv == UvTransform::default()));

    set(&mut s, json!({"tiling": 1, "rotation": 0, "uvOffset": [0, 0]})).unwrap();

    assert_eq!(snapshot(&mut s), plain, "the identity transform changes nothing");

    // On a UV projection, tiling is its scale
    set(&mut s, json!({"projection": "uv", "tiling": [2, 4]})).unwrap();

    assert!(matches!(projection(&s), Projection::Uv { scale, .. } if scale == [2.0, 4.0]));
}

/// Physical size keeps the fill's tiling; inspect reports the mode and older fills open on Tiling
#[test]
fn scale_mode_is_stored_without_changing_the_tiling() {
    let mut s = session();
    let layer = checker_fill(&mut s, 2.0);
    execute(&mut s, "layer.fill.projection", &json!({"layer": layer.0, "scaleMode": "physicalSize"})).unwrap();
    let LayerKind::Fill(fill) = s.doc.project.layer(layer).unwrap().kind.clone() else { panic!("a fill layer") };

    assert_eq!(fill.sampling.scale, ScaleMode::PhysicalSize);
    assert!(matches!(fill.projection, Projection::Uv { scale: [2.0, 2.0], .. }), "{:?}", fill.projection);

    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    let row = inspect["layers"].as_array().unwrap().iter().find(|l| l["id"] == layer.0).unwrap().clone();

    assert_eq!(row["fill"]["scaleMode"], json!("physicalSize"));
    assert!(execute(&mut s, "layer.fill.projection", &json!({"layer": layer.0, "scaleMode": "inches"})).is_err());

    let mut saved = serde_json::to_value(&fill).unwrap();
    saved["sampling"].as_object_mut().unwrap().remove("scale");
    let loaded: Fill = serde_json::from_value(saved).unwrap();

    assert_eq!(loaded.sampling.scale, ScaleMode::Tiling);
}
