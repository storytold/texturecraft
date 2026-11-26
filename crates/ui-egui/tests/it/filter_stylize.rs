//! Anisotropic Kuwahara's, PBR Validate (Metallic Roughness)'s, Height To Normal's and Color Match's rows in Properties

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::core::{
    AnisotropicKuwahara, ColorMatch, Filter, HeightToNormal, LayerId, PbrValidate, SourceColorMode, TargetColorMode, ValidationMode,
};
use texturecraft_ui_egui::TextureCraftApp;

use crate::filter_support::{TALL_WINDOW_HEIGHT, WINDOW_HEIGHT, along, app_with, click_at, filter_of, lowest, pick, undo_depth};

/// Anisotropic Kuwahara: Extract Direction (its Custom Input slot right under it), then Radius … Anisotropy and the Anisotropy Angle dial, in the reference's order
#[test]
fn anisotropic_kuwahara_rows() {
    let (mut harness, effect) = app_with("anisotropicKuwahara", "mask", WINDOW_HEIGHT);
    let rows = ["Radius", "Smoothness", "Sharpness", "Tensor Smoothness", "Anisotropy", "Anisotropy Angle"]
        .map(|label| harness.get_by_role_and_label(Role::Slider, label).rect().center().y);
    assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "the reference's order: {rows:?}");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Custom Input").is_none(), "no slot for the current channel");

    pick(&mut harness, "Current Channel", "Custom Input");

    let Filter::AnisotropicKuwahara(kuwahara) = filter_of(&harness, effect) else { panic!("still Kuwahara") };
    assert!(kuwahara.custom_input);

    let slot = lowest(&harness, Role::ComboBox, "Custom Input");
    let radius = harness.get_by_role_and_label(Role::Slider, "Radius").rect();
    assert!(slot.center().y < radius.center().y, "its slot right under Extract Direction");

    // A click at the left end of Sharpness's track reaches its least value, the library's
    let sharpness = harness.get_by_role_and_label(Role::Slider, "Sharpness").rect();
    click_at(&mut harness, along(sharpness, 0.0));

    let Filter::AnisotropicKuwahara(kuwahara) = filter_of(&harness, effect) else { panic!("still Kuwahara") };
    assert!((kuwahara.sharpness - AnisotropicKuwahara::LIBRARY.sharpness).abs() < 0.02, "{}", kuwahara.sharpness);
}

/// The height of the label line a stacked setting shows above its control (the panel style's stack rhythm)
const STACK_LABEL_LINE: f32 = 18.0;

/// PBR Validate: three switches, each label above its switch across the panel; each click sets its own setting
#[test]
fn pbr_validate_rows() {
    let (mut harness, effect) = app_with("pbrValidate", "content", TALL_WINDOW_HEIGHT);
    let firsts = ["Albedo", "50 sRGB", "70-100% Reflective"].map(|option| harness.get_by_role_and_label(Role::RadioButton, option).rect());
    for pair in firsts.windows(2) {
        assert!((pair[0].left() - pair[1].left()).abs() < 1.0, "every switch starts at the panel's edge: {pair:?}");
        assert!(pair[1].top() - pair[0].bottom() >= STACK_LABEL_LINE, "a label line between the switches: {pair:?}");
    }
    let last = harness.get_by_role_and_label(Role::RadioButton, "60-100% Reflective").rect();
    let combined = harness.get_by_role_and_label(Role::RadioButton, "Combined").rect();
    assert!((last.right() - combined.right()).abs() < 1.0, "all as wide: {last:?} {combined:?}");

    for option in ["Albedo", "50 sRGB", "70-100% Reflective"] {
        let segment = harness.get_by_role_and_label(Role::RadioButton, option).rect();
        click_at(&mut harness, segment.center());
    }

    assert_eq!(filter_of(&harness, effect), Filter::PbrValidate(PbrValidate { mode: ValidationMode::Albedo, albedo_dark: 50, metal_low: 70 }));
}

/// Where along Surface Size's track the click lands: away from the default and between whole centimetres, so rounding shows
const OFF_DEFAULT_SHARE: f32 = 0.37;

/// A click on Surface Size that rounds to the centimetres it holds changes nothing, so it adds no undo step
#[test]
fn height_to_normal_click_rounding_to_the_stored_size_adds_no_step() {
    let (mut harness, effect) = app_with("heightToNormal", "mask", WINDOW_HEIGHT);
    let size = harness.get_by_role_and_label(Role::Slider, "Surface Size (cm)").rect();
    click_at(&mut harness, along(size, OFF_DEFAULT_SHARE));
    let (depth, after_first) = (undo_depth(&harness), filter_of(&harness, effect));

    click_at(&mut harness, along(size, OFF_DEFAULT_SHARE));

    assert_eq!(filter_of(&harness, effect), after_first);
    assert_eq!(undo_depth(&harness), depth, "the same whole centimetres again: no new step");
}

/// Height To Normal: its two switches, then whole Surface Size (cm) and Height Depth (cm), or Normal Intensity without world units
#[test]
fn height_to_normal_rows() {
    let (mut harness, effect) = app_with("heightToNormal", "mask", WINDOW_HEIGHT);
    let rows = [
        (Role::CheckBox, "Overwrite Existing Normal"),
        (Role::CheckBox, "Use World Units"),
        (Role::Slider, "Surface Size (cm)"),
        (Role::Slider, "Height Depth (cm)"),
    ]
    .map(|(role, label)| harness.get_by_role_and_label(role, label).rect().center().y);
    assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "the reference's order: {rows:?}");

    // A click on Surface Size's track sets whole centimetres
    let size = harness.get_by_role_and_label(Role::Slider, "Surface Size (cm)").rect();
    click_at(&mut harness, along(size, OFF_DEFAULT_SHARE));

    let Filter::HeightToNormal(filter) = filter_of(&harness, effect) else { panic!("still Height To Normal") };
    assert!(filter.surface_size != 300.0 && filter.surface_size.fract() == 0.0, "{}", filter.surface_size);
    assert!(harness.query_by_role_and_label(Role::Slider, "Normal Intensity").is_none());

    let overwrite = harness.get_by_role_and_label(Role::CheckBox, "Overwrite Existing Normal").rect();
    click_at(&mut harness, overwrite.center());
    let world = harness.get_by_role_and_label(Role::CheckBox, "Use World Units").rect();
    click_at(&mut harness, world.center());

    let Filter::HeightToNormal(filter) = filter_of(&harness, effect) else { panic!("still Height To Normal") };
    assert_eq!((filter.overwrite, filter.world_units, filter.intensity), (false, false, HeightToNormal::LIBRARY.intensity));
    assert!(harness.query_by_role_and_label(Role::Slider, "Surface Size (cm)").is_none());

    // A click at the left end of Normal Intensity's track reaches 0
    let intensity = harness.get_by_role_and_label(Role::Slider, "Normal Intensity").rect();
    click_at(&mut harness, along(intensity, 0.0));

    let Filter::HeightToNormal(filter) = filter_of(&harness, effect) else { panic!("still Height To Normal") };
    assert!(filter.intensity < 0.5, "{}", filter.intensity);
}

/// The Color Match settings effect `effect` holds now
fn color_match_of(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> ColorMatch {
    let Filter::ColorMatch(matcher) = filter_of(harness, effect) else { panic!("still Color Match") };
    matcher
}

/// Color Match: Source Color Mode (a list: its three words don't fit a switch), Target Color Mode (a switch, a list where its words don't fit), Target Color, Custom Color Variation and its Color Variation card; each mode's color or image slot right under it, clicked through the pointer
#[test]
fn color_match_rows() {
    let (mut harness, effect) = app_with("colorMatch", "content", TALL_WINDOW_HEIGHT);

    // Target Color Mode's control: its switch's segment, or its list where the words don't fit
    let target_mode = |harness: &Harness<'_, TextureCraftApp>, shown: &str| {
        harness
            .query_all_by(|node| matches!(node.role(), Role::RadioButton | Role::ComboBox) && node.label().as_deref() == Some(shown))
            .map(|node| node.rect())
            .min_by(|a, b| a.top().total_cmp(&b.top()))
            .unwrap()
    };
    let rows = [
        harness.get_by_role_and_label(Role::ComboBox, "Average").rect(),
        target_mode(&harness, "Parameter"),
        harness.get_by_role_and_label(Role::Button, "Target Color").rect(),
        harness.get_by_role_and_label(Role::CheckBox, "Custom Color Variation").rect(),
        harness.get_by_role_and_label(Role::Slider, "Hue").rect(),
        harness.get_by_role_and_label(Role::Slider, "Chroma").rect(),
        harness.get_by_role_and_label(Role::Slider, "Luma").rect(),
    ]
    .map(|rect| rect.center().y);
    assert!(rows.windows(2).all(|pair| pair[0] < pair[1]), "the reference's order: {rows:?}");
    assert!(harness.query_by_role_and_label(Role::Button, "Source Color").is_none(), "Average shows no Source Color");

    pick(&mut harness, "Parameter", "Image Input");

    assert_eq!(color_match_of(&harness, effect).target_mode, TargetColorMode::ImageInput);
    assert!(harness.query_by_role_and_label(Role::Button, "Target Color").is_none(), "the Image Input slot replaces Target Color");

    let shown = harness.query_all_by_label("Image Input").count();
    assert!(shown >= 2, "the mode and its slot: {shown}");

    // The source list's own Parameter row (not a control showing Parameter)
    let list = harness.get_by_role_and_label(Role::ComboBox, "Average").rect();
    click_at(&mut harness, list.center());
    let row = harness
        .query_all_by(|node| !matches!(node.role(), Role::RadioButton | Role::ComboBox) && node.label().as_deref() == Some("Parameter"))
        .map(|node| node.rect())
        .find(|rect| rect.top() >= list.bottom() - 1.0)
        .unwrap();
    click_at(&mut harness, row.center());

    assert_eq!(color_match_of(&harness, effect).source_mode, SourceColorMode::Parameter);

    let source = harness.get_by_role_and_label(Role::Button, "Source Color").rect();
    assert!(source.center().y < target_mode(&harness, "Image Input").center().y, "Source Color right under its mode");

    let luma = harness.get_by_role_and_label(Role::Slider, "Luma").rect();
    click_at(&mut harness, along(luma, 1.0));

    assert!(color_match_of(&harness, effect).luma > 0.95, "{}", color_match_of(&harness, effect).luma);

    let custom = harness.get_by_role_and_label(Role::CheckBox, "Custom Color Variation").rect();
    click_at(&mut harness, custom.center());

    assert!(!color_match_of(&harness, effect).custom_variation);
    assert!(harness.query_by_role_and_label(Role::Slider, "Hue").is_none(), "the Color Variation card goes with its switch");
}
