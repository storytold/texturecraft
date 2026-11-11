//! The Assets panel's layout: columns at the reference width, type row, size menu, breadcrumb, side panes and toolbar

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::core::ProceduralKind;
use texturecraft_engine::core::project::ENVIRONMENTS;
use texturecraft_engine::materials::{MATERIALS, SmartMaterial};
use texturecraft_engine::smart_masks::SMART_MASKS;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::shelf_view::ShelfView;

/// The reference's capture window: its Assets panel shows four columns there
const REFERENCE_WINDOW: egui::Vec2 = egui::vec2(1920.0, 1032.0);
/// A window whose default Assets panel is too narrow for the full type row
const NARROW_WINDOW: egui::Vec2 = egui::vec2(1100.0, 1032.0);
/// The types' positions in the shelf's type row (its table is crate-private)
const MATERIALS_TYPE: usize = 0;
const SMART_MASKS_TYPE: usize = 2;
const BRUSHES_TYPE: usize = 4;
const TEXTURES_TYPE: usize = 6;
const ENVIRONMENTS_TYPE: usize = 7;
/// How far the saved brush's spacing differs from the current brush's, so loading it shows
const SAVED_SPACING_CHANGE: f32 = 0.25;
/// Tile columns the reference shows at its panel width
const REFERENCE_COLUMNS: usize = 4;

/// The sample project with the shelf on the Materials type
fn shelf(size: egui::Vec2) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder().with_size(size).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.shelf_tab = MATERIALS_TYPE;
    harness.run_steps(4);
    harness
}

/// The plain materials in the grid's order (by name)
fn plain_materials() -> Vec<&'static SmartMaterial> {
    let mut plain: Vec<_> = MATERIALS.iter().filter(|material| !material.smart).collect();
    plain.sort_by_key(|material| material.label.to_lowercase());
    plain
}

/// A clickable node labelled `label` (its tooltip, once shown, has the same label)
fn button<'a>(harness: &'a Harness<'_, TextureCraftApp>, label: &'a str) -> egui_kittest::Node<'a> {
    harness.get_all_by_label(label).next().unwrap_or_else(|| panic!("no {label}"))
}

#[test]
fn the_default_panel_shows_the_reference_column_count() {
    let harness = shelf(REFERENCE_WINDOW);
    let tops: Vec<f32> = plain_materials().iter().take(REFERENCE_COLUMNS + 1).map(|material| harness.get_by_label(material.label).rect().top()).collect();

    assert!(tops[..REFERENCE_COLUMNS].iter().all(|top| *top == tops[0]), "the first {REFERENCE_COLUMNS} tiles share a row: {tops:?}");
    assert!(tops[REFERENCE_COLUMNS] > tops[0], "the next tile starts a row: {tops:?}");
}

#[test]
fn a_click_on_the_picked_type_shows_every_type() {
    let mut harness = shelf(REFERENCE_WINDOW);
    let mask = SMART_MASKS[0].label;
    assert!(harness.query_by_label(mask).is_none(), "the Materials type hides smart masks");

    button(&harness, "Materials").click();
    harness.run_steps(3);

    assert!(harness.query_by_label(mask).is_some(), "no type picked shows smart masks too");
}

#[test]
fn the_search_narrows_the_picked_type_and_greys_types_without_matches() {
    let mut harness = shelf(REFERENCE_WINDOW);
    let material = plain_materials().into_iter().find(|material| material.label.to_lowercase().contains("rust")).unwrap();
    let mask = SMART_MASKS.iter().find(|mask| mask.label.to_lowercase().contains("rust")).unwrap();
    harness.state_mut().ui.shelf_filter = "rust".into();
    harness.state_mut().ui.shelf_tab = SMART_MASKS_TYPE;
    harness.run_steps(3);

    assert!(harness.query_by_label(mask.label).is_some(), "the matching mask shows");
    assert!(harness.query_by_label(material.label).is_none(), "a matching material of another type does not");
    assert!(button(&harness, "Environments").accesskit_node().is_disabled(), "no environment matches");
    assert!(!button(&harness, "Materials").accesskit_node().is_disabled());

    button(&harness, "Materials").click();
    harness.run_steps(3);
    assert_eq!(harness.state().ui.shelf_filter, "rust", "picking a type keeps the search");
    assert!(harness.query_by_label(material.label).is_some());
}

#[test]
fn the_breadcrumb_arrow_picks_a_library() {
    let mut harness = shelf(REFERENCE_WINDOW);
    let material = plain_materials()[0].label;

    button(&harness, "Browse folders").click();
    harness.run_steps(2);
    for library in ["Project", "Starter assets", "Your assets"] {
        assert!(harness.query_by_label(library).is_some(), "the list offers {library}");
    }
    harness.get_by_label("Project").click();
    harness.run_steps(3);

    assert_eq!(harness.state().ui.shelf_path, ["project"]);
    assert!(harness.query_by_label(material).is_none(), "built-in materials are not in the project");
}

#[test]
fn the_thumbnail_size_menu_switches_to_the_list() {
    let mut harness = shelf(REFERENCE_WINDOW);
    let material = plain_materials()[0].label;
    let tile_width = harness.get_by_label(material).rect().width();

    button(&harness, "Thumbnail size").click();
    harness.run_steps(2);
    harness.get_by_label_contains("List").click();
    harness.run_steps(3);

    assert_eq!(harness.state().ui.shelf_view, ShelfView::List);
    assert!(harness.get_by_label(material).rect().width() > tile_width * 2.0, "a list row spans the panel");
}

#[test]
fn reset_query_shows_every_asset_again() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().ui.shelf_filter = "rust".into();
    harness.state_mut().ui.shelf_path = vec!["starter".into()];
    harness.run_steps(2);

    button(&harness, "Reset query").click();
    harness.run_steps(3);

    let ui = &harness.state().ui;
    assert!(ui.shelf_filter.is_empty() && ui.shelf_path.is_empty());
    assert!(harness.query_by_label(SMART_MASKS[0].label).is_some(), "every type shows");
}

#[test]
fn refresh_reads_the_saved_brushes_again() {
    let mut harness = shelf(REFERENCE_WINDOW);
    let stale = vec![std::path::PathBuf::from("gone.tcbrush")];
    harness.state_mut().brushes = Some(stale.clone());
    harness.run_steps(1);

    button(&harness, "Refresh").click();
    harness.run_steps(3);

    assert_ne!(harness.state().brushes, Some(stale), "the brushes folder was read again");
    assert!(harness.query_by_label("Import resources...").is_some(), "the toolbar has the import button");
}

#[test]
fn a_saved_search_restores_its_query() {
    let mut harness = shelf(REFERENCE_WINDOW);
    button(&harness, "Saved searches").click();
    harness.state_mut().ui.shelf_filter = "rust".into();
    harness.run_steps(3);

    button(&harness, "Save the current search").click();
    harness.run_steps(2);
    assert_eq!(harness.state().ui.shelf_saved_searches.len(), 1);
    harness.state_mut().ui.shelf_filter.clear();
    harness.run_steps(2);

    harness.get_by_label("rust").click();
    harness.run_steps(2);
    assert_eq!(harness.state().ui.shelf_filter, "rust");

    harness.get_by_label("rust").click_secondary();
    harness.run_steps(2);
    // The context menu opens last, so its Delete is the newest node with that label
    harness.get_all_by_label("Delete").last().unwrap().click();
    harness.run_steps(2);
    assert!(harness.state().ui.shelf_saved_searches.is_empty());
}

#[test]
fn filter_by_path_picks_a_folder() {
    let mut harness = shelf(REFERENCE_WINDOW);
    button(&harness, "Filter by path").click();
    harness.run_steps(3);

    harness.get_by_label("Starter assets").click();
    harness.run_steps(2);

    assert_eq!(harness.state().ui.shelf_path, ["starter"]);
}

#[test]
fn a_narrow_panel_keeps_the_toolbar_and_folds_the_type_row() {
    let mut harness = shelf(NARROW_WINDOW);
    harness.state_mut().ui.shelf_panes.saved_searches = true;
    harness.state_mut().ui.shelf_panes.filter_by_path = true;
    harness.run_steps(3);

    for label in ["Refresh", "Reset query", "Import resources...", "Saved searches", "Filter by path"] {
        let rect = button(&harness, label).rect();
        assert!(rect.bottom() <= NARROW_WINDOW.y, "{label} is inside the window: {rect:?}");
    }
    assert!(harness.query_by_label("Filter by type").is_some(), "the type row folds into one dropdown");
    assert!(harness.query_by_label("Environments").is_none());
}

#[test]
fn an_environment_tile_switches_the_lighting() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().ui.shelf_tab = ENVIRONMENTS_TYPE;
    harness.run_steps(3);
    let shown = harness.state().session.doc.project.display.environment.clone();
    let other = ENVIRONMENTS.iter().find(|name| **name != shown).unwrap();

    harness.get_by_label(other).click();
    harness.run_steps(2);

    assert_eq!(harness.state().session.doc.project.display.environment, *other);
}

#[test]
fn a_saved_brush_is_a_tile_that_loads_the_preset() {
    let mut harness = shelf(REFERENCE_WINDOW);
    let mut saved = harness.state().session.tool.brush.clone();
    saved.spacing += SAVED_SPACING_CHANGE;
    let path = std::env::temp_dir().join(format!("shelf-panel-{}.tcbrush", std::process::id()));
    std::fs::write(&path, serde_json::to_vec(&saved).unwrap()).unwrap();
    harness.state_mut().brushes = Some(vec![path.clone()]);
    harness.state_mut().ui.shelf_tab = BRUSHES_TYPE;
    harness.run_steps(3);

    harness.get_by_label(path.file_stem().unwrap().to_str().unwrap()).click();
    harness.run_steps(2);
    std::fs::remove_file(&path).unwrap();

    assert_eq!(harness.state().session.tool.brush.spacing, saved.spacing, "the click loads the saved stroke");
}

#[test]
fn saved_smart_materials_keep_their_type_pickable_in_your_assets() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().library = Some(vec![std::path::PathBuf::from("Mine.tcmat")]);
    harness.state_mut().ui.shelf_path = vec!["user".into()];
    harness.state_mut().ui.shelf_tab = BRUSHES_TYPE;
    harness.run_steps(3);

    assert!(!button(&harness, "Smart materials").accesskit_node().is_disabled(), "a saved smart material is in Your assets");
    assert!(button(&harness, "Materials").accesskit_node().is_disabled(), "no plain material is");
}

#[test]
fn saved_smart_materials_are_read_whatever_the_type_and_after_a_refresh() {
    let mut harness = shelf(REFERENCE_WINDOW);
    assert!(harness.state().library.is_some(), "read on the Materials type");

    button(&harness, "Refresh").click();
    harness.run_steps(2);

    assert!(harness.state().library.is_some(), "read again after a refresh");
}

#[test]
fn procedurals_are_in_the_textures_type() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().ui.shelf_tab = TEXTURES_TYPE;
    harness.run_steps(3);

    assert!(harness.query_by_label(ProceduralKind::ALL[0].label()).is_some());
}
