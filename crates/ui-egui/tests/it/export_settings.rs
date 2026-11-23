//! Export textures › SETTINGS (texturecraft #671): the reference's lists, menus and tooltips, and
//! Cancel going back to the settings the window opened with.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use texturecraft_ui_egui::TextureCraftApp;

/// The Export textures window open on SETTINGS
fn harness() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(256, 64).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    h
}

/// Hover the newest node labelled `label` until its tooltip shows
fn hover(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    let center = h.get_all_by_label(label).last().unwrap_or_else(|| panic!("no {label}")).rect().center();
    h.hover_at(center);
    h.run_steps(30);
}

/// Open the dropdown showing `value`
fn open_combo(h: &mut Harness<'_, TextureCraftApp>, value: &str) {
    h.get_all_by_role(egui::Role::ComboBox).find(|c| c.value().as_deref() == Some(value)).unwrap_or_else(|| panic!("no dropdown at {value}")).click();
    h.run_steps(4);
}

fn click(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    h.get_all_by_label(label).last().unwrap_or_else(|| panic!("no {label}")).click();
    h.run_steps(4);
}

#[test]
fn cancel_undoes_the_changes_and_the_window_reopens_on_settings() {
    let mut h = harness();
    let before = h.state().ui.export.clone();
    let e = &mut h.state_mut().ui.export;
    e.size = 512;
    e.padding = "dilation".into();
    e.skip = vec![0];
    e.tab = 2;
    h.run_steps(3);
    click(&mut h, "Cancel");
    let app = h.state();
    assert!(!app.ui.windows.export);
    assert_eq!((app.ui.export.size, app.ui.export.padding.as_str(), app.ui.export.skip.as_slice()), (before.size, before.padding.as_str(), &[][..]));

    h.state_mut().ui.windows.export = true;
    h.run_steps(4);
    assert_eq!(h.state().ui.export.tab, 0, "the window reopens on SETTINGS");

    // Save settings keeps them
    h.state_mut().ui.export.size = 1024;
    h.run_steps(3);
    click(&mut h, "Save settings");
    assert!(!h.state().ui.windows.export);
    assert_eq!(h.state().ui.export.size, 1024);
}

#[test]
fn closing_the_window_is_cancel() {
    let mut h = harness();
    h.state_mut().ui.export.size = 2048;
    h.run_steps(3);
    h.get_by_role_and_label(egui::Role::Button, "Close window").click();
    h.run_steps(4);
    assert!(!h.state().ui.windows.export);
    assert_eq!(h.state().ui.export.size, 0);
}

#[test]
fn the_lists_hold_the_reference_entries() {
    let mut h = harness();
    open_combo(&mut h, "Based on output template");
    for item in ["bmp", "jpeg", "png", "targa", "tiff", "hdr", "exr", "webp"] {
        assert!(h.query_by_label(item).is_some(), "file type {item}");
    }
    assert!(h.query_by_label("tga").is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(3);

    open_combo(&mut h, "Based on each Texture Set's size");
    for item in ["Maximum size for all Texture Sets", "128", "256", "8192"] {
        assert!(h.query_by_label(item).is_some(), "size {item}");
    }
    click(&mut h, "Maximum size for all Texture Sets");
    assert_eq!(h.state().ui.export.size, texturecraft_ui_egui::state::SIZE_OF_LARGEST_SET);

    open_combo(&mut h, "Document channels + Normal + AO (No Alpha)");
    for item in ["2D View", "Document channels + Normal + AO (With Alpha)", "Sketchfab", "glTF PBR Metal Roughness", "Unreal Engine (Packed)"] {
        assert!(h.query_by_label(item).is_some(), "template {item}");
    }
    h.key_press(egui::Key::Escape);
    h.run_steps(3);

    open_combo(&mut h, "Dilation infinite");
    let order: Vec<String> = ["No padding (passthrough)", "Dilation infinite", "Dilation + transparent", "Dilation + default background color"]
        .iter()
        .map(|l| format!("{:.0}", h.get_all_by_label(l).last().unwrap().rect().top()))
        .collect();
    let mut sorted = order.clone();
    sorted.sort_by_key(|y| y.parse::<i32>().unwrap());
    assert_eq!(order, sorted, "paddings in the reference order");
}

#[test]
fn the_default_template_has_the_reference_name_and_old_settings_follow_it() {
    let mut h = harness();
    assert_eq!(h.state().ui.export.preset, "Document channels + Normal + AO (No Alpha)");
    h.state_mut().ui.export.preset = "Document channels + Normal + AO".into();
    h.run_steps(3);
    assert_eq!(h.state().ui.export.preset, "Document channels + Normal + AO (No Alpha)");
}

#[test]
fn the_set_list_menu_checks_unchecks_and_inverts() {
    let mut h = harness();
    let sets = h.state().session.set_count();
    click(&mut h, egui_phosphor::regular::LIST_CHECKS);
    click(&mut h, "Uncheck all");
    assert_eq!(h.state().ui.export.skip.len(), sets);
    click(&mut h, egui_phosphor::regular::LIST_CHECKS);
    click(&mut h, "Invert all");
    assert!(h.state().ui.export.skip.is_empty());
    h.state_mut().ui.export.skip = vec![0];
    click(&mut h, egui_phosphor::regular::LIST_CHECKS);
    click(&mut h, "Check all");
    assert!(h.state().ui.export.skip.is_empty());
}

#[test]
fn the_output_directory_resets_and_the_dilation_follows_the_padding() {
    let mut h = harness();
    let default = h.state().ui.export.dir.clone();
    h.state_mut().ui.export.dir = "/somewhere/else".into();
    h.run_steps(3);
    hover(&mut h, "R");
    assert!(h.query_by_label("Reset to default").is_some());
    click(&mut h, "R");
    assert_eq!(h.state().ui.export.dir, default);

    hover(&mut h, "16");
    assert!(h.query_by_label("This padding mode does not use a dilation width").is_some());
    h.state_mut().ui.export.padding = "dilation".into();
    h.run_steps(3);
    hover(&mut h, "16");
    assert!(h.query_all_by_label("Dilation in pixels").next().is_some());
    click(&mut h, "16");
    assert!(h.query_by_label("Dilation (pixels)").is_some());

    // A click on the slider sets the width and leaves the menu open
    h.get_by_role(egui::Role::Slider).click();
    h.run_steps(4);
    assert_ne!(h.state().ui.export.dilation, 16);
    assert!(h.query_by_label("Dilation (pixels)").is_some(), "the menu stays open");
}

#[test]
fn export_actions_explain_what_they_change() {
    let mut h = harness();
    for (button, tip) in [
        ("Include all", "Select every texture set, UV tile and output map"),
        ("Reset all", "Restore the export defaults and select every texture"),
        ("Cancel", "Restore the settings from when you opened this window"),
        ("Save settings", "Keep these settings for the next export"),
        ("Export", "Write the selected textures using the settings above"),
    ] {
        hover(&mut h, button);
        assert!(h.query_by_label(tip).is_some(), "{button}: {tip}");
    }
}

#[test]
fn the_page_reset_puts_the_parameters_back() {
    let mut h = harness();
    let e = &mut h.state_mut().ui.export;
    e.size = 4096;
    e.padding = "none".into();
    e.skip = vec![0];
    h.run_steps(3);
    click(&mut h, "Restore the default export parameters");
    let e = &h.state().ui.export;
    assert_eq!((e.size, e.padding.as_str()), (0, "infinite"));
    assert_eq!(e.skip, vec![0], "the texture set ticks are not parameters of the page");
}
