//! Filter image inputs and Attributes in Properties: Bevel Smooth's Distance Map card after the filter's, Warp's Custom Noise slot under its Source Mode, the read-only Attributes row ending the filter card

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::core::{LayerId, ProceduralKind, Source};
use texturecraft_ui_egui::TextureCraftApp;

use crate::filter_support::{WINDOW_HEIGHT, along, app_with, click_at, lowest, pick, topmost};

/// The source effect `effect` holds in its input `id`
fn input(harness: &Harness<'_, TextureCraftApp>, effect: LayerId, id: &str) -> Option<Source> {
    harness.state().session.doc.project.effect(effect).unwrap().1.inputs.get(id).cloned()
}

/// A click on the control of `role` named `label` (the lowest, where a list above shows the same name)
fn click(harness: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    let at = lowest(harness, role, label).center();
    click_at(harness, at);
}

/// The Distance Map slot picks a pattern from the source picker, × puts it back to white, its ramp sets a gray
#[test]
fn the_distance_map_slot_picks_its_source() {
    let (mut harness, effect) = app_with("bevelSmooth", "mask", WINDOW_HEIGHT);
    assert!(harness.query_by_label("Image inputs").is_none(), "an input every mode reads is a card of its own, no band");
    assert!(lowest(&harness, Role::ComboBox, "Distance Map").top() > topmost(&harness, Role::Button, "Attributes").bottom(), "after the filter card");

    click(&mut harness, Role::ComboBox, "Distance Map");

    assert!(harness.query_by_role_and_label(Role::Tab, "Anchor points").is_some(), "the slot opens the source picker");

    click(&mut harness, Role::ListBoxOption, "Clouds");

    let Some(Source::Procedural { pattern, low, high, .. }) = input(&harness, effect, "distanceMap") else { panic!("a procedural") };
    assert_eq!((pattern.kind, low, high), (ProceduralKind::Clouds, [0.0; 3], [1.0; 3]));
    assert!(!egui::Popup::is_any_open(&harness.ctx), "a pick closes the picker");

    // The filter card has its own ×: the slot's is the lower one
    click(&mut harness, Role::Button, "Clear");

    assert_eq!(input(&harness, effect, "distanceMap"), None, "back to its default white");

    let ramp = harness.get_by_role_and_label(Role::Slider, "Distance Map").rect();
    click_at(&mut harness, along(ramp, 0.0));

    let Some(Source::Uniform([gray, ..])) = input(&harness, effect, "distanceMap") else { panic!("a gray") };
    assert!(gray < 0.05, "{gray}");
}

/// Warp shows its Custom Noise slot right under Source Mode only in that mode, above the source's sliders; a pattern picked there shows its Scale and Contrast
#[test]
fn the_custom_noise_slot_follows_the_source_mode() {
    let (mut harness, effect) = app_with("warp", "mask", WINDOW_HEIGHT);
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Custom Noise").is_none());

    pick(&mut harness, "Default Noise", "Custom Noise");

    let ramp = harness.get_by_role_and_label(Role::Slider, "Custom Noise").rect();
    click_at(&mut harness, along(ramp, 0.25));

    let Some(Source::Uniform([gray, ..])) = input(&harness, effect, "customNoise") else { panic!("expected a uniform input") };
    assert!((gray - 0.25).abs() < 0.05);

    let slot = lowest(&harness, Role::ComboBox, "Custom Noise");
    let blur = harness.get_by_role_and_label(Role::Slider, "Source Blur").rect();
    assert!(slot.bottom() <= blur.top(), "the slot sits above Source Blur");

    click_at(&mut harness, slot.center());
    click(&mut harness, Role::ListBoxOption, "Clouds");

    let Some(Source::Procedural { pattern, .. }) = input(&harness, effect, "customNoise") else { panic!("a procedural custom noise") };
    assert!(harness.query_by_label("Image inputs").is_none(), "a mode's input is not under Image inputs");

    // The picked pattern's Scale and Contrast show inside the mode's card, like every slot's, and set the pattern
    let scale = harness.get_by_role_and_label(Role::Slider, "Scale").rect();
    assert!(scale.top() >= slot.bottom() && harness.query_by_role_and_label(Role::Slider, "Contrast").is_some());

    click_at(&mut harness, along(scale, 0.0));

    let Some(Source::Procedural { pattern: scaled, .. }) = input(&harness, effect, "customNoise") else { panic!("still a procedural") };
    assert!(scaled.scale < pattern.scale, "{} -> {}", pattern.scale, scaled.scale);
}

/// Attributes is closed at first; a click on the filter card's last row shows the filter's read-only rows in the card, above the input cards; another hides them
#[test]
fn the_attributes_row_opens_on_a_click() {
    let (mut harness, _) = app_with("bevelSmooth", "mask", WINDOW_HEIGHT);
    let category = "Category: Filter";
    assert!(harness.query_by_label(category).is_none(), "closed by default");

    let band = topmost(&harness, Role::Button, "Attributes");
    assert!(band.bottom() < lowest(&harness, Role::ComboBox, "Distance Map").top(), "above the input's card");

    click_at(&mut harness, band.center());

    assert!(harness.query_by_label(category).is_some());
    assert!(harness.query_by_label("Description: Draws a smooth ramp from the edge of the shapes.").is_some());
    assert!(harness.query_by_label("Tags: bevel, edge, ramp").is_some());
    assert!(harness.query_by_label("Size: 256 × 256").is_some(), "the texture set's size");
    assert!(harness.get_by_label(category).rect().top() > band.bottom(), "the rows hang under the row");
    assert!(harness.get_by_label(category).rect().bottom() < lowest(&harness, Role::ComboBox, "Distance Map").top(), "inside the filter card");

    click_at(&mut harness, band.center());

    assert!(harness.query_by_label(category).is_none());
}

/// An open section that makes the panel scroll keeps its rows in place from frame to frame (the scroll bar refits no row)
#[test]
fn the_sheet_keeps_its_layout_when_the_scroll_bar_comes_in() {
    let (mut harness, _) = app_with("bevelSmooth", "mask", WINDOW_HEIGHT);
    let attributes = topmost(&harness, Role::Button, "Attributes").center();
    click_at(&mut harness, attributes);
    let layout = |harness: &Harness<'_, TextureCraftApp>| -> Vec<egui::Rect> {
        harness.query_all_by_role(Role::ComboBox).chain(harness.query_all_by_role(Role::Button)).map(|node| node.rect()).collect()
    };
    let first = layout(&harness);

    for frame in 0..3 {
        harness.run_steps(1);

        assert_eq!(layout(&harness), first, "frame {frame}");
    }
}
