//! The Transform filter: each UV tile's content scaled, turned, moved and flipped, repeating round the tile or empty past it

use serde_json::json;
use texturecraft_engine::core::{EffectKind, Filter, FlipMode, LayerId, RepeatMode, TileTransform};
use texturecraft_engine::{Session, execute};

use crate::filter_support::{SIZE, add_filter, refused, roughness, session, set};

fn transform_of(s: &Session, effect: LayerId) -> TileTransform {
    match &s.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Filter(Filter::Transform(transform)) => *transform,
        other => panic!("not a transform: {other:?}"),
    }
}

/// A new Transform changes nothing; Offset moves the content round the tile, Repeat mode None leaves empty past it, Flip mode turns it over
#[test]
fn transform_moves_scales_and_flips_each_tile() {
    let mut s = session();
    let row = SIZE / 2;
    let empty = roughness(&mut s, (8, row));

    execute(&mut s, "layer.add.fill", &json!({"color": [0.5, 0.5, 0.5], "roughness": 0.25})).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"channel": "roughness", "procedural": "gradient"})).unwrap();
    let before: Vec<f32> = (0..SIZE).map(|x| roughness(&mut s, (x, row))).collect();
    let was = |x: u32| before[(x % SIZE) as usize];

    let effect = add_filter(&mut s, json!({"filter": "transform"}));

    assert_eq!(transform_of(&s, effect), TileTransform::LIBRARY);
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Transform");

    for x in [20, 64, 100] {
        assert!((roughness(&mut s, (x, row)) - was(x)).abs() < 1e-4, "a new Transform changes nothing at {x}");
    }

    // A quarter tile on: each texel reads 32 texels further right, the last ones round from the left
    set(&mut s, effect, json!({"offset": [0.25, 0.0]}));

    for x in [20, 50, 110] {
        assert!((roughness(&mut s, (x, row)) - was(x + SIZE / 4)).abs() < 2e-3, "offset at {x}");
    }

    set(&mut s, effect, json!({"offset": 0.0, "flipMode": "horizontally"}));

    assert!((roughness(&mut s, (20, row)) - was(SIZE - 1 - 20)).abs() < 2e-3, "flipped, the left reads the right");

    // Twice across about the centre: the centre stays; without repeat the edges read empty (the layer's content is clear there)
    set(&mut s, effect, json!({"flipMode": "none", "scaling": [2.0, 2.0], "repeatMode": "none"}));

    assert!((roughness(&mut s, (SIZE / 2, row)) - was(SIZE / 2)).abs() < 2e-2, "the centre stays");
    assert!((roughness(&mut s, (8, row)) - empty).abs() < 1e-4, "past the content reads empty");

    // Repeating, texel 8 reads 2 · 8.5 / 128 − 0.5 of a tile on: between texels 80 and 81
    set(&mut s, effect, json!({"repeatMode": "horizontallyAndVertically"}));

    assert!((roughness(&mut s, (8, row)) - (was(80) + was(81)) / 2.0).abs() < 2e-3, "the tile comes round again");

    let edited = transform_of(&s, effect);

    assert_eq!((edited.repeat, edited.flip, edited.scaling), (RepeatMode::HorizontallyAndVertically, FlipMode::None, [2.0, 2.0]));

    set(&mut s, effect, json!({"rotation": -30.0, "tileSafeRotation": false}));

    let turned = transform_of(&s, effect);
    assert_eq!((turned.rotation, turned.tile_safe), (330.0, false), "a turn reads 0..360");

    refused(
        &mut s,
        effect,
        &[
            json!({"repeatMode": "sideways"}),
            json!({"flipMode": "diagonal"}),
            json!({"scaling": [1.0]}),
            json!({"offset": "far"}),
            json!({"tileSafeRotation": 1}),
        ],
    );

    assert_eq!(transform_of(&s, effect), turned, "a refused value changes nothing");

    set(&mut s, effect, json!({"scaling": [1e9, -5.0]}));

    assert_eq!(transform_of(&s, effect).scaling, [TileTransform::SCALING.1, TileTransform::SCALING.0], "scaling clamps");

    execute(&mut s, "effect.filter.restoreDefaults", &json!({"effect": effect.0})).unwrap();

    assert_eq!(transform_of(&s, effect), TileTransform::LIBRARY);
}
