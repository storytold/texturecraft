//! The Transform filter in Properties: Scaling and Offset beside its preview, Tile Safe Rotation, Rotation, Repeat mode and Flip mode

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use texturecraft_engine::core::{Filter, FlipMode, RepeatMode, TileTransform};
use texturecraft_ui_egui::TextureCraftApp;

use crate::filter_support::{WINDOW_HEIGHT, along, app_with, click_at, drag, filter_of, undo, undo_depth};

/// Every row sets its own setting through the pointer, in the reference's order
#[test]
fn transform_rows_set_the_transform() {
    let (mut harness, effect) = app_with("transform", "mask", WINDOW_HEIGHT);
    let transform = |harness: &Harness<'_, TextureCraftApp>| match filter_of(harness, effect) {
        Filter::Transform(transform) => transform,
        other => panic!("not a transform: {other:?}"),
    };
    assert_eq!(transform(&harness), TileTransform::LIBRARY);

    let top = |harness: &Harness<'_, TextureCraftApp>, role: Role, label: &str| harness.get_by_role_and_label(role, label).rect().top();
    let order = [
        top(&harness, Role::Slider, "Scaling"),
        top(&harness, Role::Slider, "Offset"),
        top(&harness, Role::CheckBox, "Tile Safe Rotation"),
        top(&harness, Role::Slider, "Rotation"),
        top(&harness, Role::ComboBox, "Horizontally & Vertically"),
        top(&harness, Role::ComboBox, "None"),
    ];
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "Scaling, Offset, Tile Safe Rotation, Rotation, Repeat mode, Flip mode: {order:?}");

    // Linked, the Scaling track sets both directions
    let scaling = harness.get_by_role_and_label(Role::Slider, "Scaling").rect();
    click_at(&mut harness, along(scaling, 1.0));

    let scaled = transform(&harness).scaling;
    assert!(scaled[0] > 4.5 && scaled[0] == scaled[1], "{scaled:?}");

    let offset = harness.get_by_role_and_label(Role::Slider, "Offset").rect();
    click_at(&mut harness, along(offset, 1.0));

    let moved = transform(&harness).offset;
    assert!(moved[0] > 0.95 && moved[1] == 0.0, "the first Offset slider moves across: {moved:?}");

    let tile_safe = harness.get_by_role_and_label(Role::CheckBox, "Tile Safe Rotation").rect();
    click_at(&mut harness, tile_safe.center());

    assert!(!transform(&harness).tile_safe);

    let rotation = harness.get_by_role_and_label(Role::Slider, "Rotation").rect();
    click_at(&mut harness, rotation.center());

    let degrees = transform(&harness).rotation;
    assert!((150.0..210.0).contains(&degrees), "{degrees}");

    let flip = harness.get_by_role_and_label(Role::ComboBox, "None").rect();
    click_at(&mut harness, flip.center());
    let vertically = harness.get_by_label("Vertically").rect();
    click_at(&mut harness, vertically.center());

    assert_eq!(transform(&harness).flip, FlipMode::Vertically);

    let repeat = harness.get_by_role_and_label(Role::ComboBox, "Horizontally & Vertically").rect();
    click_at(&mut harness, repeat.center());
    let none = harness.get_by_label("None").rect();
    click_at(&mut harness, none.center());

    assert_eq!(transform(&harness).repeat, RepeatMode::None);
}

/// A Rotation drag is one undo step, and undo turns the tile back
#[test]
fn rotation_drag_is_one_undo_step() {
    let (mut harness, effect) = app_with("transform", "mask", WINDOW_HEIGHT);
    let depth = undo_depth(&harness);
    let rotation = harness.get_by_role_and_label(Role::Slider, "Rotation").rect();

    drag(&mut harness, along(rotation, 0.0), along(rotation, 0.5));

    let Filter::Transform(turned) = filter_of(&harness, effect) else { panic!() };
    assert!((150.0..210.0).contains(&turned.rotation), "{turned:?}");
    assert_eq!(undo_depth(&harness), depth + 1, "one undo step for the drag");

    undo(&mut harness);

    assert_eq!(filter_of(&harness, effect), Filter::Transform(TileTransform::LIBRARY));
}
