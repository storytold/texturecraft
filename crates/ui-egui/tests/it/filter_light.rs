//! The light and shadow filters in Properties: Glow's Used on a Mask, Mode and signed Width, Drop Shadow's Angle and Distance, the Baked Lighting cards and rows (the Sun's angle names)

use crate::filter_support::{TALL_WINDOW_HEIGHT, WINDOW_HEIGHT, along, app_with, click_at, drag, filter_of, topmost, undo, undo_depth};
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::core::{Filter, LightingOutput};

/// Clicks on Used on a Mask, Precise and the left end of Width turn the glow into a precise inner glow laid as white on the mask
#[test]
fn glow_rows_set_used_on_a_mask_precise_and_an_inner_width() {
    let (mut harness, effect) = app_with("glow", "mask", WINDOW_HEIGHT);

    let toggle = harness.get_by_role_and_label(Role::CheckBox, "Used on a Mask").rect();
    click_at(&mut harness, toggle.center());
    let precise = harness.get_by_role_and_label(Role::RadioButton, "Precise").rect();
    click_at(&mut harness, precise.center());
    let width = harness.get_by_role_and_label(Role::Slider, "Width").rect();
    click_at(&mut harness, along(width, 0.0));

    let Filter::HaloGlow { on_mask, precise, width, .. } = filter_of(&harness, effect) else { panic!("{:?}", filter_of(&harness, effect)) };
    assert!(on_mask && precise, "{on_mask} {precise}");
    assert!(width < -0.9, "inner glow: {width}");
}

/// Drop Shadow's Distance reaches both ways and its Angle row carries a dial
#[test]
fn drop_shadow_distance_goes_both_ways() {
    let (mut harness, effect) = app_with("dropShadow", "mask", WINDOW_HEIGHT);
    let distance = harness.get_by_role_and_label(Role::Slider, "Distance").rect();

    click_at(&mut harness, along(distance, 0.0));

    let Filter::DropShadow { distance, angle, .. } = filter_of(&harness, effect) else { panic!() };
    assert!(distance < -0.45, "towards the light: {distance}");
    assert_eq!(angle, 90.0);

    // The dial sits between the Angle track and its value, so the track ends short of Distance's
    let angle_track = harness.get_by_role_and_label(Role::Slider, "Angle").rect();
    let distance_track = harness.get_by_role_and_label(Role::Slider, "Distance").rect();
    assert!(angle_track.right() < distance_track.right() - DIAL_ROOM, "{angle_track:?} {distance_track:?}");
}

/// How much shorter than a plain slider's track an angle row's track is at least: room for its dial
const DIAL_ROOM: f32 = 10.0;

/// Drop Shadow's Angle dial row: a drag turns it in one undo step, undo turns it back
#[test]
fn drop_shadow_angle_drag_is_one_undo_step() {
    let (mut harness, effect) = app_with("dropShadow", "mask", WINDOW_HEIGHT);
    let depth = undo_depth(&harness);
    let angle = harness.get_by_role_and_label(Role::Slider, "Angle").rect();

    drag(&mut harness, along(angle, 0.5), along(angle, 0.0));

    let Filter::DropShadow { angle, .. } = filter_of(&harness, effect) else { panic!("{:?}", filter_of(&harness, effect)) };
    assert!(angle < 0.5, "turned to 0: {angle}");
    assert_eq!(undo_depth(&harness), depth + 1, "one undo step for the drag");

    undo(&mut harness);

    assert!(matches!(filter_of(&harness, effect), Filter::DropShadow { angle, .. } if angle == 90.0), "{:?}", filter_of(&harness, effect));
}

/// Baked Lighting Stylized's cards edit their own settings: Sun Intensity's right end, Light 1's Intensity and Output's Specular only
#[test]
fn baked_lighting_stylized_cards_set_the_sun_a_light_and_the_output() {
    let (mut harness, effect) = app_with("bakedLighting", "content", TALL_WINDOW_HEIGHT);

    let sun = harness.get_by_role_and_label(Role::Slider, "Sun Intensity").rect();
    click_at(&mut harness, along(sun, 1.0));

    // Light 1's card comes before Light 2's
    let light_1 = topmost(&harness, Role::Slider, "Intensity");
    click_at(&mut harness, along(light_1, 1.0));
    let output = harness.get_by_role_and_label(Role::ComboBox, "Diffuse + Specular").rect();
    click_at(&mut harness, output.center());
    let specular = harness.get_by_label("Specular only").rect();
    click_at(&mut harness, specular.center());

    let filter = filter_of(&harness, effect);
    let (stylized, light) = filter.stylized_lighting().unwrap();

    assert!(light.sun.intensity > 9.5, "{}", light.sun.intensity);
    assert!(stylized.lights[0].intensity > 1.9 && stylized.lights[1].intensity == 0.0, "{:?}", stylized.lights);
    assert_eq!(stylized.output, LightingOutput::Specular);
}

/// Baked Lighting Environment's rows: Horizontal Rotation with its dial, a signed Exposure, and the stacked Specular Occlusion Intensity under its label
#[test]
fn baked_lighting_environment_rows_set_rotation_exposure_and_occlusion() {
    let (mut harness, effect) = app_with("bakedLightingEnvironment", "content", TALL_WINDOW_HEIGHT);

    let rotation = harness.get_by_role_and_label(Role::Slider, "Horizontal Rotation").rect();
    click_at(&mut harness, rotation.center());
    let exposure = harness.get_by_role_and_label(Role::Slider, "Exposure").rect();
    click_at(&mut harness, along(exposure, 0.0));
    let specular = harness.get_by_role_and_label(Role::Slider, "Specular Occlusion Intensity").rect();
    click_at(&mut harness, along(specular, 1.0));

    let Filter::EnvironmentLighting { horizontal, exposure, specular_occlusion, ambient_occlusion, .. } = filter_of(&harness, effect) else {
        panic!("{:?}", filter_of(&harness, effect))
    };

    assert!((150.0..210.0).contains(&horizontal), "half way round: {horizontal}");
    assert!(exposure < -4.5, "darker: {exposure}");
    assert!(specular_occlusion > 0.95 && ambient_occlusion == 1.0, "{specular_occlusion} {ambient_occlusion}");
}
