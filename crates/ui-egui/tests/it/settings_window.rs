//! Edit > Settings: the General, Shortcuts and Libraries pages, OK and Cancel.

use egui::{Event, Key, Modifiers, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_engine::Session;
use texturecraft_ui_egui::settings_window::{PROJECT_OPEN_NOTE, SettingsPage};
use texturecraft_ui_egui::state::{LibraryLocation, UiState};
use texturecraft_ui_egui::unavailable::NOT_AVAILABLE;
use texturecraft_ui_egui::{TextureCraftApp, menus};

#[path = "../../src/ui_state_store/memory_storage.rs"]
mod memory_storage;
use memory_storage::MemoryStorage;

/// The section captions of the General page, top to bottom.
const SECTIONS: [&str; 13] = [
    "INTERFACE OPTIONS",
    "LAYER STACK OPTIONS",
    "CAMERA OPTIONS",
    "BAKING OPTIONS",
    "PREVIEW OPTIONS",
    "TEMPORARY FILES",
    "SPARSE VIRTUAL TEXTURE",
    "IRAY HARDWARE",
    "APPEARANCE",
    "VIEWPORT",
    "RESOURCES",
    "AUTOSAVE",
    "FOLDERS",
];

fn harness(session: Session) -> Harness<'static, TextureCraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The exact-text checks need English, whatever the OS language is
    h.state_mut().ui.prefs.language = "en".into();
    h.run_steps(4);
    h
}

fn open_settings(h: &mut Harness<'_, TextureCraftApp>) {
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "ui.settings");
    h.run_steps(4);
}

fn page(h: &Harness<'_, TextureCraftApp>) -> Option<SettingsPage> {
    h.state().ui.windows.settings_draft.as_ref().map(|draft| draft.page)
}

/// Scroll the page to the widget labelled `label`, then click it.
fn click(h: &mut Harness<'_, TextureCraftApp>, label: &str) {
    h.get_by_label(label).scroll_to_me();
    h.run_steps(3);
    h.get_by_label(label).click();
    h.run_steps(4);
}

fn toggled(h: &Harness<'_, TextureCraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// The text field labelled `label`, focused and given `text`.
fn type_into(h: &mut Harness<'_, TextureCraftApp>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.run_steps(2);
    h.get_by_role_and_label(Role::TextInput, label).type_text(text);
    h.run_steps(2);
}

#[test]
fn the_left_column_switches_between_the_three_pages() {
    let mut h = harness(texturecraft_engine::sample_session(128, 32).unwrap());
    open_settings(&mut h);
    assert_eq!(page(&h), Some(SettingsPage::General));
    assert!(toggled(&h, "General"), "the chosen page is highlighted");
    assert!(!toggled(&h, "Shortcuts"));
    assert!(h.query_by_label("INTERFACE OPTIONS").is_some());

    click(&mut h, "Shortcuts");
    assert_eq!(page(&h), Some(SettingsPage::Shortcuts));
    assert!(h.query_by_label("ACTIONS").is_some());
    assert!(h.query_by_label("INTERFACE OPTIONS").is_none());

    click(&mut h, "Libraries");
    assert_eq!(page(&h), Some(SettingsPage::Libraries));
    assert!(h.query_by_label("LIBRARY LOCATIONS").is_some());
    assert!(h.query_by_label(PROJECT_OPEN_NOTE).is_some());

    click(&mut h, "General");
    assert_eq!(page(&h), Some(SettingsPage::General));
}

#[test]
fn the_general_page_lists_the_reference_sections_in_order_without_privacy() {
    let mut h = harness(texturecraft_engine::sample_session(128, 32).unwrap());
    open_settings(&mut h);

    // Rows below the scrolled view are laid out too, in page order
    let mut shown: Vec<String> = h
        .query_all(egui_kittest::kittest::by().predicate(|node| node.label().or_else(|| node.value()).is_some_and(|text| SECTIONS.contains(&text.as_str()))))
        .filter_map(|node| node.accesskit_node().label().or_else(|| node.accesskit_node().value()))
        .collect();
    // A label on screen also has a node for its text run
    shown.dedup();
    assert_eq!(shown, SECTIONS.map(String::from).to_vec());
    assert!(h.query_by_label("PRIVACY").is_none(), "no Privacy section");
    assert!(h.query_by_label("Automatically send usage statistics").is_none());
}

#[test]
fn the_general_page_starts_at_the_reference_defaults() {
    let mut h = harness(texturecraft_engine::sample_session(128, 32).unwrap());
    open_settings(&mut h);
    h.state_mut().ui.windows.settings_draft.as_mut().unwrap().prefs.language = texturecraft_ui_egui::i18n::SYSTEM_LANGUAGE.into();
    h.run_steps(3);
    assert_eq!(h.get_by_role_and_label(Role::ComboBox, "Language").value().as_deref(), Some("Default (System Language)"));
    assert_eq!(h.get_by_role_and_label(Role::ComboBox, "Viewport Scaling").value().as_deref(), Some("Auto"));
    assert_eq!(h.get_by_role_and_label(Role::SpinButton, "Local cache budget (in MB)").accesskit_node().numeric_value(), Some(4000.0));
    for label in [
        "Show keyboard helper",
        "Show world axes",
        "Use simplified thumbnails (improves performance)",
        "Save preprocessed scene files",
        "Enable live preview baking process",
        "Enable GPU raytracing",
        "Hardware support acceleration",
    ] {
        assert!(toggled(&h, label), "{label} is ticked");
    }
    for label in ["Only display the selected material when painting", "CPU"] {
        assert!(!toggled(&h, label), "{label} is not ticked");
    }
    assert!(h.query_all_by_label("Inverse direction").all(|node| node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::False)));

    let prefs = texturecraft_ui_egui::state::Prefs::default();
    assert_eq!((prefs.rotation_speed, prefs.zoom_speed, prefs.wheel_speed), (2.0, 1.0, 1.0));
}

#[test]
fn a_control_the_app_lacks_is_disabled_and_says_so_on_hover() {
    let mut h = harness(texturecraft_engine::sample_session(128, 32).unwrap());
    h.ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
    open_settings(&mut h);
    let away = h.get_by_role_and_label(Role::Tab, "Libraries").rect().center();
    for label in [
        "Show keyboard helper",
        "Show world axes",
        "Only display the selected material when painting",
        "Use simplified thumbnails (improves performance)",
        "Save preprocessed scene files",
        "Enable live preview baking process",
        "Hardware support acceleration",
        "CPU",
    ] {
        h.hover_at(away);
        h.run_steps(3);
        assert!(h.query_by_label(NOT_AVAILABLE).is_none(), "no hint before hovering {label}");
        h.get_by_label(label).scroll_to_me();
        h.run_steps(3);
        let node = h.get_by_label(label);
        assert!(node.accesskit_node().is_disabled(), "{label} is disabled");
        node.hover();
        h.run_steps(6);
        assert!(h.query_by_label(NOT_AVAILABLE).is_some(), "hovering {label} says it is not available yet");
    }
    for step in ["−", "+"] {
        assert!(h.query_all_by_label(step).any(|node| node.accesskit_node().is_disabled()), "the cache budget's {step} is disabled");
    }
    assert!(!h.get_by_label("Enable GPU raytracing").accesskit_node().is_disabled(), "GPU raytracing switches bakes to the CPU");
    for label in ["Rotation speed", "Zoom speed", "Wheel speed"] {
        assert!(h.query_by_label(label).is_some(), "{label} is shown");
    }
}

#[test]
fn cancel_drops_the_changes_and_ok_applies_and_keeps_them() {
    let mut h = harness(texturecraft_engine::sample_session(128, 32).unwrap());
    let before = h.state().ui.prefs.clone();
    open_settings(&mut h);
    h.get_by_role_and_label(Role::SpinButton, "Autosave every").set_value_accesskit(25.0);
    click(&mut h, "Enable GPU raytracing");
    h.run_steps(2);
    assert_eq!(h.state().ui.prefs, before, "edits wait for OK");
    click(&mut h, "Cancel");
    assert!(!h.state().ui.windows.settings && h.state().ui.windows.settings_draft.is_none(), "Cancel closes the window");
    assert_eq!(h.state().ui.prefs, before, "Cancel drops the edits");

    open_settings(&mut h);
    assert_eq!(
        h.get_by_role_and_label(Role::SpinButton, "Autosave every").accesskit_node().numeric_value(),
        Some(f64::from(before.autosave_minutes)),
        "a new window starts from the settings in use"
    );
    h.get_by_role_and_label(Role::SpinButton, "Autosave every").set_value_accesskit(25.0);
    h.get_by_role_and_label(Role::SpinButton, "Max CPU threads for work").set_value_accesskit(1.0);
    h.get_by_role_and_label(Role::SpinButton, "Max project size on open").set_value_accesskit(12.0);
    h.run_steps(2);
    h.query_all_by_label("Inverse direction").next().unwrap().scroll_to_me();
    h.run_steps(3);
    h.query_all_by_label("Inverse direction").next().unwrap().click();
    h.run_steps(2);
    click(&mut h, "Enable GPU raytracing");
    click(&mut h, "OK");

    let prefs = h.state().ui.prefs.clone();
    assert!(!h.state().ui.windows.settings, "OK closes the window");
    assert_eq!((prefs.autosave_minutes, prefs.work_threads, prefs.project_open_limit_gib), (25, 1, 12));
    assert!(prefs.zoom_inverse && !prefs.wheel_inverse && !prefs.gpu_raytracing);
    assert!(prefs.drag_zoom_scale() < 0.0, "inverted zoom runs the other way");

    let mut storage = MemoryStorage::default();
    eframe::App::save(h.state_mut(), &mut storage);
    let restored: UiState = serde_json::from_str(&storage.values["ui.json"]).unwrap();
    assert_eq!(restored.prefs, prefs, "the settings are kept between runs");
}

#[test]
fn a_shortcut_edited_on_the_shortcuts_page_applies_on_ok() {
    let mut h = harness(texturecraft_engine::sample_session(128, 32).unwrap());
    open_settings(&mut h);
    click(&mut h, "Shortcuts");
    let save = texturecraft_ui_egui::keymap::bindings().iter().find(|b| b.id == "file.save").unwrap();
    click(&mut h, &format!("Edit the shortcut of {}", save.label));
    assert!(h.query_by_label("Press a key...").is_some(), "the row waits for a key");
    h.event(Event::Key { key: Key::F5, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let edited = h.state().ui.windows.settings_draft.as_ref().unwrap().keymap.get("file.save").cloned();
    assert_eq!(edited.as_deref(), Some("F5"), "the row takes the pressed key");
    assert!(h.query_by_label("Press a key...").is_none(), "the row stops waiting");
    assert!(!h.state().ui.keymap.contains_key("file.save"), "the shortcut waits for OK");

    // Help > Shortcuts rebinds another item while Settings is open
    h.state_mut().ui.keymap.insert("ui.about".into(), "F9".into());

    click(&mut h, "OK");
    assert_eq!(h.state().ui.keymap.get("file.save").map(String::as_str), Some("F5"));
    assert_eq!(h.state().ui.keymap.get("ui.about").map(String::as_str), Some("F9"), "OK keeps a shortcut changed elsewhere");
}

#[test]
fn a_library_location_is_added_removed_and_kept() {
    let mut h = harness(Session::closed().unwrap());
    let folder = std::env::temp_dir().join(format!("tc-settings-library-{}", std::process::id()));
    open_settings(&mut h);
    click(&mut h, "Libraries");
    for name in ["your_assets", "starter_assets"] {
        assert!(h.query_by_label(name).is_some(), "{name} is listed");
        assert!(h.get_by_label(&format!("Remove {name}")).accesskit_node().is_disabled(), "{name} is built in");
    }

    type_into(&mut h, "Name", "my_assets");
    type_into(&mut h, "Path", &folder.display().to_string());
    click(&mut h, "Add the location");
    assert!(h.query_by_label("my_assets").is_some(), "the new location is listed");
    click(&mut h, "OK");
    let added = LibraryLocation { name: "my_assets".into(), path: folder.display().to_string() };
    assert_eq!(h.state().ui.prefs.libraries, vec![added]);

    let mut storage = MemoryStorage::default();
    eframe::App::save(h.state_mut(), &mut storage);
    let restored: UiState = serde_json::from_str(&storage.values["ui.json"]).unwrap();
    assert_eq!(restored.prefs.libraries, h.state().ui.prefs.libraries, "added locations are kept between runs");

    open_settings(&mut h);
    click(&mut h, "Libraries");
    click(&mut h, "Remove my_assets");
    click(&mut h, "OK");
    assert!(h.state().ui.prefs.libraries.is_empty());
}

#[test]
fn library_locations_cannot_change_while_a_project_is_open() {
    let mut h = harness(texturecraft_engine::sample_session(128, 32).unwrap());
    open_settings(&mut h);
    click(&mut h, "Libraries");
    assert!(h.get_by_role_and_label(Role::TextInput, "Name").accesskit_node().is_disabled());
    assert!(h.get_by_label("Add the location").accesskit_node().is_disabled());
}

#[test]
fn closing_the_window_from_elsewhere_stops_waiting_for_a_key() {
    let mut h = harness(texturecraft_engine::sample_session(128, 32).unwrap());
    open_settings(&mut h);
    click(&mut h, "Shortcuts");
    let save = texturecraft_ui_egui::keymap::bindings().iter().find(|b| b.id == "file.save").unwrap();
    click(&mut h, &format!("Edit the shortcut of {}", save.label));
    assert!(texturecraft_ui_egui::keymap::capturing(&h.ctx), "the row waits for a key");

    // Reset UI, a layout or the control channel can close the window
    h.state_mut().ui.windows.settings = false;
    h.run_steps(3);
    assert!(!texturecraft_ui_egui::keymap::capturing(&h.ctx), "shortcuts work again");
    assert!(h.state().ui.windows.settings_draft.is_none());
}
