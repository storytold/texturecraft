//! Anisotropic Kuwahara, PBR Validate (Metallic Roughness), Height To Normal and Color Match

use serde_json::json;
use texturecraft_engine::core::{
    AnisotropicKuwahara, Channel, ColorMatch, Filter, HeightToNormal, PbrValidate, SourceColorMode, TargetColorMode, ValidationMode,
};
use texturecraft_engine::{Session, execute};

use crate::filter_support::{SIZE, add_filter, filter_of, refused, session, set, sized_session};

/// The composited base color at texel `(x, y)`
fn base_color(s: &mut Session, (x, y): (u32, u32)) -> [f32; 3] {
    s.update();
    s.caches[0].composite.value(Channel::BaseColor, x, y)
}

/// Texels across a set fine enough for filters measured in texels at 2K (Kuwahara's radius, a height's slope)
const FINE_SET_SIZE: u32 = 512;

/// A new Anisotropic Kuwahara starts where the reference does, takes every setting (one undo step) and smooths a noisy fill
#[test]
fn anisotropic_kuwahara_takes_its_settings_and_smooths() {
    let mut s = sized_session(FINE_SET_SIZE);
    let fill = execute(&mut s, "layer.add.fill", &json!({"color": [0.5, 0.5, 0.5]})).unwrap()["layer"].as_u64().unwrap();
    let pattern = json!({"layer": fill, "channel": "baseColor", "procedural": "Clouds", "scale": 64, "low": [0, 0, 0], "high": [1, 1, 1]});
    execute(&mut s, "layer.fill.set", &pattern).unwrap();
    let texels: Vec<(u32, u32)> = (0..32).map(|k| (FINE_SET_SIZE / 4 + k, FINE_SET_SIZE / 2)).collect();
    let before: Vec<f32> = texels.iter().map(|t| base_color(&mut s, *t)[0]).collect();

    let effect = add_filter(&mut s, json!({"filter": "Anisotropic Kuwahara"}));
    assert_eq!(filter_of(&s, effect), Filter::AnisotropicKuwahara(AnisotropicKuwahara::LIBRARY));

    set(&mut s, effect, json!({"radius": AnisotropicKuwahara::MAX_RADIUS}));
    let after: Vec<f32> = texels.iter().map(|t| base_color(&mut s, *t)[0]).collect();
    let steps = |v: &[f32]| v.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>();
    assert!(steps(&after) < steps(&before) * 0.9, "smoother: {} vs {}", steps(&after), steps(&before));

    let all = json!({"extractDirection": "customInput", "radius": 20.0, "smoothness": 0.5, "sharpness": 0.1,
        "tensorSmoothness": 0.3, "anisotropy": 0.4, "angle": 390.0});
    set(&mut s, effect, all);
    let expected = AnisotropicKuwahara {
        custom_input: true,
        radius: AnisotropicKuwahara::MAX_RADIUS,
        smoothness: 0.5,
        sharpness: 0.25,
        tensor_smoothness: 0.3,
        anisotropy: 0.4,
        angle: 30.0,
    };
    assert_eq!(filter_of(&s, effect), Filter::AnisotropicKuwahara(expected), "clamped, the angle wrapped");

    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "extractDirection": "luma"})).is_err());

    let read = base_color(&mut s, texels[0]);
    assert!(read.iter().all(|v| v.is_finite()), "a flat Custom Input reads no direction: {read:?}");

    execute(&mut s, "edit.undo", &json!({})).unwrap();
    let Filter::AnisotropicKuwahara(undone) = filter_of(&s, effect) else { panic!("still Kuwahara") };
    assert_eq!(undone, AnisotropicKuwahara { radius: AnisotropicKuwahara::MAX_RADIUS, ..AnisotropicKuwahara::LIBRARY }, "one undo step");

    let inspect = execute(&mut s, "project.inspect", &json!({})).unwrap().to_string();

    assert!(inspect.contains("Anisotropic Kuwahara"));
}

/// PBR Validate starts with the Color and Metal pills on, shows a too dark non-metal base color in red, leaves the metal as it is and takes its three switches
#[test]
fn pbr_validate_marks_implausible_base_colors() {
    let mut s = session();
    let texel = (SIZE / 4, SIZE / 2);

    // sRGB 20: darker than both dark thresholds
    let dark = 0.007;
    execute(&mut s, "layer.add.fill", &json!({"color": [dark, dark, dark], "metallic": 0.0})).unwrap();
    let effect = add_filter(&mut s, json!({"filter": "pbrValidate"}));
    assert_eq!(filter_of(&s, effect), Filter::PbrValidate(PbrValidate::LIBRARY));

    let off = &s.doc.project.effect(effect).unwrap().1.off_channels;

    assert!(!off.contains(&Channel::BaseColor) && off.contains(&Channel::Metallic) && off.contains(&Channel::Roughness), "{off:?}");
    assert_eq!(base_color(&mut s, texel), PbrValidate::INVALID, "too dark");
    assert_eq!(s.caches[0].composite.value(Channel::Metallic, texel.0, texel.1)[0], 0.0, "the metal is left as it is");

    set(&mut s, effect, json!({"validationMode": "metal"}));
    let shown = base_color(&mut s, texel);
    assert!((shown[0] - dark).abs() < 1e-3, "Metal leaves non-metal colors unchecked: {shown:?}");

    set(&mut s, effect, json!({"validationMode": "albedo", "albedoDarkRangeThreshold": 50, "metalReflectanceRange": 70}));

    assert_eq!(filter_of(&s, effect), Filter::PbrValidate(PbrValidate { mode: ValidationMode::Albedo, albedo_dark: 50, metal_low: 70 }));

    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "albedoDarkRangeThreshold": 40})).is_err());
    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "validationMode": "all"})).is_err());
}

/// The composited packed normal at texel `(x, y)`
fn normal(s: &mut Session, (x, y): (u32, u32)) -> [f32; 3] {
    s.update();
    s.caches[0].composite.value(Channel::Normal, x, y)
}

/// How far the packed normals of `texels` tilt from flat, summed
fn tilt(s: &mut Session, texels: &[(u32, u32)]) -> f32 {
    texels.iter().map(|texel| normal(s, *texel)).map(|[x, y, _]| (x - 0.5).abs() + (y - 0.5).abs()).sum()
}

/// Height To Normal starts with the Normal pill only, turns the layer's height into its normal, overwrites or overlays it, one undo step
#[test]
fn height_to_normal_turns_the_height_into_the_normal() {
    let mut s = sized_session(FINE_SET_SIZE);
    let fill = execute(&mut s, "layer.add.fill", &json!({"color": [0.5, 0.5, 0.5]})).unwrap()["layer"].as_u64().unwrap();
    let bumps = json!({"layer": fill, "channel": "height", "procedural": "Clouds", "scale": 16, "low": [-0.05, -0.05, -0.05], "high": [0.05, 0.05, 0.05]});
    execute(&mut s, "layer.fill.set", &bumps).unwrap();
    let texels: Vec<(u32, u32)> = (0..32).map(|k| (FINE_SET_SIZE / 4 + k, FINE_SET_SIZE / 2)).collect();

    assert!(tilt(&mut s, &texels) < 1e-3, "a fill's own normal is flat");

    let color = base_color(&mut s, texels[0]);

    let effect = add_filter(&mut s, json!({"filter": "heightToNormal"}));
    assert_eq!(filter_of(&s, effect), Filter::HeightToNormal(HeightToNormal::LIBRARY));

    let off = &s.doc.project.effect(effect).unwrap().1.off_channels;

    assert!(!off.contains(&Channel::Normal) && off.contains(&Channel::BaseColor) && off.contains(&Channel::Height), "{off:?}");

    let made = tilt(&mut s, &texels);
    assert!(made > 0.01, "the bumps tilt the normal: {made}");
    assert_eq!(base_color(&mut s, texels[0]), color, "the base color is left as it is");

    set(&mut s, effect, json!({"heightDepth": 64.0}));

    assert!(tilt(&mut s, &texels) > made * 1.5, "deeper heights tilt more");

    set(&mut s, effect, json!({"useWorldUnits": false, "normalIntensity": 0.0}));

    assert!(tilt(&mut s, &texels) < 1e-3, "no intensity: flat");

    execute(&mut s, "layer.fill.set", &json!({"layer": fill, "channel": "normal", "value": [0.7, 0.5, 0.95]})).unwrap();

    assert!(tilt(&mut s, &texels) < 1e-3, "Overwrite Existing Normal replaces the fill's normal");

    set(&mut s, effect, json!({"overwriteExistingNormal": false}));

    assert!((normal(&mut s, texels[0])[0] - 0.7).abs() < 0.02, "laid over the fill's normal: {:?}", normal(&mut s, texels[0]));

    let all = json!({"overwriteExistingNormal": true, "useWorldUnits": true, "surfaceSize": 0.0, "heightDepth": 500.0,
        "normalIntensity": 70.0});
    set(&mut s, effect, all);
    let expected = HeightToNormal {
        surface_size: 1.0,
        height_depth: HeightToNormal::MAX_HEIGHT_DEPTH,
        intensity: HeightToNormal::MAX_INTENSITY,
        ..HeightToNormal::LIBRARY
    };
    assert_eq!(filter_of(&s, effect), Filter::HeightToNormal(expected), "clamped");
    assert!(normal(&mut s, texels[0]).iter().all(|v| v.is_finite()));

    execute(&mut s, "edit.undo", &json!({})).unwrap();
    let Filter::HeightToNormal(undone) = filter_of(&s, effect) else { panic!("still Height To Normal") };
    assert!(!undone.overwrite && undone.surface_size == 300.0, "one undo step: {undone:?}");

    assert!(execute(&mut s, "effect.set", &json!({"effect": effect.0, "useWorldUnits": "yes"})).is_err());

    let inspect = execute(&mut s, "project.inspect", &json!({})).unwrap().to_string();

    assert!(inspect.contains("Height To Normal"));
}

/// Color Match starts with the Color pill only and moves the colors to its target keeping their pattern, all of them at the Color Variation's 0, keeping each texel's difference from the whole texture's mean without Custom Color Variation; Parameter and Image Input modes read their colors
#[test]
fn color_match_moves_the_colors_to_the_target() {
    let mut s = session();
    let fill = execute(&mut s, "layer.add.fill", &json!({"color": [0.5, 0.5, 0.5]})).unwrap()["layer"].as_u64().unwrap();
    let reds = json!({"layer": fill, "channel": "baseColor", "procedural": "Clouds", "scale": 8, "low": [0.3, 0.02, 0.02], "high": [0.9, 0.1, 0.1]});
    execute(&mut s, "layer.fill.set", &reds).unwrap();
    let texels: Vec<(u32, u32)> = (0..24).map(|k| (SIZE / 4 + k * 2, SIZE / 2)).collect();
    let before: Vec<[f32; 3]> = texels.iter().map(|t| base_color(&mut s, *t)).collect();
    let spread = |colors: &[[f32; 3]]| colors.iter().map(|c| c[0]).fold(f32::MIN, f32::max) - colors.iter().map(|c| c[0]).fold(f32::MAX, f32::min);

    assert!(spread(&before) > 0.05, "the clouds vary: {before:?}");

    let effect = add_filter(&mut s, json!({"filter": "Color Match"}));
    assert_eq!(filter_of(&s, effect), Filter::ColorMatch(ColorMatch::LIBRARY));

    let off = &s.doc.project.effect(effect).unwrap().1.off_channels;

    assert!(!off.contains(&Channel::BaseColor) && off.contains(&Channel::Roughness), "{off:?}");

    let new: Vec<[f32; 3]> = texels.iter().map(|t| base_color(&mut s, *t)).collect();

    assert!(spread(&new) > 0.02, "a new Color Match keeps the pattern: {new:?}");

    let redness = new.iter().map(|c| c[0] - c[2]).sum::<f32>() / new.len() as f32;
    assert!(redness.abs() < 0.05, "around the gray target: {redness} {new:?}");

    // Color Variation at 0: every texel is the target itself
    set(&mut s, effect, json!({"hue": 0.0, "chroma": 0.0, "luma": 0.0}));
    let gray = ColorMatch::LIBRARY.target_color;
    for texel in &texels {
        let color = base_color(&mut s, *texel);
        assert!(color.iter().zip(gray).all(|(v, g)| (v - g).abs() < 2e-3), "Color Variation 0: the target itself, {color:?}");
    }

    // Without Custom Color Variation the texels keep their differences from the mean, now around the target
    set(&mut s, effect, json!({"customColorVariation": false}));
    let kept: Vec<[f32; 3]> = texels.iter().map(|t| base_color(&mut s, *t)).collect();

    assert!(spread(&kept) > 0.02, "variation kept: {kept:?}");

    let redness = kept.iter().map(|c| c[0] - c[2]).sum::<f32>() / kept.len() as f32;
    assert!(redness.abs() < 0.05, "the reds are gray on average now: {redness} {kept:?}");

    let brighter = |colors: &[[f32; 3]], a: usize, b: usize| colors[a][0] + colors[a][1] > colors[b][0] + colors[b][1];
    let (lightest, darkest) =
        (0..texels.len()).fold((0, 0), |(hi, lo), k| (if before[k][0] > before[hi][0] { k } else { hi }, if before[k][0] < before[lo][0] { k } else { lo }));

    assert!(brighter(&kept, lightest, darkest), "the lightest red stays the lightest");

    // Source Color Parameter: the texels of that color land on the target
    let source = before[lightest];
    set(&mut s, effect, json!({"sourceColorMode": "parameter", "sourceColor": source, "targetColor": [0.05, 0.1, 0.6]}));
    let landed = base_color(&mut s, texels[lightest]);
    assert!((landed[2] - 0.6).abs() < 0.01 && (landed[0] - 0.05).abs() < 0.01, "{landed:?}");

    // Image Input: mid gray until a source is set
    let all = json!({"sourceColorMode": "input", "targetColorMode": "imageInput", "customColorVariation": true, "hue": 400.0,
        "chroma": 2.0, "luma": -1.0});
    set(&mut s, effect, all);
    let expected = ColorMatch {
        source_mode: SourceColorMode::Input,
        source_color: source,
        target_mode: TargetColorMode::ImageInput,
        target_color: [0.05, 0.1, 0.6],
        custom_variation: true,
        hue: 360.0,
        chroma: 1.0,
        luma: 0.0,
    };
    assert_eq!(filter_of(&s, effect), Filter::ColorMatch(expected), "clamped");

    set(&mut s, effect, json!({"chroma": 0.0, "hue": 0.0}));
    let image = base_color(&mut s, texels[0]);
    assert!(image.iter().all(|v| (v - 0.5).abs() < 2e-3), "the Image Input's mid gray: {image:?}");

    execute(&mut s, "edit.undo", &json!({})).unwrap();
    let Filter::ColorMatch(undone) = filter_of(&s, effect) else { panic!("still Color Match") };
    assert_eq!((undone.hue, undone.chroma), (360.0, 1.0), "one undo step: {undone:?}");

    refused(
        &mut s,
        effect,
        &[json!({"sourceColorMode": "median"}), json!({"targetColorMode": "input"}), json!({"customColorVariation": 1}), json!({"targetColor": "red"})],
    );
    let inspect = execute(&mut s, "project.inspect", &json!({})).unwrap().to_string();

    assert!(inspect.contains("Color Match"));
}
