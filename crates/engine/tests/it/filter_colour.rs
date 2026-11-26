//! The colour filters (Color Balance, Color Correct, HSL Perceptive, Contrast Luminosity) and the shared edge wrap of Blur and Bevel

use serde_json::{Value, json};
use texturecraft_engine::core::{Bevel, Channel, Filter};
use texturecraft_engine::execute;

use crate::filter_support::{center, facing_session as session, filter_of, filtered, set};

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).abs()).fold(0.0, f32::max)
}

/// Color Correct: neutral by default, then each tone's contrast, luminosity and saturation; one undo step per edit
#[test]
fn color_correct_changes_nothing_until_its_tones_move() {
    let mut s = session();
    let color = [0.6, 0.3, 0.2];
    let effect = filtered(&mut s, color, json!({"filter": "colorCorrect"}));

    let neutral = filter_of(&s, effect);
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Color Correct");
    assert!(distance(center(&mut s, Channel::BaseColor), color) < 4e-3, "the defaults change nothing");

    set(&mut s, effect, json!({"shadows": [0.0, 0.0, 0.0], "midtones": [0.0, 0.0, 0.0], "highlights": [0.0, 0.0, 0.0]}));

    let gray = center(&mut s, Channel::BaseColor);
    assert!((gray[0] - gray[1]).abs() < 4e-3 && (gray[1] - gray[2]).abs() < 4e-3, "saturation 0 everywhere is gray: {gray:?}");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert_eq!(filter_of(&s, effect), neutral, "one undo step takes back all three tones");

    set(&mut s, effect, json!({"midtones": [0.0, 0.3, 0.5], "shadows": [0.0, 0.3, 0.5], "highlights": [0.0, 0.3, 0.5]}));

    let lighter = center(&mut s, Channel::BaseColor);
    assert!((0..3).all(|i| lighter[i] > color[i] + 0.1), "luminosity lifts every tone: {lighter:?}");

    let roughness = center(&mut s, Channel::Roughness);
    assert!(roughness[0] > 0.4, "scalars take contrast and luminosity: {roughness:?}");

    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "midtones": "x"})).is_err());

    let Filter::ColorCorrect { midtones, .. } = filter_of(&s, effect) else { panic!() };
    assert_eq!(midtones, [0.0, 0.3, 0.5], "a refused edit changes nothing");

    set(&mut s, effect, json!({"midtones": [5.0, -5.0, 5.0]}));
    let Filter::ColorCorrect { midtones, .. } = filter_of(&s, effect) else { panic!() };
    assert_eq!(midtones, [1.0, -1.0, 1.0], "clamped to the ranges");
}

/// HSL Perceptive: a new filter is Perceptive, projects saved before stay Regular; a perceptive hue turn keeps lightness better
#[test]
fn hsl_perceptive_turns_the_hue_at_the_same_lightness() {
    let mut s = session();
    let yellow = [0.8, 0.7, 0.05];
    let effect = filtered(&mut s, yellow, json!({"filter": "hsl", "hue": 0.5}));

    assert!(matches!(filter_of(&s, effect), Filter::Hsl { perceptive: true, .. }), "new filters are perceptive");

    let perceptive = center(&mut s, Channel::BaseColor);
    set(&mut s, effect, json!({"mode": "regular"}));

    assert!(matches!(filter_of(&s, effect), Filter::Hsl { perceptive: false, .. }));

    let regular = center(&mut s, Channel::BaseColor);

    // Rec. 709 luminance weights
    let relative_luminance = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let drift = |color: [f32; 3]| (relative_luminance(color) - relative_luminance(yellow)).abs();
    assert!(drift(perceptive) < drift(regular), "perceptive {perceptive:?} regular {regular:?}");

    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "mode": "vivid"})).is_err());

    // Saved before the mode: Regular, and saved again without the field
    let old: Filter = serde_json::from_value(json!({"Hsl": {"hue": 0.5, "saturation": 0.0, "lightness": 0.0}})).unwrap();

    assert_eq!(old, Filter::Hsl { hue: 0.5, saturation: 0.0, lightness: 0.0, perceptive: false });
    assert_eq!(serde_json::to_value(&old).unwrap(), json!({"Hsl": {"hue": 0.5, "saturation": 0.0, "lightness": 0.0}}));
}

/// Contrast Luminosity starts neutral (contrast 0) like the other colour filters
#[test]
fn contrast_luminosity_starts_neutral() {
    let mut s = session();
    let effect = filtered(&mut s, [0.2; 3], json!({"filter": "contrastLuminosity"}));

    assert_eq!(filter_of(&s, effect), Filter::ContrastLuminosity { contrast: 0.0, luminosity: 0.0 });
    assert!(distance(center(&mut s, Channel::BaseColor), [0.2; 3]) < 4e-3);
}

/// Blur and Bevel take the shared edge wrap: effect.set {wrap}, saved only when on, old projects load with it off
#[test]
fn blur_and_bevel_take_the_edge_wrap() {
    let mut s = session();
    let blur = filtered(&mut s, [0.2; 3], json!({"filter": "blur"}));

    assert!(matches!(filter_of(&s, blur), Filter::Blur { wrap: false, .. }));

    set(&mut s, blur, json!({"wrap": true}));

    assert!(matches!(filter_of(&s, blur), Filter::Blur { wrap: true, .. }));

    assert!(execute(&mut s, "effect.set", &json!({"effect": blur.0, "wrap": "yes"})).is_err());

    let bevel = filtered(&mut s, [0.2; 3], json!({"filter": "bevel", "wrap": true}));

    assert!(matches!(filter_of(&s, bevel), Filter::DistanceBevel(Bevel { wrap: true, .. })));

    let old: Filter = serde_json::from_value(json!({"Blur": {"radius": 4.0}})).unwrap();

    assert_eq!(old, Filter::Blur { radius: 4.0, wrap: false });
    assert_eq!(serde_json::to_value(&old).unwrap(), json!({"Blur": {"radius": 4.0}}), "off is not saved");
}
