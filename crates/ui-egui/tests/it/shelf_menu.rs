//! An asset tile's right-click menu: the info block says what the asset is, and its rows act on it

use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::materials::MATERIALS;
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::file_dialogs::Answer;

/// The types' positions in the shelf's type row (its table is crate-private)
const MATERIALS_TYPE: usize = 0;
const BRUSHES_TYPE: usize = 4;
const TEXTURES_TYPE: usize = 6;

/// The sample project with the shelf on type `tab`
fn shelf(tab: usize) -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1600.0, 960.0))
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.shelf_tab = tab;
    harness.run_steps(4);
    harness
}

fn right_click(harness: &mut Harness<'_, TextureCraftApp>, label: &str) {
    harness.get_all_by_label(label).next().unwrap_or_else(|| panic!("no tile {label}")).click_secondary();
    harness.run_steps(3);
}

#[test]
fn a_built_in_tile_menu_says_what_the_asset_is_and_where_it_lives() {
    // The grid's first plain material (by name), so its tile is in view
    let material = MATERIALS.iter().filter(|material| !material.smart).min_by_key(|material| material.label.to_lowercase()).unwrap();
    let mut harness = shelf(MATERIALS_TYPE);

    right_click(&mut harness, material.label);

    for row in [
        format!("Name {}", material.label),
        "Usages material".to_string(),
        format!("Path /Starter assets/Materials/{}/", material.category),
        format!("Category {}", material.category),
    ] {
        assert!(harness.query_by_label(&row).is_some(), "the menu has the info row {row:?}");
    }
    assert!(harness.get_by_label("Export resource...").accesskit_node().is_disabled(), "a built-in has no file to export");
    assert!(harness.get_by_label("Show in").accesskit_node().is_disabled(), "a built-in has no file to show");

    harness.get_by_label("Close").click();
    harness.run_steps(3);

    assert!(harness.query_by_label("Usages material").is_none(), "Close closes the menu");
}

#[test]
fn export_resource_writes_the_image_as_imported() {
    let mut harness = shelf(TEXTURES_TYPE);
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([180, 90, 40, 255])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    let name = harness.state_mut().session.import_resource_bytes("plate.png", bytes.clone()).unwrap();
    harness.run_steps(3);
    let dir = super::temp_dir::TempDir::new("shelf-export-resource");
    let target = dir.join("exported.png");
    let answer = target.clone();
    harness.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![answer.clone()]))));

    right_click(&mut harness, &name);
    harness.get_by_label("Export resource...").click();
    harness.run_steps(4);

    assert_eq!(std::fs::read(&target).unwrap(), bytes, "the exported file is the resource's own bytes");
}

fn preset_names(harness: &Harness<'_, TextureCraftApp>) -> Vec<String> {
    harness.state().session.brush_presets.list.iter().map(|preset| preset.name.clone()).collect()
}

/// Selects the menu's Name field text and types `text`
fn type_name(harness: &mut Harness<'_, TextureCraftApp>, text: &str) {
    harness.get_by_role_and_label(egui::Role::TextInput, "Name").click();
    harness.run_steps(3);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.event(egui::Event::Text(text.into()));
    harness.run_steps(3);
}

#[test]
fn a_saved_brush_menu_renames_replaces_and_deletes_it() {
    let dir = super::temp_dir::TempDir::new("shelf-rename-brush");
    let mut harness = shelf(BRUSHES_TYPE);
    harness.state_mut().session.brush_presets.dir = Some(dir.to_path_buf());

    // Keep the saved brush in view at the beginning of the sorted grid
    texturecraft_engine::execute(&mut harness.state_mut().session, "brush.preset.store", &json!({"name": "Aa ink"})).unwrap();
    harness.run_steps(3);

    right_click(&mut harness, "Aa ink");
    type_name(&mut harness, "Never applied");
    harness.key_press(Key::Escape);
    harness.run_steps(3);

    assert_eq!(preset_names(&harness), ["Aa ink"], "Escape keeps the name");

    right_click(&mut harness, "Aa ink");
    type_name(&mut harness, "Aa pen");
    harness.key_press(Key::Enter);
    harness.run_steps(3);

    assert_eq!(preset_names(&harness), ["Aa pen"], "Enter renames the brush");

    texturecraft_engine::execute(&mut harness.state_mut().session, "tool.brush", &json!({"size": 77})).unwrap();
    right_click(&mut harness, "Aa pen");
    harness.get_by_label("Replace with the current brush").click();
    harness.run_steps(3);

    assert_eq!(harness.state().session.brush_presets.list[0].brush, harness.state().session.tool.brush, "the preset holds the current brush");

    right_click(&mut harness, "Aa pen");
    harness.get_by_label("Delete").click();
    harness.run_steps(3);

    assert!(preset_names(&harness).is_empty(), "Delete removes the brush");
}

#[test]
fn delete_removes_a_project_image() {
    let mut harness = shelf(TEXTURES_TYPE);
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([180, 90, 40, 255])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    let name = harness.state_mut().session.import_resource_bytes("plate.png", bytes).unwrap();
    harness.run_steps(3);

    right_click(&mut harness, &name);
    harness.get_by_label("Delete").click();
    harness.run_steps(3);

    assert!(!harness.state().session.doc.resources.contains_key(&name));
}

#[test]
fn a_project_image_menu_renames_it_keeping_its_star_and_greys_reload_without_a_file() {
    let mut harness = shelf(TEXTURES_TYPE);
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([180, 90, 40, 255])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    let name = harness.state_mut().session.import_resource_bytes("plate.png", bytes).unwrap();
    harness.run_steps(3);
    pick_from_menu(&mut harness, &name, "Add to favorites");

    right_click(&mut harness, &name);
    assert!(harness.get_by_label("Reload").accesskit_node().is_disabled(), "an image not imported from a file can't reload");
    type_name(&mut harness, "stone");
    harness.key_press(Key::Enter);
    harness.run_steps(3);

    let resources = &harness.state().session.doc.resources;
    assert!(resources.contains_key("stone.png") && !resources.contains_key(&name), "Enter renames the image, keeping its extension");
    let favorites: Vec<&str> = harness.state().ui.shelf_favorites.iter().map(|favorite| favorite.id.as_str()).collect();
    assert_eq!(favorites, ["stone.png"], "the star follows the rename");
}

#[test]
fn show_in_opens_the_folder_of_a_saved_brush() {
    let dir = super::temp_dir::TempDir::new("shelf-show-in");
    let mut harness = shelf(BRUSHES_TYPE);
    harness.state_mut().session.brush_presets.dir = Some(dir.to_path_buf());
    texturecraft_engine::execute(&mut harness.state_mut().session, "brush.preset.store", &json!({"name": "Aa ink"})).unwrap();
    harness.run_steps(3);

    right_click(&mut harness, "Aa ink");
    harness.get_by_label("Show in").click();
    harness.run_steps(3);
    let item = if cfg!(target_os = "windows") {
        "Show in Explorer..."
    } else if cfg!(target_os = "macos") {
        "Show in Finder..."
    } else {
        "Show in file manager..."
    };
    harness.get_by_label(item).click();
    harness.run_steps(3);

    let shown = harness.state().file_dialogs.shown_folders().to_vec();
    assert!(shown.iter().any(|folder| folder.canonicalize().ok() == dir.canonicalize().ok()), "the brush's folder is shown: {shown:?}");
}

#[test]
fn shift_f10_opens_the_menu_of_the_focused_tile_on_its_first_row_and_escape_returns_to_it() {
    let mut harness = shelf(TEXTURES_TYPE);
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([180, 90, 40, 255])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    let name = harness.state_mut().session.import_resource_bytes("plate.png", bytes).unwrap();
    harness.run_steps(3);

    harness.get_all_by_label(&name).next().unwrap().focus();
    harness.run_steps(2);

    assert!(harness.get_all_by_label(&name).next().unwrap().is_focused(), "the tile takes the focus");
    harness.key_press_modifiers(Modifiers::SHIFT, Key::F10);
    harness.run_steps(3);

    assert!(harness.query_by_label("Usages texture").is_some(), "Shift+F10 opens the menu");
    assert!(harness.get_by_label("Delete").is_focused(), "the first row that can be picked takes the focus (Reload can't: no file)");

    harness.key_press(Key::Escape);
    harness.run_steps(3);

    assert!(harness.query_by_label("Usages texture").is_none(), "Escape closes it");
    assert!(harness.get_all_by_label(&name).next().unwrap().accesskit_node().is_focused(), "the focus is back on the tile");
}

/// Whether the tile `first` comes before the tile `second` in the grid (rows top to bottom, each left to right)
fn comes_before(harness: &Harness<'_, TextureCraftApp>, first: &str, second: &str) -> bool {
    let place = |label: &str| {
        let rect = harness.get_by_label(label).rect();
        (rect.top(), rect.left())
    };
    place(first) < place(second)
}

fn pick_from_menu(harness: &mut Harness<'_, TextureCraftApp>, tile: &str, row: &str) {
    right_click(harness, tile);
    harness.get_by_label(row).click();
    harness.run_steps(3);
}

#[test]
fn favorites_come_first_newest_first_with_a_star_until_removed() {
    let mut plain: Vec<_> = MATERIALS.iter().filter(|material| !material.smart).collect();
    plain.sort_by_key(|material| material.label.to_lowercase());
    let (first, second, third) = (plain[0].label, plain[1].label, plain[2].label);
    let mut harness = shelf(MATERIALS_TYPE);

    pick_from_menu(&mut harness, third, "Add to favorites");
    pick_from_menu(&mut harness, second, "Add to favorites");

    assert!(comes_before(&harness, second, third) && comes_before(&harness, third, first), "favorites first, the newest first");
    assert_eq!(harness.get_by_label(second).accesskit_node().description().as_deref(), Some("Favorite"), "the star is announced");
    assert_eq!(harness.get_by_label(first).accesskit_node().description(), None);

    pick_from_menu(&mut harness, second, "Remove from favorites");

    assert!(comes_before(&harness, first, second), "a removed favorite goes back to its place by name");
    assert!(comes_before(&harness, third, first), "the other favorite stays first");
}

#[test]
fn set_custom_preview_shows_a_picked_picture_until_removed() {
    let material = MATERIALS.iter().filter(|material| !material.smart).min_by_key(|material| material.label.to_lowercase()).unwrap();
    let mut harness = shelf(MATERIALS_TYPE);
    let dir = super::temp_dir::TempDir::new("shelf-custom-preview");
    let picture = dir.join("preview.png");
    image::RgbaImage::from_pixel(8, 4, image::Rgba([40, 160, 90, 255])).save(&picture).unwrap();
    let not_a_picture = dir.join("notes.png");
    std::fs::write(&not_a_picture, b"not a picture").unwrap();
    let answers = std::sync::Arc::new(std::sync::Mutex::new(vec![picture.clone(), not_a_picture]));
    let pick = answers.clone();
    harness.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![pick.lock().unwrap().remove(0)]))));
    let picked =
        |harness: &Harness<'_, TextureCraftApp>| harness.state().ui.shelf_custom_previews.iter().map(|preview| preview.file.clone()).collect::<Vec<_>>();

    pick_from_menu(&mut harness, material.label, "Set custom preview...");
    harness.run_steps(2);

    assert_eq!(picked(&harness), std::slice::from_ref(&picture), "the tile shows the picked picture");

    pick_from_menu(&mut harness, material.label, "Set custom preview...");
    harness.run_steps(2);

    assert_eq!(picked(&harness), [picture], "a file that isn't a picture leaves the preview as it was");

    pick_from_menu(&mut harness, material.label, "Remove custom preview");

    assert!(picked(&harness).is_empty(), "the tile shows its own thumbnail again");
    right_click(&mut harness, material.label);
    assert!(harness.query_by_label("Remove custom preview").is_none(), "only a tile with a custom preview offers to remove it");
}

#[test]
fn custom_preview_textures_live_until_their_last_tile_releases_them() {
    let mut materials: Vec<_> = MATERIALS.iter().filter(|material| !material.smart).collect();
    materials.sort_by_key(|material| material.label.to_lowercase());
    let (first, second) = (materials[0].label, materials[1].label);
    let mut harness = shelf(MATERIALS_TYPE);
    let directory = super::temp_dir::TempDir::new("shared-shelf-preview");
    let shared = directory.join("shared.png");
    let replacement = directory.join("replacement.png");
    let last = directory.join("last.png");
    for path in [&shared, &replacement, &last] {
        image::RgbaImage::from_pixel(164, 164, image::Rgba([40, 160, 90, 255])).save(path).unwrap();
    }
    let mut answers = vec![shared.clone(), shared, replacement, last].into_iter();
    harness.state_mut().file_dialogs.set_override(Some(Box::new(move |_| Answer::Paths(vec![answers.next().unwrap()]))));

    let allocated = |harness: &Harness<'_, TextureCraftApp>| {
        harness
            .ctx
            .tex_manager()
            .read()
            .allocated()
            .filter(|(_, texture)| texture.name.starts_with("custom_preview_"))
            .fold((0, 0), |(count, bytes), (_, texture)| (count + 1, bytes + texture.size[0] * texture.size[1] * texture.bytes_per_pixel))
    };

    pick_from_menu(&mut harness, first, "Set custom preview...");
    pick_from_menu(&mut harness, second, "Set custom preview...");
    let shared = allocated(&harness);
    pick_from_menu(&mut harness, first, "Set custom preview...");
    let separate = allocated(&harness);
    pick_from_menu(&mut harness, second, "Remove custom preview");
    let remaining = allocated(&harness);
    pick_from_menu(&mut harness, first, "Set custom preview...");
    let replaced_last = allocated(&harness);
    pick_from_menu(&mut harness, first, "Remove custom preview");
    let cleared = allocated(&harness);

    println!(
        "Custom preview (textures, bytes): shared {shared:?}, replaced {separate:?}, removed {remaining:?}, replaced last {replaced_last:?}, cleared {cleared:?}"
    );
    assert_eq!(shared.0, 1);
    assert!(shared.1 > 0);
    assert_eq!(separate, (2, 2 * shared.1));
    assert_eq!(remaining, shared);
    assert_eq!(replaced_last, shared);
    assert_eq!(cleared, (0, 0));
}

#[test]
fn the_shelf_uploads_only_the_project_thumbnails_in_view() {
    const RESOURCE_COUNT: usize = 100;
    let mut harness = shelf(TEXTURES_TYPE);
    harness.state_mut().ui.shelf_filter = "visible-tile".into();
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([80, 140, 40, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    for index in 0..RESOURCE_COUNT {
        harness.state_mut().session.import_resource_bytes(&format!("visible-tile-{index:03}.png"), png.clone()).unwrap();
    }

    harness.run_steps(3);

    let allocated = harness
        .ctx
        .tex_manager()
        .read()
        .allocated()
        .filter(|(_, texture)| texture.name.starts_with("res_visible-tile-"))
        .fold((0, 0), |(count, bytes), (_, texture)| (count + 1, bytes + texture.size[0] * texture.size[1] * texture.bytes_per_pixel));
    println!("Shelf of {RESOURCE_COUNT} textures: {} uploads retained, {} bytes", allocated.0, allocated.1);
    assert!(allocated.0 > 0 && allocated.0 < RESOURCE_COUNT);
}
