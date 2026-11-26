//! The shape filters: Threshold's modes, Mirror, Mask Outline, Bevel, Bevel Smooth, Directional Distance, Warp's modes and Blur Slope's source

use serde_json::{Value, json};
use texturecraft_engine::core::{
    Bevel, BevelBlur, BevelCorner, BevelDirection, BevelSmooth, DirectionalDistance, EffectKind, Filter, MaskOutline, Mirror, MirrorCorner, MirrorMode,
    OutlinePosition, SlopeBlending, SourceSlopeBlur, SourceWarp, ThresholdMode, WarpBlending, WarpMode, WarpSource, WarpSourceMode,
};
use texturecraft_engine::{Session, execute};

use crate::filter_support::{SIZE, add_filter, filter_of, refused, roughness, session, set};

/// Threshold: a new one is Greater than 0.01; each mode compares its way; one saved before keeps Greater or equal
#[test]
fn threshold_modes_compare_their_way() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({"color": [0.5, 0.5, 0.5], "roughness": 0.25})).unwrap();
    let effect = add_filter(&mut s, json!({"filter": "threshold"}));
    assert_eq!(filter_of(&s, effect), Filter::Threshold { level: 0.01, mode: ThresholdMode::Greater }, "the library defaults");

    let texel = (SIZE / 4, SIZE / 2);
    assert_eq!(roughness(&mut s, texel), 1.0);

    set(&mut s, effect, json!({"level": 0.25}));

    assert_eq!(roughness(&mut s, texel), 0.0, "a value at the level is not greater");

    for (mode, white) in [("greaterOrEqual", 1.0), ("lower", 0.0), ("lowerOrEqual", 1.0)] {
        set(&mut s, effect, json!({"mode": mode}));

        assert_eq!(roughness(&mut s, texel), white, "{mode}");
    }

    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "mode": "sideways"})).is_err());
    assert_eq!(filter_of(&s, effect), Filter::Threshold { level: 0.25, mode: ThresholdMode::LowerOrEqual }, "a refused mode changes nothing");

    execute(&mut s, "effect.filter.restoreDefaults", &json!({"effect": effect.0})).unwrap();

    assert_eq!(filter_of(&s, effect), Filter::Threshold { level: 0.01, mode: ThresholdMode::Greater });
}

/// Mirror: the left half of each UV tile copied onto the right (inverted the other way, or a corner onto the rest), its axis white in Display Debug
#[test]
fn mirror_copies_one_side_over_the_other() {
    let mut s = session();
    execute(&mut s, "layer.add.fill", &json!({"color": [0.5, 0.5, 0.5], "roughness": 0.25})).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"channel": "roughness", "procedural": "gradient"})).unwrap();
    let row = SIZE / 2;
    let (left, right) = ((20, row), (SIZE - 1 - 20, row));
    let (left_before, right_before) = (roughness(&mut s, left), roughness(&mut s, right));
    assert!((left_before - right_before).abs() > 0.05, "the gradient differs across: {left_before} {right_before}");

    let effect = add_filter(&mut s, json!({"filter": "mirror"}));
    assert_eq!(filter_of(&s, effect), Filter::Mirror(Mirror::LIBRARY));
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Mirror");
    assert!((roughness(&mut s, left) - left_before).abs() < 2e-3, "the left side stays");
    assert!((roughness(&mut s, right) - left_before).abs() < 2e-3, "the right side reads the left");

    set(&mut s, effect, json!({"invertAxisX": true}));

    assert!((roughness(&mut s, left) - right_before).abs() < 2e-3, "inverted, the left side reads the right");

    set(&mut s, effect, json!({"mode": "corner", "cornerType": "topRight"}));

    assert!((roughness(&mut s, (20, 20)) - roughness(&mut s, (SIZE - 1 - 20, 20))).abs() < 2e-3, "a corner mirrors across too");

    let Filter::Mirror(mirror) = filter_of(&s, effect) else { panic!() };
    assert_eq!((mirror.mode, mirror.corner), (MirrorMode::Corner, MirrorCorner::TopRight));

    set(&mut s, effect, json!({"mode": "axisX", "displayDebug": true}));

    assert_eq!(roughness(&mut s, (SIZE / 2, row)), 1.0, "Display Debug draws the axis white");

    refused(&mut s, effect, &[json!({"mode": "diagonal"}), json!({"cornerType": "middle"}), json!({"invertAxisY": "yes"})]);

    set(&mut s, effect, json!({"axisYOffset": 3.0}));
    let Filter::Mirror(mirror) = filter_of(&s, effect) else { panic!() };
    assert_eq!(mirror.offset, [0.5, 1.0], "offsets clamp to the tile");
}

/// Mask Outline: a band along the edge where the gradient crosses its Threshold, outside, inside or on both sides, on black or over the values
#[test]
fn mask_outline_draws_a_band_along_the_threshold_edge() {
    let mut s = session();
    let (at, values, _) = shape_edge(&mut s);

    let effect = add_filter(&mut s, json!({"filter": "maskOutline"}));
    assert_eq!(filter_of(&s, effect), Filter::MaskOutline(MaskOutline::LIBRARY), "the library defaults");
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Mask Outline");

    set(&mut s, effect, json!({"threshold": 0.5, "width": 0.03}));

    assert_eq!(roughness(&mut s, at(2)), 1.0, "outside, next to the edge");
    assert_eq!(roughness(&mut s, at(8)), 0.0, "past the band");
    assert_eq!(roughness(&mut s, at(-2)), 0.0, "inside the shapes");

    set(&mut s, effect, json!({"maskPosition": "inside"}));

    assert_eq!((roughness(&mut s, at(-2)), roughness(&mut s, at(2))), (1.0, 0.0), "inside");

    set(&mut s, effect, json!({"maskPosition": "both"}));

    assert_eq!((roughness(&mut s, at(-2)), roughness(&mut s, at(2))), (1.0, 1.0), "both sides");

    set(&mut s, effect, json!({"startFromBlack": false}));
    let far = at(8);
    assert!((roughness(&mut s, far) - values[far.0 as usize]).abs() < 2e-3, "over the values");

    set(&mut s, effect, json!({"startFromBlack": true, "maskPosition": "outside", "blur": 1.0}));
    let (near, further) = (roughness(&mut s, at(1)), roughness(&mut s, at(2)));
    assert!(near > further && further > 0.0 && near < 1.0, "a blurred band fades out: {near} {further}");

    refused(&mut s, effect, &[json!({"maskPosition": "around"}), json!({"startFromBlack": 1})]);

    set(&mut s, effect, json!({"width": 4.0, "curveShape": -3.0}));
    let Filter::MaskOutline(outline) = filter_of(&s, effect) else { panic!() };
    assert_eq!((outline.position, outline.width, outline.curve), (OutlinePosition::Outside, 1.0, -1.0), "values clamp");

    execute(&mut s, "effect.filter.restoreDefaults", &json!({"effect": effect.0})).unwrap();

    assert_eq!(filter_of(&s, effect), Filter::MaskOutline(MaskOutline::LIBRARY));
}

/// A fill whose roughness is a repeating gradient: the middle row's values
fn gradient_row(s: &mut Session) -> Vec<f32> {
    execute(s, "layer.add.fill", &json!({"color": [0.5, 0.5, 0.5], "roughness": 0.25})).unwrap();
    execute(s, "layer.fill.set", &json!({"channel": "roughness", "procedural": "gradient"})).unwrap();
    middle_row(s)
}

/// The composited roughness along the middle row
fn middle_row(s: &mut Session) -> Vec<f32> {
    (0..SIZE).map(|x| roughness(s, (x, SIZE / 2))).collect()
}

/// A roughness gradient crossing 0.5 away from the tile's sides along the middle row: texel `steps` outward of the shapes' first texel at its edge (inward below 0), the values before any filter, and the outward direction along x
pub(crate) fn shape_edge(s: &mut Session) -> (impl Fn(i64) -> (u32, u32) + use<>, Vec<f32>, i64) {
    let row = SIZE / 2;
    let values = gradient_row(s);
    let edge = (SIZE / 4..SIZE * 3 / 4).find(|x| (values[*x as usize] >= 0.5) != (values[*x as usize - 1] >= 0.5)).unwrap();
    let (inner, outward): (u32, i64) = if values[edge as usize] >= 0.5 { (edge, -1) } else { (edge - 1, 1) };
    (move |steps: i64| ((inner as i64 + steps * outward) as u32, row), values, outward)
}

/// Bevel: a negative Distance slopes the shapes down to their edge, a positive one rises around them; Smoothing, Corner Type, Blur Mode and Wrap set; one saved before keeps its width bevel
#[test]
fn bevel_slopes_inwards_or_outwards() {
    let mut s = session();
    let (at, values, _) = shape_edge(&mut s);
    let effect = add_filter(&mut s, json!({"filter": "bevel"}));
    assert_eq!(filter_of(&s, effect), Filter::DistanceBevel(Bevel::LIBRARY), "the library defaults");
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Bevel");

    let value = |at: (u32, u32)| values[at.0 as usize];

    // Inwards over a tenth of the tile (12.8 texels here)
    let (edge, outside) = (roughness(&mut s, at(0)), roughness(&mut s, at(3)));
    assert!(edge < value(at(0)) * 0.2, "down at the edge: {edge} vs {}", value(at(0)));
    assert!((outside - value(at(3))).abs() < 2e-3, "outside untouched");

    set(&mut s, effect, json!({"distance": 0.1}));
    let (near, further) = (roughness(&mut s, at(1)), roughness(&mut s, at(6)));
    assert!(near > 0.8 && further > value(at(6)) && near > further, "rises around the shapes: {near} {further}");
    assert!((roughness(&mut s, at(-2)) - value(at(-2))).abs() < 2e-3, "inside untouched");

    set(&mut s, effect, json!({"smoothing": 2.0, "blurMode": "uniform"}));
    let smoothed = roughness(&mut s, at(1));
    assert!((smoothed - near).abs() > 1e-3, "smoothing blurs the slope: {smoothed} vs {near}");

    set(&mut s, effect, json!({"cornerType": "angular", "wrap": true}));
    refused(&mut s, effect, &[json!({"cornerType": "square"}), json!({"blurMode": "even"}), json!({"wrap": 1})]);

    set(&mut s, effect, json!({"distance": -3.0, "smoothing": 9.0}));
    let expected = Bevel { wrap: true, distance: -1.0, corner: BevelCorner::Angular, smoothing: 5.0, blur: BevelBlur::Uniform };
    assert_eq!(filter_of(&s, effect), Filter::DistanceBevel(expected), "values clamp");

    execute(&mut s, "effect.filter.restoreDefaults", &json!({"effect": effect.0})).unwrap();

    assert_eq!(filter_of(&s, effect), Filter::DistanceBevel(Bevel::LIBRARY));

    let old: Filter = serde_json::from_value(json!({"Bevel": {"width": 16.0}})).unwrap();

    assert_eq!(old, Filter::Bevel { width: 16.0, wrap: false }, "a bevel saved before keeps its width");
}

/// Bevel Smooth: a ramp from the edge outwards (shapes white), inwards (around them black) or across it (mid gray on it)
#[test]
fn bevel_smooth_ramps_from_the_edge() {
    let mut s = session();
    let (at, _, _) = shape_edge(&mut s);
    let effect = add_filter(&mut s, json!({"filter": "bevelSmooth"}));
    assert_eq!(filter_of(&s, effect), Filter::BevelSmooth(BevelSmooth::LIBRARY), "the library defaults");
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Bevel Smooth");

    set(&mut s, effect, json!({"smoothing": 0.0}));

    let (inside, near, further) = (roughness(&mut s, at(-3)), roughness(&mut s, at(1)), roughness(&mut s, at(6)));
    assert_eq!(inside, 1.0, "out: the shapes stay white");
    assert!(near > further && further > 0.0 && near < 1.0, "fades outwards: {near} {further}");

    set(&mut s, effect, json!({"direction": "in"}));
    let (outside, inner) = (roughness(&mut s, at(3)), roughness(&mut s, at(-3)));
    assert!(outside == 0.0 && inner > 0.0 && inner < 1.0, "in: rises inwards: {outside} {inner}");

    set(&mut s, effect, json!({"direction": "inOut"}));
    let (edge_in, edge_out) = (roughness(&mut s, at(0)), roughness(&mut s, at(1)));
    assert!(edge_in > 0.5 && edge_out < 0.5 && edge_in - edge_out < 0.2, "across: mid gray at the edge: {edge_in} {edge_out}");

    set(&mut s, effect, json!({"curveOffset": 0.5}));

    assert!(roughness(&mut s, at(0)) > edge_in, "the offset raises the ramp");

    refused(&mut s, effect, &[json!({"direction": "up"}), json!({"wrap": "yes"})]);

    set(&mut s, effect, json!({"distance": 2.0, "smoothing": 20.0, "maskThreshold": -1.0, "distanceMapMultiplier": 4.0, "curveShape": 5.0}));
    let Filter::BevelSmooth(smooth) = filter_of(&s, effect) else { panic!() };
    assert_eq!(
        (smooth.direction, smooth.distance, smooth.smoothing, smooth.threshold, smooth.multiplier, smooth.curve_shape),
        (BevelDirection::InOut, 0.3, 10.0, 0.0, 1.0, 1.0),
        "values clamp"
    );
}

/// Directional Distance: a trail fading away from the shapes towards its Angle, none against it
#[test]
fn directional_distance_trails_one_way() {
    let mut s = session();
    let (at, _, outward) = shape_edge(&mut s);
    let effect = add_filter(&mut s, json!({"filter": "directionalDistance"}));
    assert_eq!(filter_of(&s, effect), Filter::DirectionalDistance(DirectionalDistance::LIBRARY), "the library defaults");
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Directional Distance");

    // Along the row, away from the shapes
    let away = if outward > 0 { 0.0 } else { 180.0 };
    set(&mut s, effect, json!({"angle": away, "distance": 0.05}));
    let (inside, near, further) = (roughness(&mut s, at(-2)), roughness(&mut s, at(1)), roughness(&mut s, at(4)));
    assert_eq!(inside, 1.0, "the shapes are white");
    assert!(near > further && further > 0.0, "fades away from them: {near} {further}");

    set(&mut s, effect, json!({"contrast": 0.5}));

    assert_eq!(roughness(&mut s, at(1)), 1.0, "contrast hardens the near trail");

    set(&mut s, effect, json!({"angle": away + 180.0, "contrast": 0.0}));

    assert!(roughness(&mut s, at(1)) < near, "against the angle the edge gets less trail");

    set(&mut s, effect, json!({"distance": 1.0, "angle": -90.0, "contrast": 3.0, "distanceMapMultiplier": -1.0}));
    let expected = DirectionalDistance { distance: 0.3, angle: 270.0, contrast: 1.0, multiplier: 0.0 };
    assert_eq!(filter_of(&s, effect), Filter::DirectionalDistance(expected), "values clamp, the angle wraps");
}

/// Texels either side of a texel that must all rise (or fall) for the warp and slope tests to read it as on a slope
const SLOPE_WINDOW: usize = 8;

/// Warp: library defaults, Previous Input moving texels by their values, and blended directions
#[test]
fn warp_modes_move_texels_their_way() {
    let mut s = session();
    let before = gradient_row(&mut s);
    let after = middle_row;
    let moved = |values: &[f32]| values.iter().zip(&before).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);

    let effect = add_filter(&mut s, json!({"filter": "warp"}));
    assert_eq!(filter_of(&s, effect), Filter::SourceWarp(SourceWarp::LIBRARY), "the library defaults");
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Warp");

    set(&mut s, effect, json!({"intensity": 0.05, "divider": 1}));
    let warped = after(&mut s);
    assert!(moved(&warped) > 0.01 && warped.iter().all(|v| v.is_finite()), "the noise moves texels: {}", moved(&warped));

    set(&mut s, effect, json!({"intensity": 0.0}));

    assert!(moved(&after(&mut s)) < 2e-3, "no intensity, no move");

    // Along the gradient where it rises: a texel above mid gray reads further up it, so it gets lighter
    let rising = |x: usize| (x - SLOPE_WINDOW..x + SLOPE_WINDOW).all(|k| before[k + 1] > before[k]);
    let falling = |x: usize| (x - SLOPE_WINDOW..x + SLOPE_WINDOW).all(|k| before[k + 1] < before[k]);
    let light = |x: &usize| (0.6..0.85).contains(&before[*x]);
    let (texel, angle) = match (SLOPE_WINDOW..SIZE as usize - SLOPE_WINDOW).find(|x| light(x) && rising(*x)) {
        Some(x) => (x, 0.0),
        None => ((SLOPE_WINDOW..SIZE as usize - SLOPE_WINDOW).find(|x| light(x) && falling(*x)).unwrap(), 180.0),
    };
    set(&mut s, effect, json!({"intensity": 0.05, "warpMode": "directional", "angle": angle, "sourceMode": "previousInput"}));
    let directional = after(&mut s);
    assert!(directional[texel] > before[texel] + 0.01, "{} -> {}", before[texel], directional[texel]);

    set(&mut s, effect, json!({"sourceMode": "customNoise"}));

    assert!(moved(&after(&mut s)) < 2e-3, "an unlinked custom noise is flat: nothing moves");

    set(&mut s, effect, json!({"warpMode": "multiDirectional", "sourceMode": "defaultNoise", "directions": 2, "blendingMode": "max"}));
    let max = after(&mut s);
    set(&mut s, effect, json!({"blendingMode": "min"}));
    let min = after(&mut s);
    set(&mut s, effect, json!({"blendingMode": "average"}));
    let average = after(&mut s);
    assert!(max.iter().zip(&min).all(|(max, min)| max + 1e-4 >= *min) && max.iter().zip(&min).any(|(max, min)| max - min > 0.01), "Max over Min");
    assert!(average.iter().zip(max.iter().zip(&min)).all(|(average, (max, min))| *average <= max + 2e-3 && *average >= min - 2e-3), "the average between");

    set(&mut s, effect, json!({"blendingMode": "chain", "directions": 4}));

    assert!(after(&mut s).iter().all(|v| v.is_finite()));

    refused(&mut s, effect, &[json!({"warpMode": "spiral"}), json!({"directions": 3}), json!({"blendingMode": "sum"}), json!({"sourceMode": "image"})]);

    set(&mut s, effect, json!({"intensity": 5.0, "angle": -90.0, "sourceTiling": 99.0, "sourceBlur": -1.0, "seed": 7}));
    let Filter::SourceWarp(warp) = filter_of(&s, effect) else { panic!() };
    assert_eq!((warp.intensity, warp.angle, warp.source.tiling, warp.source.blur, warp.seed), (1.0, 270.0, 16.0, 0.0, 7), "values clamp, the angle wraps");
    assert_eq!((warp.mode, warp.blending, warp.directions), (WarpMode::MultiDirectional, WarpBlending::Chain, 4));
}

/// Opening saved Directional Warp preserves the existing settings without a catalog reset.
#[test]
fn a_saved_directional_warp_has_no_catalog_defaults() {
    let mut session = session();
    let effect = add_filter(&mut session, json!({"filter": "warp"}));

    let saved = json!({"DirectionalWarp": {"intensity": 8.0, "angle": 30.0, "scale": 4.0, "seed": 3}});
    let old: Filter = serde_json::from_value(saved.clone()).unwrap();

    assert_eq!(old, Filter::DirectionalWarp { intensity: 8.0, angle: 30.0, scale: 4.0, seed: 3, divider: 1 });
    assert_eq!(serde_json::to_value(&old).unwrap(), saved, "saves as before");

    session.doc.project.effect_mut(effect).unwrap().kind = EffectKind::Filter(old.clone());
    let reset = execute(&mut session, "effect.filter.restoreDefaults", &json!({"effect": effect.0})).unwrap();

    assert_eq!(reset, json!({"changed": false}));
    assert_eq!(filter_of(&session, effect), old);
}

/// Blur Slope: library defaults, Min under Blur under Max, reading downhill of its Previous Input, Quality, one undo, old saves
#[test]
fn slope_blur_reads_downhill_of_its_source() {
    let mut s = session();
    let before = gradient_row(&mut s);
    let after = middle_row;
    let moved = |values: &[f32]| values.iter().zip(&before).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);

    let effect = add_filter(&mut s, json!({"filter": "blurSlope"}));
    assert_eq!(filter_of(&s, effect), Filter::SourceSlopeBlur(SourceSlopeBlur::LIBRARY), "the library defaults");
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Blur Slope");

    set(&mut s, effect, json!({"intensity": 0.05, "divider": 1}));
    let blur = after(&mut s);
    assert!(moved(&blur) > 0.005 && blur.iter().all(|v| v.is_finite()), "the noise's slope smears: {}", moved(&blur));

    set(&mut s, effect, json!({"blendingMode": "min"}));
    let min = after(&mut s);
    set(&mut s, effect, json!({"blendingMode": "max"}));
    let max = after(&mut s);
    assert!(max.iter().zip(&min).all(|(max, min)| max + 1e-4 >= *min) && max.iter().zip(&min).any(|(max, min)| max - min > 0.01), "Max over Min");
    assert!(blur.iter().zip(max.iter().zip(&min)).all(|(blur, (max, min))| *blur <= max + 2e-3 && *blur >= min - 2e-3), "Blur between");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();
    let Filter::SourceSlopeBlur(slope) = filter_of(&s, effect) else { panic!() };
    assert_eq!(slope.blending, SlopeBlending::Min, "one undo step");

    // Read downhill of the gradient itself: every texel where it rises reads darker values
    set(&mut s, effect, json!({"blendingMode": "blur", "sourceMode": "previousInput"}));
    let downhill = after(&mut s);
    let rising = (SLOPE_WINDOW..SIZE as usize - SLOPE_WINDOW).find(|x| (x - SLOPE_WINDOW..x + SLOPE_WINDOW).all(|k| before[k + 1] > before[k] + 1e-4)).unwrap();

    assert!(downhill[rising] < before[rising] - 0.005, "{} -> {}", before[rising], downhill[rising]);

    set(&mut s, effect, json!({"quality": 0.0}));
    let coarse = after(&mut s);
    assert!(coarse.iter().all(|v| v.is_finite()) && coarse != downhill, "fewer reads, another smear");

    set(&mut s, effect, json!({"sourceMode": "customNoise"}));

    assert!(moved(&after(&mut s)) < 2e-3, "an unlinked custom noise is flat: nothing smears");

    refused(&mut s, effect, &[json!({"blendingMode": "average"}), json!({"sourceMode": "image"}), json!({"divider": 7})]);

    set(&mut s, effect, json!({"intensity": 5.0, "quality": -1.0, "sourceTiling": 99.0, "sourceBlur": 2.0, "sourceBalance": 0.25, "seed": 7}));
    let Filter::SourceSlopeBlur(slope) = filter_of(&s, effect) else { panic!() };
    let source = WarpSource { mode: WarpSourceMode::CustomNoise, blur: 1.0, balance: 0.25, contrast: 0.0, tiling: 16.0 };
    assert_eq!((slope.intensity, slope.quality, slope.source, slope.seed), (1.0, 0.0, source, 7), "values clamp");

    // Saved Blur Slope keeps its existing settings
    let saved = json!({"SlopeBlur": {"radius": 16.0, "scale": 4.0, "seed": 1}});
    let old: Filter = serde_json::from_value(saved.clone()).unwrap();

    assert_eq!(old, Filter::SlopeBlur { radius: 16.0, scale: 4.0, seed: 1, divider: 1 });
    assert_eq!(serde_json::to_value(&old).unwrap(), saved, "saves as before");

    s.doc.project.effect_mut(effect).unwrap().kind = EffectKind::Filter(old.clone());
    let reset = execute(&mut s, "effect.filter.restoreDefaults", &json!({"effect": effect.0})).unwrap();

    assert_eq!(reset, json!({"changed": false}));
    assert_eq!(filter_of(&s, effect), old);
}

#[test]
fn slope_blur_source_tiling_changes_sampled_inputs() {
    let mut session = session();
    let _ = gradient_row(&mut session);
    let effect = add_filter(&mut session, json!({"filter": "blurSlope", "intensity": 0.05, "divider": 1}));
    set(&mut session, effect, json!({"sourceMode": "customNoise"}));
    execute(&mut session, "layer.fill.set", &json!({"effect": effect.0, "input": "customNoise", "procedural": "gradient"})).unwrap();

    for mode in ["previousInput", "customNoise"] {
        set(&mut session, effect, json!({"sourceMode": mode, "sourceTiling": 1.0}));
        let original = middle_row(&mut session);

        set(&mut session, effect, json!({"sourceTiling": 2.0}));
        let repeated = middle_row(&mut session);

        assert!(original.iter().zip(&repeated).any(|(original, repeated)| (original - repeated).abs() > 0.01), "{mode} should repeat the source");
    }
}
