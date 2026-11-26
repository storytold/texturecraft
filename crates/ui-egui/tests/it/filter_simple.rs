//! The simple adjustment filters in Properties: Invert's switch, Height Adjust's Invert above its sliders, Histogram Scan's
//! Invert Position and Posterize's Apply to Alpha under theirs, Pixelate and FXAA (Anti-aliasing)'s switches, and the reference's names on
//! Clamp, Smoothstep, Histogram Shift, Sharpen, Blur and Blur Directional; Highpass's and Grayscale Conversion's rows; the simple sliders
//! (Contrast Luminosity, Clamp, Smoothstep, Sharpen, Histogram Range, Edge Detect, Replace Color) setting their own setting, a drag as one undo step

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::core::{ConversionMode, Filter, HighpassSpace};

use crate::filter_support::{WINDOW_HEIGHT, along, app_with, click_at, drag, filter_of, lowest, pick, undo, undo_depth};

/// Invert's only row is its switch: a click turns it off, another back on
#[test]
fn invert_switch_turns_it_off_and_on() {
    let (mut harness, effect) = app_with("invert", "mask", WINDOW_HEIGHT);
    let switch = harness.get_by_role_and_label(Role::CheckBox, "Invert").rect();

    click_at(&mut harness, switch.center());

    assert_eq!(filter_of(&harness, effect), Filter::InvertOff);

    let switch = harness.get_by_role_and_label(Role::CheckBox, "Invert").rect();
    click_at(&mut harness, switch.center());

    assert_eq!(filter_of(&harness, effect), Filter::Invert);
}

/// Height Adjust: Invert above Offset and Multiply; the switch flips it and Multiply's track sets it
#[test]
fn height_adjust_rows() {
    let (mut harness, effect) = app_with("heightAdjust", "mask", WINDOW_HEIGHT);
    let invert = harness.get_by_role_and_label(Role::CheckBox, "Invert").rect();
    let offset = harness.get_by_role_and_label(Role::Slider, "Offset").rect();
    let multiply = harness.get_by_role_and_label(Role::Slider, "Multiply").rect();
    assert!(invert.center().y < offset.center().y && offset.center().y < multiply.center().y, "Invert, Offset, Multiply");

    click_at(&mut harness, invert.center());
    click_at(&mut harness, along(multiply, 1.0));

    let Filter::HeightAdjust { invert, offset, multiply } = filter_of(&harness, effect) else { panic!() };
    assert!(invert && offset == 0.0 && multiply > 9.5, "{invert} {offset} {multiply}");
}

/// Histogram Scan's Invert Position sits under Position and Contrast
#[test]
fn histogram_scan_switch_under_its_sliders() {
    let (mut harness, effect) = app_with("histogramScan", "mask", WINDOW_HEIGHT);
    let contrast = harness.get_by_role_and_label(Role::Slider, "Contrast").rect();
    let invert = harness.get_by_role_and_label(Role::CheckBox, "Invert Position").rect();
    assert!(invert.center().y > contrast.center().y);

    click_at(&mut harness, invert.center());

    assert!(matches!(filter_of(&harness, effect), Filter::HistogramScan { invert_position: true, .. }));
}

/// Posterize's Apply to Alpha sits under Colors Amount
#[test]
fn posterize_switch_under_its_slider() {
    let (mut harness, effect) = app_with("posterize", "mask", WINDOW_HEIGHT);
    let amount = harness.get_by_role_and_label(Role::Slider, "Colors Amount").rect();
    let alpha = harness.get_by_role_and_label(Role::CheckBox, "Apply to Alpha").rect();
    assert!(alpha.center().y > amount.center().y);

    click_at(&mut harness, alpha.center());

    assert_eq!(filter_of(&harness, effect), Filter::Posterize { levels: 8.0, alpha: true });
}

/// Histogram Shift's Position: a click on its middle shifts by a half turn back
#[test]
fn histogram_shift_position_middle_is_a_half_turn_back() {
    let (mut harness, effect) = app_with("histogramShift", "mask", WINDOW_HEIGHT);
    let position = harness.get_by_role_and_label(Role::Slider, "Position").rect();

    click_at(&mut harness, position.center());

    let Filter::HistogramShift { offset } = filter_of(&harness, effect) else { panic!() };
    assert!((offset + 0.5).abs() < 0.05, "the middle is a half turn back: {offset}");
}

/// The setting slider case `label` of a filter sets, read back from the filter
type ReadSetting = fn(&Filter) -> Option<f32>;

/// One slider per case, clicked at `share` of its track: (library filter, slider, share, the setting it sets, the value expected)
const SLIDER_CASES: [(&str, &str, f32, ReadSetting, f32); 12] = [
    ("contrastLuminosity", "Contrast", 1.0, |filter| if let Filter::ContrastLuminosity { contrast, .. } = filter { Some(*contrast) } else { None }, 1.0),
    ("contrastLuminosity", "Luminosity", 0.0, |filter| if let Filter::ContrastLuminosity { luminosity, .. } = filter { Some(*luminosity) } else { None }, -1.0),
    ("clamp", "Min", 0.5, |filter| if let Filter::Clamp { min, .. } = filter { Some(*min) } else { None }, 0.5),
    ("clamp", "Max", 0.5, |filter| if let Filter::Clamp { max, .. } = filter { Some(*max) } else { None }, 0.5),
    ("smoothstep", "Minimum", 0.5, |filter| if let Filter::Smoothstep { min, .. } = filter { Some(*min) } else { None }, 0.5),
    ("smoothstep", "Maximum", 0.5, |filter| if let Filter::Smoothstep { max, .. } = filter { Some(*max) } else { None }, 0.5),
    ("sharpen", "Sharpen Intensity", 1.0, |filter| if let Filter::Sharpen { amount } = filter { Some(*amount) } else { None }, 4.0),
    ("histogramRange", "Range", 1.0, |filter| if let Filter::HistogramRange { range, .. } = filter { Some(*range) } else { None }, 1.0),
    ("histogramRange", "Position", 0.0, |filter| if let Filter::HistogramRange { position, .. } = filter { Some(*position) } else { None }, 0.0),
    ("edgeDetect", "Width", 0.5, |filter| if let Filter::EdgeDetect { width } = filter { Some(*width) } else { None }, 32.0),
    ("replaceColor", "Tolerance", 1.0, |filter| if let Filter::ReplaceColor { tolerance, .. } = filter { Some(*tolerance) } else { None }, 1.0),
    ("replaceColor", "Softness", 1.0, |filter| if let Filter::ReplaceColor { softness, .. } = filter { Some(*softness) } else { None }, 1.0),
];

/// How near a clicked slider's setting must land to the value expected
const SLIDER_TOLERANCE: f32 = 0.01;

/// Each simple filter's sliders set their own setting through the pointer
#[test]
fn simple_sliders_set_their_own_setting() {
    for (filter, label, share, read, expected) in SLIDER_CASES {
        let (mut harness, effect) = app_with(filter, "content", WINDOW_HEIGHT);
        let before = filter_of(&harness, effect);
        let track = harness.get_by_role_and_label(Role::Slider, label).rect();

        click_at(&mut harness, along(track, share));

        let after = filter_of(&harness, effect);
        let value = read(&after).unwrap();

        assert!((value - expected).abs() < SLIDER_TOLERANCE, "{filter} {label}: {value}, expected {expected}");
        assert_ne!(read(&before), read(&after), "{filter} {label} changed");
    }
}

/// A slider drag is one undo step, and undo puts the value back
#[test]
fn a_slider_drag_is_one_undo_step() {
    let (mut harness, effect) = app_with("sharpen", "content", WINDOW_HEIGHT);
    let depth = undo_depth(&harness);
    let track = harness.get_by_role_and_label(Role::Slider, "Sharpen Intensity").rect();

    drag(&mut harness, along(track, 0.5), along(track, 1.0));

    assert!(matches!(filter_of(&harness, effect), Filter::Sharpen { amount } if amount > 3.9), "{:?}", filter_of(&harness, effect));
    assert_eq!(undo_depth(&harness), depth + 1);

    undo(&mut harness);

    assert_eq!(filter_of(&harness, effect), Filter::Sharpen { amount: 1.0 });
}

/// Pixelate: Pixels Amount (32) with Apply To Alpha under it
#[test]
fn pixelate_rows() {
    let (mut harness, effect) = app_with("pixelate", "mask", WINDOW_HEIGHT);
    let amount = harness.get_by_role_and_label(Role::Slider, "Pixels Amount");
    assert_eq!(amount.accesskit_node().numeric_value().map(f64::round), Some(32.0));

    let amount = amount.rect();
    let alpha = harness.get_by_role_and_label(Role::CheckBox, "Apply To Alpha").rect();
    assert!(alpha.center().y > amount.center().y);

    click_at(&mut harness, alpha.center());

    assert!(matches!(filter_of(&harness, effect), Filter::Mosaic { alpha: false, .. }));

    click_at(&mut harness, along(amount, 1.0));
    let Filter::Mosaic { size, .. } = filter_of(&harness, effect) else { panic!() };
    assert!((Filter::pixels_amount(size) - 128.0).abs() < 0.5, "the right end is 128 across: {size}");
}

/// FXAA (Anti-aliasing): only its Affect Alpha Channel switch
#[test]
fn fxaa_rows() {
    let (mut harness, effect) = app_with("fxaa", "mask", WINDOW_HEIGHT);
    let affect = harness.get_by_role_and_label(Role::CheckBox, "Affect Alpha Channel").rect();

    click_at(&mut harness, affect.center());

    assert_eq!(filter_of(&harness, effect), Filter::Fxaa { alpha: true });
}

/// Blur Directional's Intensity and Angle take the reference's names, and Intensity's right end is the widest blur
#[test]
fn blur_directional_rows_take_the_reference_names() {
    let (mut harness, effect) = app_with("blurDirectional", "mask", WINDOW_HEIGHT);
    let intensity = harness.get_by_role_and_label(Role::Slider, "Intensity").rect();
    assert!(harness.query_by_role_and_label(Role::Slider, "Angle").is_some());

    click_at(&mut harness, along(intensity, 1.0));

    assert!(matches!(filter_of(&harness, effect), Filter::DirectionalBlur { radius, .. } if radius > 250.0));
}

/// Highpass: Color Space, Radius and Apply To Alpha, each setting its own value
#[test]
fn highpass_rows() {
    let (mut harness, effect) = app_with("highPass", "mask", WINDOW_HEIGHT);
    let radius = harness.get_by_role_and_label(Role::Slider, "Radius");
    assert_eq!(radius.accesskit_node().numeric_value().map(|value| (value * 100.0).round()), Some(300.0));

    let radius = radius.rect();
    let space = harness.get_by_role_and_label(Role::ComboBox, "Working space / Mask").rect();
    let alpha = harness.get_by_role_and_label(Role::CheckBox, "Apply To Alpha").rect();
    assert!(space.center().y < radius.center().y && radius.center().y < alpha.center().y, "the reference's order");

    pick(&mut harness, "Working space / Mask", "sRGB (from ACEScg)");
    click_at(&mut harness, alpha.center());

    assert_eq!(filter_of(&harness, effect), Filter::HighPass { radius: 48.0, space: HighpassSpace::SrgbFromAcescg, alpha: true });

    click_at(&mut harness, along(radius, 1.0));

    assert!(matches!(filter_of(&harness, effect), Filter::HighPass { radius, .. } if radius > 250.0), "Radius 16 = 256 texels at 2K");
}

/// Grayscale Conversion: Channel Input (Custom Input shows its slot right under it), Invert, Balance, Contrast, Mode and the weights in Channels Weights only
#[test]
fn grayscale_conversion_rows() {
    let (mut harness, effect) = app_with("grayscale", "mask", WINDOW_HEIGHT);
    let invert = harness.get_by_role_and_label(Role::CheckBox, "Invert").rect().center().y;
    let sliders = ["Balance", "Contrast", "Red", "Green", "Blue", "Alpha"].map(|label| harness.get_by_role_and_label(Role::Slider, label).rect().center().y);
    let rows: Vec<f32> = std::iter::once(invert).chain(sliders).collect();

    assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "the reference's order: {rows:?}");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Custom Input").is_none(), "no slot for the current channel");

    pick(&mut harness, "Current Channel", "Custom Input");

    let Filter::GrayscaleConversion(conversion) = filter_of(&harness, effect) else { panic!() };
    assert!(conversion.custom_input);

    // The lowest: Channel Input shows the same name where its words don't fit a switch
    let slot = lowest(&harness, Role::ComboBox, "Custom Input");
    let invert = harness.get_by_role_and_label(Role::CheckBox, "Invert").rect();
    assert!(slot.center().y < invert.center().y, "its slot right under Channel Input");

    click_at(&mut harness, invert.center());
    pick(&mut harness, "Channels Weights", "Max");

    let Filter::GrayscaleConversion(conversion) = filter_of(&harness, effect) else { panic!() };
    assert!(conversion.invert && conversion.mode == ConversionMode::Max);
    assert!(harness.query_by_label("Red").is_none(), "no weights outside Channels Weights");
}
