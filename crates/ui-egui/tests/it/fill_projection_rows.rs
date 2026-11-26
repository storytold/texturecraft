//! A fill's Fill section shows the rows its projection has, and each new row changes the projection

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::{Fill, LayerId, Projection, ShapeCrop};
use texturecraft_ui_egui::TextureCraftApp;

use crate::fill_support::{click_label, fill_app, fill_of};
use crate::filter_support::click_at;
use crate::{PROPERTIES_WINDOW, SETTLE_FRAMES};

/// The app with a new fill layer (projected as `projection`) selected in Properties, and its layer id
fn app_with_fill(projection: &str) -> (Harness<'static, TextureCraftApp>, u64) {
    let (harness, LayerId(layer)) = fill_app(json!({}), Some(projection), None, PROPERTIES_WINDOW);
    (harness, layer)
}

/// The fill layer's projection as the document holds it
fn projection(harness: &Harness<'_, TextureCraftApp>, layer: u64) -> Projection {
    fill(harness, layer).projection
}

/// The fill layer as the document holds it
fn fill(harness: &Harness<'_, TextureCraftApp>, layer: u64) -> Fill {
    fill_of(harness, LayerId(layer))
}

#[test]
fn each_projection_shows_its_own_rows() {
    let shows = |harness: &Harness<'_, TextureCraftApp>, label: &str| harness.query_by_label(label).is_some();

    let (harness, _) = app_with_fill("planar");
    for label in ["Depth culling", "Backface culling", "Angle", "Extends Outside Shape", "Offset X", "Rotation Y", "Scale Z"] {
        assert!(shows(&harness, label), "planar: {label}");
    }

    assert!(!shows(&harness, "Reset frame"), "planar has no frame to reset");

    let (harness, _) = app_with_fill("warp");

    // A warp's 3D projection settings are its frame's: where its centre is, its turn and its size
    for label in ["Depth", "Reset frame", "Depth culling", "Offset X", "Rotation X", "Scale X"] {
        assert!(shows(&harness, label), "warp: {label}");
    }

    assert!(!shows(&harness, "Backface culling"), "a warp has no backface culling");

    let (harness, _) = app_with_fill("cylindrical");
    for label in ["Angle", "Backface culling", "Extends Outside Shape", "Offset X"] {
        assert!(shows(&harness, label), "cylindrical: {label}");
    }

    assert!(!shows(&harness, "Depth culling"));

    let (harness, _) = app_with_fill("triplanar");
    assert!(shows(&harness, "Hardness") && !shows(&harness, "UV Wrap") && !shows(&harness, "Backface culling"));

    let (mut harness, layer) = app_with_fill("uvTile");
    assert!(shows(&harness, "Fill (Match per UV Tile)"));
    assert!(!shows(&harness, "Rotation"), "per UV tile there is nothing to place");
    assert!(shows(&harness, "Filtering and edges"));

    let at = harness.get_by_role_and_label(egui::Role::ComboBox, "Bilinear | HQ").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(SETTLE_FRAMES);
    click_label(&mut harness, "Nearest");

    assert_eq!(fill(&harness, layer).sampling.filtering, texturecraft_engine::core::Filtering::Nearest);
}

#[test]
fn changing_one_physical_size_preserves_the_other_axes_tiling() {
    let (mut harness, layer) = app_with_fill("planar");
    let tiling = [1.23456, 2.34567];
    harness.state_mut().run("layer.fill.projection", json!({"layer": layer, "tiling": tiling}));
    harness.run_steps(SETTLE_FRAMES);
    click_label(&mut harness, "Physical size");

    let track = harness.get_by_role_and_label(egui::Role::Slider, "Size (cm)").rect();
    click_at(&mut harness, track.center());

    let Projection::Planar { uv, .. } = projection(&harness, layer) else { panic!("still planar") };

    assert_ne!(uv.tiling[0], tiling[0]);
    assert_eq!(uv.tiling[1], tiling[1]);
}

#[test]
fn culling_toggles_change_the_projection() {
    let (mut harness, layer) = app_with_fill("planar");
    click_label(&mut harness, "Depth culling");
    click_label(&mut harness, "Backface culling");
    let Projection::Planar { projector, .. } = projection(&harness, layer) else { panic!("still planar") };

    assert!(!projector.depth.on, "a new planar starts depth culled; the toggle turns it off");
    assert!(projector.backface.on);
}

/// What the Depth culling row's i says on hover
const DEPTH_CULLING_TIP: &str = "Paints only the surface the projector sees first; parts hidden behind it stay unpainted.";

/// What the Backface culling row's i says on hover
const BACKFACE_CULLING_TIP: &str = "Leaves out faces turned away from the projector, so the fill doesn't run onto their back.";

/// A point away from the Properties panel, where the pointer leaves no tooltip
const AWAY_FROM_PANEL: egui::Pos2 = egui::pos2(800.0, 500.0);

/// Frames for the pointer to rest long enough for a tooltip
const TOOLTIP_FRAMES: usize = 60;

/// Room the i's tooltip leaves at the panel's right edge for the values
const TIP_VALUE_ROOM: f32 = 70.0;

#[test]
fn the_culling_rows_i_explains_the_row_on_hover_without_covering_its_values() {
    let (mut harness, layer) = app_with_fill("planar");

    // The i ends on the panel's right edge, where the values end
    let panel_right = harness.get_by_label(DEPTH_CULLING_TIP).rect().right();
    for (tip, row) in [(DEPTH_CULLING_TIP, "Depth culling"), (BACKFACE_CULLING_TIP, "Backface culling")] {
        let info = harness.get_by_label(tip).rect();
        let switch = harness.get_by_label(row).rect();

        assert!(info.left() >= switch.right() && (info.center().y - switch.center().y).abs() < 1.0, "{row}: the i sits at its row's end");
        assert_eq!(info.right(), panel_right, "{row}: on the panel's right edge");
        assert_eq!(harness.query_all_by_label(tip).count(), 1, "{row}: no tooltip before the hover");

        harness.hover_at(info.center());
        harness.run_steps(TOOLTIP_FRAMES);
        let shown = harness.query_all_by_label(tip).map(|node| node.rect()).find(|rect| *rect != info);
        let Some(shown) = shown else { panic!("{row}: hovering the i shows its tooltip") };
        assert!(shown.top() >= info.bottom() - 2.0, "{row}: the tooltip hangs under the row");
        assert!(shown.right() <= panel_right - TIP_VALUE_ROOM, "{row}: the tooltip ends before the value column");

        harness.hover_at(AWAY_FROM_PANEL);
        harness.run_steps(SETTLE_FRAMES);
    }

    // The i only explains: a click on it changes nothing
    let before = projection(&harness, layer);
    let info = harness.get_by_label(DEPTH_CULLING_TIP).rect().center();
    click_at(&mut harness, info);

    assert_eq!(projection(&harness, layer), before);
}

/// What a warp's Depth row says on its i
const WARP_DEPTH_TIP: &str = "How far the warp projection reaches into the mesh, in front of and behind its frame, in lengths of the frame's longer side. It bounds the fill only when cropped to its shape.";

#[test]
fn shape_crop_sits_beside_its_label_and_a_warps_depth_explains_itself() {
    let (harness, _) = app_with_fill("warp");

    // Beside its label like Filtering, not under a stacked label at the panel's full width
    let crop = harness.get_by_label("Extends Outside Shape").rect();
    let filtering = harness.get_by_label("Bilinear | HQ").rect();

    assert!((crop.left() - filtering.left()).abs() < 0.5, "Shape crop's list starts on the controls column: {crop:?} vs {filtering:?}");
    assert!((crop.width() - filtering.width()).abs() < 0.5, "and is as wide as Filtering's");

    // Depth, not "Projection Depth": its i, on its row, tells it apart from Depth culling
    let info = harness.get_by_label(WARP_DEPTH_TIP).rect();
    let depth = harness.get_by_label("Depth").rect();
    assert!((info.center().y - depth.center().y).abs() < 1.0 && info.right() <= depth.left() + 1.0, "the i sits left of Depth");
    assert!(harness.query_by_label("Projection Depth").is_none());
}

#[test]
fn shape_crop_is_picked_from_its_list() {
    // Tri-planar's Shape crop sits highest in the panel, in view
    let (mut harness, layer) = app_with_fill("triplanar");
    click_label(&mut harness, "Extends Outside Shape");
    click_label(&mut harness, "Cropped to Shape");
    let Projection::Triplanar { projector, .. } = projection(&harness, layer) else { panic!("still tri-planar") };

    assert_eq!(projector.crop, ShapeCrop::Cropped);
}

#[test]
fn triplanar_hardness_runs_from_soft_to_the_hardest_blend() {
    let (mut harness, layer) = app_with_fill("triplanar");
    let track = harness.get_by_label("Hardness").rect();
    click_at(&mut harness, egui::pos2(track.right() - 1.0, track.center().y));
    let Projection::Triplanar { sharpness, .. } = projection(&harness, layer) else { panic!("still tri-planar") };

    assert!(sharpness > 48.0, "Hardness 1 is the hardest blend, 64: {sharpness}");

    click_at(&mut harness, egui::pos2(track.left() + 1.0, track.center().y));
    let Projection::Triplanar { sharpness, .. } = projection(&harness, layer) else { panic!("still tri-planar") };
    assert!(sharpness < 1.5, "Hardness 0 is the softest, 1: {sharpness}");
}

#[test]
fn reset_frame_puts_a_warps_corners_back() {
    let (mut harness, layer) = app_with_fill("warp");
    let pulled = json!({"layer": layer, "warpCorners": [[-0.7, 0.6], [0.5, 0.5], [0.5, -0.5], [-0.5, -0.5]]});
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.fill.projection", &pulled).unwrap();
    harness.run_steps(SETTLE_FRAMES);

    click_label(&mut harness, "Reset frame");
    let Projection::Warp { corners, .. } = projection(&harness, layer) else { panic!("still a warp") };

    assert_eq!(corners, Projection::FRAME_CORNERS);
}

#[test]
fn the_3d_projection_settings_fold_away() {
    let (mut harness, _) = app_with_fill("spherical");

    assert!(harness.query_by_label("Scale X").is_some(), "the band starts open");

    click_label(&mut harness, "3D projection settings");

    assert!(harness.query_by_label("Scale X").is_none());
    assert!(harness.query_by_label("Spherical projection").is_some(), "the rows above it stay");
}

#[test]
fn the_fill_caption_resets_the_whole_fill_section_to_its_projections_defaults_in_one_undo_step() {
    use texturecraft_engine::core::{Filtering, Sampling};
    let (mut harness, layer) = app_with_fill("planar");
    harness.state_mut().run("layer.fill.projection", json!({"layer": layer, "tiling": 3, "filtering": "nearest", "shapeCrop": "cropped", "symmetry": true}));
    harness.run_steps(SETTLE_FRAMES);
    let changed = fill(&harness, layer);
    let reset = harness.get_by_label("Fill: Reset to defaults").rect().center();
    let edges = harness.get_by_label("Restore defaults").rect().center();

    // The band's reset puts back only Filtering and edges, through a real pointer click
    click_at(&mut harness, edges);
    let after_band = fill(&harness, layer);

    assert_eq!((after_band.sampling.filtering, crop(&after_band.projection)), (Filtering::default(), Some(ShapeCrop::default())));
    assert!(matches!(after_band.projection, Projection::Planar { .. }), "the projection stays planar");
    assert!(harness.get_by_label("Restore defaults").accesskit_node().is_disabled(), "at its defaults the band's reset is inert");

    // The caption's reset puts back the whole section
    click_at(&mut harness, reset);
    let after = fill(&harness, layer);
    assert!(
        matches!(after.projection, Projection::Planar { scale, uv, placed: None, .. } if scale == 1.0 && uv == Default::default()),
        "still planar, at a new planar's defaults: {:?}",
        after.projection
    );
    assert_eq!(after.sampling, Sampling { symmetry: changed.sampling.symmetry, ..Sampling::default() }, "the symmetry stays");
    assert!(harness.get_by_label("Fill: Reset to defaults").accesskit_node().is_disabled(), "at the defaults it is faint and inert");

    click_at(&mut harness, reset);

    assert_eq!(fill(&harness, layer), after, "a click at the defaults changes nothing");

    harness.state_mut().run("edit.undo", json!({}));

    assert_eq!(fill(&harness, layer), after_band, "one undo step brings the whole section back");
}

/// A click from one typed number field to the next types into the one clicked: the field after it keeps its own value
#[test]
fn a_click_from_one_typed_field_to_the_next_types_into_the_one_clicked() {
    let (mut harness, _) = app_with_fill("planar");
    let value = |harness: &Harness<'_, TextureCraftApp>, label: &str| harness.get_by_label(label).accesskit_node().numeric_value().unwrap();
    let untouched = value(&harness, "Offset Z");

    click_label(&mut harness, "Offset X");
    harness.event(egui::Event::Text("0.1".into()));
    harness.run_steps(SETTLE_FRAMES);
    click_label(&mut harness, "Offset Y");
    harness.event(egui::Event::Text("0.2".into()));
    harness.run_steps(SETTLE_FRAMES);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(SETTLE_FRAMES);

    assert!((value(&harness, "Offset X") - 0.1).abs() < 1e-4, "the first field took its value");
    assert!((value(&harness, "Offset Y") - 0.2).abs() < 1e-4, "the clicked field took the typing");
    assert_eq!(value(&harness, "Offset Z"), untouched, "the field after it kept its value");
}

/// A projection's Shape crop, if it has a projector
fn crop(projection: &Projection) -> Option<ShapeCrop> {
    match projection {
        Projection::Planar { projector, .. } | Projection::Triplanar { projector, .. } => Some(projector.crop),
        _ => None,
    }
}
