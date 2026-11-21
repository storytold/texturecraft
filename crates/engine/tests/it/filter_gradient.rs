//! The gradient filters: Gradient (positions, quantity, input transformation, linear blending, debug), Gradient Curve and Gradient Dynamic

use serde_json::json;
use texturecraft_engine::core::{Channel, EffectKind, Filter, GrayscaleMode, layer::gradient};
use texturecraft_engine::execute;
use texturecraft_engine::filter_lib::{LIBRARY_GRADIENT_CURVE, LIBRARY_GRADIENT_DYNAMIC, MID_GRAY};

use crate::filter_support::{add_filter, center, center_uv, facing_session as session, filter_of, filtered, refused, set};

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 6e-3
}

/// Projects saved before read a gradient as evenly spaced colors from the texel's luma, blended in linear, and save it as before
#[test]
fn a_gradient_saved_before_reads_and_saves_as_before() {
    let saved = json!({"GradientMap": {"colors": [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]]}});
    let filter: Filter = serde_json::from_value(saved.clone()).unwrap();

    assert_eq!(filter, Filter::even_gradient(vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]]));
    assert_eq!(serde_json::to_value(&filter).unwrap(), saved, "nothing new is saved");

    let Filter::GradientMap { colors, input, .. } = &filter else { panic!() };
    assert_eq!(input.mode, GrayscaleMode::Luma);

    for t in [0.0, 0.2, 0.37, 0.5, 0.81, 1.0] {
        assert_eq!(filter.ramp_at(t), gradient(colors, t), "bit for bit as before at {t}");
    }
}

/// The library's Gradient: black, mid gray and white at 0, 0.5 and 1; places, quantity, invert, balance and linear blending move what a texel reads
#[test]
fn gradient_places_quantity_and_input_transformation() {
    let mut s = session();
    let effect = filtered(&mut s, [0.25; 3], json!({"filter": "gradientMap"}));
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Gradient");

    let Filter::GradientMap { quantity, input, linear, .. } = filter_of(&s, effect) else { panic!() };
    assert_eq!((quantity, input.mode, linear), (Some(3), GrayscaleMode::Average, true));

    assert!(near(center(&mut s, Channel::BaseColor)[0], MID_GRAY / 2.0), "halfway from black to mid gray");

    set(&mut s, effect, json!({"positions": [0.0, 0.25, 1.0]}));

    assert!(near(center(&mut s, Channel::BaseColor)[0], MID_GRAY), "mid gray moved to 0.25");

    set(&mut s, effect, json!({"invert": true}));

    assert!(near(center(&mut s, Channel::BaseColor)[0], MID_GRAY + (1.0 - MID_GRAY) * (0.5 / 0.75)), "inverted, it reads 0.75");

    set(&mut s, effect, json!({"invert": false, "balance": 0.75}));

    assert!(near(center(&mut s, Channel::BaseColor)[0], MID_GRAY + (1.0 - MID_GRAY) * (0.25 / 0.75)), "balance 0.75 lifts it to 0.5");

    set(&mut s, effect, json!({"balance": 0.5, "positions": [0.0, 0.5, 1.0], "linear": false}));
    let srgb = center(&mut s, Channel::BaseColor)[0];
    assert!(near(srgb, 0.0508), "blended in sRGB, halfway from black to mid gray is sRGB 0.25: {srgb}");

    set(&mut s, effect, json!({"quantity": 4}));

    assert_eq!(filter_of(&s, effect).ramp_colors().len(), 4);

    refused(
        &mut s,
        effect,
        &[
            json!({"quantity": 11}),
            json!({"quantity": 1}),
            json!({"grayscaleMode": "x"}),
            json!({"positions": [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0]}),
            json!({"debug": 3}),
        ],
    );

    set(&mut s, effect, json!({"grayscaleMode": "max"}));
    let Filter::GradientMap { input, .. } = filter_of(&s, effect) else { panic!() };
    assert_eq!(input.mode, GrayscaleMode::Max);
}

/// A new gradient given its colors alone ramps evenly through every one of them
#[test]
fn a_gradient_given_colors_alone_ramps_evenly_through_all_of_them() {
    let mut s = session();

    let two = filtered(&mut s, [0.5; 3], json!({"filter": "gradientMap", "colors": [[0, 0, 0], [1, 1, 1]]}));
    let mid = center(&mut s, Channel::BaseColor)[0];
    assert!(near(mid, 0.5), "black to white reads mid gray halfway: {mid}");

    execute(&mut s, "effect.remove", &json!({"effect": two.0})).unwrap();
    let five = add_filter(&mut s, json!({"filter": "gradientMap", "colors": [[0, 0, 0], [0.25, 0.25, 0.25], [0.5, 0.5, 0.5], [0.75, 0.75, 0.75], [1, 1, 1]]}));
    assert_eq!(filter_of(&s, five).ramp_colors().len(), 5, "every color is used");
}

/// Display Gradient (Debug) shows the ramp left to right across the texture
#[test]
fn display_gradient_shows_the_ramp_across_the_texture() {
    let mut s = session();
    let effect = filtered(&mut s, [0.9; 3], json!({"filter": "gradientMap", "debug": true}));
    let (value, uv) = (center(&mut s, Channel::BaseColor), center_uv(&mut s));
    let ramp = filter_of(&s, effect).ramp_at(uv.x).unwrap();

    assert!((0..3).all(|k| (value[k] - ramp[k]).abs() < 0.03), "{value:?} vs {ramp:?} at u {}", uv.x);
}

/// An old gradient that gains a quantity keeps its colors' places and adds the new one at 1
#[test]
fn a_saved_gradient_keeps_its_places_when_it_gains_a_color() {
    let mut s = session();
    let effect = filtered(&mut s, [0.5; 3], json!({"filter": "gradientMap"}));
    if let Some(found) = s.doc.project.effect_mut(effect) {
        found.kind = EffectKind::Filter(Filter::even_gradient(vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0; 3]]));
    }
    set(&mut s, effect, json!({"quantity": 4}));
    let Filter::GradientMap { colors, positions, quantity, .. } = filter_of(&s, effect) else { panic!() };
    assert_eq!((positions, quantity), (vec![0.0, 0.5, 1.0, 1.0], Some(4)));
    assert_eq!(colors.last(), Some(&[1.0; 3]), "the new color starts as the last one");
}

/// A color added at 1 starts as the ramp's color there: its rightmost stop's, not the last one listed
#[test]
fn an_added_color_keeps_the_ramps_end() {
    let mut s = session();
    let effect = filtered(&mut s, [0.5; 3], json!({"filter": "gradientMap", "colors": [[1, 1, 1], [0, 0, 0]]}));
    set(&mut s, effect, json!({"positions": [0.8, 0.2]}));
    let before = filter_of(&s, effect).ramp_at(0.9);

    set(&mut s, effect, json!({"quantity": 3}));

    assert_eq!(before, Some([1.0; 3]), "white past its stop at 0.8");
    assert_eq!(filter_of(&s, effect).ramp_at(0.9), before, "the added color at 1 keeps the end white");
}

/// Gradient Curve: values remapped through the preset after balance, contrast and invert
#[test]
fn gradient_curve_remaps_through_its_preset() {
    let mut s = session();
    let effect = filtered(&mut s, [0.25; 3], json!({"filter": "gradientCurve"}));
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Gradient Curve");
    assert_eq!(filter_of(&s, effect), LIBRARY_GRADIENT_CURVE, "the workflow's defaults");

    let round = (1.0f32 - 0.75 * 0.75).sqrt();
    assert!(near(center(&mut s, Channel::BaseColor)[0], round), "Round at 0.25");

    set(&mut s, effect, json!({"preset": "easeIn"}));

    assert!(near(center(&mut s, Channel::BaseColor)[0], 0.0625), "Ease In at 0.25");

    set(&mut s, effect, json!({"invert": true}));

    assert!(near(center(&mut s, Channel::BaseColor)[0], 0.5625), "Ease In at 0.75");

    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "preset": "spiral"})).is_err());

    let Filter::GradientCurve { preset, .. } = filter_of(&s, effect) else { panic!() };
    assert_eq!(preset.id(), "easeIn", "a refused preset changes nothing");
}

/// Gradient Dynamic with a uniform Gradient Source: every texel reads that color (scalars its gray)
#[test]
fn gradient_dynamic_reads_its_uniform_source() {
    let mut s = session();
    let effect = filtered(&mut s, [0.9, 0.1, 0.1], json!({"filter": "gradientDynamic", "color": [0.2, 0.4, 0.6]}));
    assert_eq!(s.doc.project.effect(effect).unwrap().1.name, "Gradient Dynamic");

    let Filter::GradientDynamic { vertical, position, .. } = LIBRARY_GRADIENT_DYNAMIC else { panic!() };
    assert_eq!(filter_of(&s, effect), Filter::GradientDynamic { vertical, position, color: [0.2, 0.4, 0.6] }, "the workflow's defaults, the color given");

    let color = center(&mut s, Channel::BaseColor);
    assert!(near(color[0], 0.2) && near(color[1], 0.4) && near(color[2], 0.6), "{color:?}");

    set(&mut s, effect, json!({"orientation": "vertical", "position": 0.7}));
    let Filter::GradientDynamic { vertical, position, .. } = filter_of(&s, effect) else { panic!() };
    assert!(vertical && position == 0.7);

    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "orientation": "diagonal"})).is_err());
}
