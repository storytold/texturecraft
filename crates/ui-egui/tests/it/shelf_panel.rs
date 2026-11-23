//! The Assets panel's layout: columns at the reference width, type row, size menu, breadcrumb, side panes and toolbar

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::brush_preset_cmds::UserBrush;
use texturecraft_engine::core::ProceduralKind;
use texturecraft_engine::core::project::ENVIRONMENTS;
use texturecraft_engine::materials::{MATERIALS, SmartMaterial};
use texturecraft_engine::smart_masks::SMART_MASKS;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::saved_search::SavedSearch;
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
fn unavailable_project_images_do_not_make_textures_match() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().session.doc.resources.insert("unavailable-preview.png".into(), vec![0].into());
    harness.state_mut().ui.shelf_filter = "unavailable-preview.png".into();
    harness.run_steps(3);

    assert!(button(&harness, "Textures").accesskit_node().is_disabled());
    assert!(harness.query_by_label("unavailable-preview.png").is_none());
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
    let stale = UserBrush { name: "Stale brush marker".into(), brush: Default::default(), channels: None, file: None };
    harness.state_mut().session.brush_presets.list.push(stale);
    harness.run_steps(1);

    button(&harness, "Refresh").click();
    harness.run_steps(3);

    assert!(harness.state().session.brush_presets.find("Stale brush marker").is_none(), "the brushes folder was read again");
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
fn many_saved_searches_keep_the_path_filter_reachable() {
    let mut harness = shelf(NARROW_WINDOW);
    harness.state_mut().ui.shelf_panes.saved_searches = true;
    harness.state_mut().ui.shelf_panes.filter_by_path = true;
    harness.state_mut().ui.shelf_saved_searches = (0..100)
        .map(|index| SavedSearch { name: format!("search {index}"), filter: format!("filter {index}"), tab: MATERIALS_TYPE, path: Vec::new() })
        .collect();
    harness.run_steps(3);

    let heading = harness.get_by_label("FILTER BY PATH").rect();
    assert!(heading.bottom() < NARROW_WINDOW.y, "the path pane heading stays on screen: {heading:?}");
    assert!(harness.query_by_label("Starter assets").is_some(), "the folder tree remains reachable");
}

#[test]
fn a_long_saved_search_list_can_select_the_last_query() {
    let mut harness = shelf(NARROW_WINDOW);
    harness.state_mut().ui.shelf_panes.saved_searches = true;
    harness.state_mut().ui.shelf_saved_searches = (0..1000)
        .map(|index| SavedSearch { name: format!("search {index}"), filter: format!("filter {index}"), tab: MATERIALS_TYPE, path: Vec::new() })
        .collect();
    harness.run_steps(3);
    assert!(harness.query_by_label("search 500").is_none(), "offscreen searches do not build rows");

    harness.get_by_label("search 0").hover();
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -50_000.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(3);
    harness.get_by_label("search 999").click();
    harness.run_steps(2);

    assert_eq!(harness.state().ui.shelf_filter, "filter 999");
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
    harness.state_mut().session.brush_presets.list.push(UserBrush { name: "Session brush".into(), brush: saved.clone(), channels: None, file: None });
    harness.state_mut().ui.shelf_tab = BRUSHES_TYPE;
    harness.run_steps(3);

    harness.get_by_label("Session brush").click();
    harness.run_steps(2);

    assert_eq!(harness.state().session.tool.brush.spacing, saved.spacing, "the click applies the in-memory saved stroke");
}

#[test]
fn brush_management_follows_the_shelf_type_folder_and_search() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().session.brush_presets.list.push(UserBrush { name: "Session brush".into(), brush: Default::default(), channels: None, file: None });
    harness.state_mut().ui.shelf_tab = BRUSHES_TYPE;
    harness.run_steps(3);

    assert!(harness.query_by_label("Save current brush").is_some());
    assert!(harness.query_by_label("My brushes").is_some());
    let save_rect = harness.get_by_label("Save current brush").rect();
    assert!(save_rect.right() <= button(&harness, "Thumbnail size").rect().right(), "the controls stay inside the Assets panel: {save_rect:?}");
    if let Some(dir) = std::env::var_os("TC_UI_SNAPSHOTS") {
        std::fs::create_dir_all(&dir).unwrap();
        harness.get_by_label("Save current brush").scroll_to_me();
        harness.run_steps(2);
        harness.render().unwrap().save(std::path::PathBuf::from(dir).join("brush-management-shelf.png")).unwrap();
    }

    harness.state_mut().ui.shelf_filter = "no matching brush".into();
    harness.run_steps(2);
    assert!(harness.query_by_label("Save current brush").is_none());

    harness.state_mut().ui.shelf_filter.clear();
    harness.state_mut().ui.shelf_path = vec!["starter".into(), "Brushes".into()];
    harness.run_steps(2);
    assert!(harness.query_by_label("Save current brush").is_none());

    harness.state_mut().ui.shelf_path = vec!["user".into(), "Brushes".into()];
    harness.run_steps(2);
    assert!(harness.query_by_label("Save current brush").is_some());

    harness.state_mut().ui.shelf_tab = MATERIALS_TYPE;
    harness.run_steps(2);
    assert!(harness.query_by_label("Save current brush").is_none());
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

/// The reference's type dropdown, top to bottom
const TYPE_DROPDOWN: [&str; 10] =
    ["Materials", "Smart materials", "Smart masks", "Filters", "Brushes", "Alphas", "Textures", "Environments", "Fonts", "Show all"];

/// The node labelled `label` added last (a popup's entry is newer than any other with its label)
fn latest_node_with_label<'a>(harness: &'a Harness<'_, TextureCraftApp>, label: &'a str) -> egui_kittest::Node<'a> {
    harness.get_all_by_label(label).last().unwrap_or_else(|| panic!("no {label}"))
}

/// Hover `label` long enough for its tooltip and tell whether the "Not available yet" hint shows
fn shows_not_available(harness: &mut Harness<'_, TextureCraftApp>, label: &str) -> bool {
    let center = latest_node_with_label(harness, label).rect().center();
    harness.hover_at(center);
    harness.run_steps(30);

    harness.query_by_label("Not available yet").is_some()
}

#[test]
fn the_type_dropdown_lists_the_reference_entries_in_order() {
    let mut harness = shelf(NARROW_WINDOW);
    button(&harness, "Filter by type").click();
    harness.run_steps(3);

    let tops: Vec<f32> = TYPE_DROPDOWN.iter().map(|label| latest_node_with_label(&harness, label).rect().top()).collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "the entries run top to bottom in the reference order: {tops:?}");
    for label in TYPE_DROPDOWN {
        let disabled = latest_node_with_label(&harness, label).accesskit_node().is_disabled();
        assert_eq!(disabled, label == "Fonts", "only Fonts is greyed: {label}");
    }
    assert!(shows_not_available(&mut harness, "Fonts"), "Fonts explains why it is greyed");
}

#[test]
fn show_all_unpicks_the_type() {
    let mut harness = shelf(NARROW_WINDOW);
    button(&harness, "Filter by type").click();
    harness.run_steps(3);

    latest_node_with_label(&harness, "Show all").click();
    harness.run_steps(3);

    assert!(harness.query_by_label(SMART_MASKS[0].label).is_some(), "Show all restores assets from another type");
}

#[test]
fn the_panes_list_the_reference_rows_and_grey_the_missing_ones() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().ui.shelf_panes.saved_searches = true;
    harness.state_mut().ui.shelf_panes.filter_by_path = true;
    harness.run_steps(3);

    let saved = ["SAVED SEARCHES", "All", "Favorites", "3D Assets", "Flattened layers"];
    let paths = ["FILTER BY PATH", "All libraries", "Project", "Starter assets", "Your assets", "System fonts", "User fonts"];
    for rows in [&saved[..], &paths[..]] {
        let tops: Vec<f32> = rows.iter().map(|label| latest_node_with_label(&harness, label).rect().top()).collect();
        assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{rows:?} run top to bottom: {tops:?}");
    }
    for label in ["Favorites", "3D Assets", "Flattened layers", "System fonts", "User fonts"] {
        assert!(latest_node_with_label(&harness, label).accesskit_node().is_disabled(), "{label} is greyed");
    }
    for label in ["All", "All libraries", "Project", "Starter assets", "Your assets"] {
        assert!(!latest_node_with_label(&harness, label).accesskit_node().is_disabled(), "{label} works");
    }
    assert!(shows_not_available(&mut harness, "Flattened layers"), "a greyed saved search explains itself");
    assert!(shows_not_available(&mut harness, "User fonts"), "a greyed library explains itself");

    button(&harness, "Hide non-applicable folders").click();
    harness.run_steps(3);

    assert!(harness.query_by_label("System fonts").is_none());
    assert!(harness.query_by_label("User fonts").is_none());
    assert!(harness.query_by_label("Starter assets").is_some());
}

#[test]
fn the_splitter_resizes_both_panes_and_the_split_is_saved() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().ui.shelf_panes.saved_searches = true;
    harness.state_mut().ui.shelf_panes.filter_by_path = true;
    harness.run_steps(3);

    let initial_share = harness.state().ui.shelf_panes_split;
    let heading = harness.get_by_label("FILTER BY PATH").rect();
    let save_button = harness.get_by_label("Save the current search").rect().top();
    assert!(heading.top() > save_button, "the path pane sits below the saved searches");

    const DRAG_DISTANCE_POINTS: f32 = 120.0;
    const POSITION_TOLERANCE_POINTS: f32 = 2.0;
    let divider = harness.get_by_label("Resize Assets filter panes").rect();
    let from = egui::pos2(divider.center().x, divider.top() + 1.0);
    let to = from + egui::vec2(0.0, DRAG_DISTANCE_POINTS);
    harness.hover_at(from);
    harness.run_steps(1);
    harness.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    harness.run_steps(2);

    assert_eq!(harness.state().ui.shelf_panes_split, initial_share, "an off-center press leaves the splitter in place");

    for step in 1..=4 {
        harness.hover_at(from + (to - from) * (step as f32 / 4.0));
        harness.run_steps(1);
    }
    harness.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.run_steps(3);

    let moved = harness.get_by_label("FILTER BY PATH").rect();
    assert!((moved.top() - heading.top() - DRAG_DISTANCE_POINTS).abs() < POSITION_TOLERANCE_POINTS, "the path pane follows the drag: {heading:?} to {moved:?}");
    assert!(harness.state().ui.shelf_panes_split > initial_share, "saved searches take a larger share");
    assert!(harness.query_by_label("User fonts").is_some(), "the shorter path pane still lists its rows");

    // States saved before the splitter existed still load the default
    let saved = serde_json::to_value(&harness.state().ui).unwrap();
    let restored: texturecraft_ui_egui::state::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(restored.shelf_panes_split, harness.state().ui.shelf_panes_split);
    let mut older = saved;
    assert!(older.as_object_mut().unwrap().remove("shelf_panes_split").is_some(), "the saved state names the split");
    let restored: texturecraft_ui_egui::state::UiState = serde_json::from_value(older).unwrap();
    assert_eq!(restored.shelf_panes_split, initial_share);
}

#[test]
fn the_smallest_saved_search_pane_can_scroll_to_and_select_the_last_query() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().ui.shelf_panes.saved_searches = true;
    harness.state_mut().ui.shelf_panes.filter_by_path = true;
    harness.state_mut().ui.shelf_panes_split = 0.0;
    harness.state_mut().ui.shelf_saved_searches =
        vec![SavedSearch { name: "Rust search".into(), filter: "rust".into(), tab: MATERIALS_TYPE, path: Vec::new() }];
    harness.run_steps(3);

    scroll_pane_to_end(&mut harness, "SAVED SEARCHES");

    let last_row = harness.get_by_label("Rust search").rect();
    let next_heading = harness.get_by_label("FILTER BY PATH").rect();
    assert!(last_row.center().y < next_heading.top(), "the final search should be inside its pane: {last_row:?}");

    harness.get_by_label("Rust search").click();
    harness.run_steps(3);

    assert_eq!(harness.state().ui.shelf_filter, "rust");
}

#[test]
fn the_smallest_path_pane_can_scroll_to_its_last_library() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().ui.shelf_panes.saved_searches = true;
    harness.state_mut().ui.shelf_panes.filter_by_path = true;
    harness.state_mut().ui.shelf_panes_split = 1.0;
    harness.run_steps(3);

    scroll_pane_to_end(&mut harness, "FILTER BY PATH");

    let last_row = harness.get_by_label("User fonts").rect();
    let toolbar = button(&harness, "Filter by path").rect();
    assert!(last_row.center().y < toolbar.top(), "the final library should be above the toolbar: {last_row:?}");
}

fn scroll_pane_to_end(harness: &mut Harness<'_, TextureCraftApp>, heading: &str) {
    let heading = harness.get_by_label(heading).rect();
    harness.hover_at(egui::pos2(heading.left() + 24.0, heading.bottom() + 12.0));
    harness.run_steps(1);

    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -1000.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(8);
}

#[test]
fn the_splitter_is_exposed_as_a_resizable_control() {
    let mut harness = shelf(REFERENCE_WINDOW);
    harness.state_mut().ui.shelf_panes.saved_searches = true;
    harness.state_mut().ui.shelf_panes.filter_by_path = true;
    harness.run_steps(3);

    let before = harness.state().ui.shelf_panes_split;

    let divider = harness.get_by_label("Resize Assets filter panes");
    assert_eq!(divider.accesskit_node().role(), egui::accesskit::Role::Splitter);
    divider.increment_accesskit();
    harness.run_steps(3);

    assert!(harness.state().ui.shelf_panes_split > before);
}
