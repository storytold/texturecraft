//! Every docked panel group sits under one header bar: its names at the left, the shown one raised as a box when the group has several, a close × at the right

use craft_ui::docking::{Action, Placement};
use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::docking::{Dock, Panel, permissions};

use crate::{BAKE_RESOLUTION, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION, click_at, panel_close_button};

const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 960.0);

/// A floating window over the viewport, `[x, y, width, height]`
const FLOATING_RECT: [f32; 4] = [600.0, 200.0, 320.0, 400.0];

fn app() -> Harness<'static, TextureCraftApp> {
    let session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);

    harness
}

/// The rect of the tab whose name starts with `name`
fn tab_rect(harness: &Harness<'_, TextureCraftApp>, name: &str) -> egui::Rect {
    harness
        .get_all_by_role(Role::Tab)
        .find(|tab| tab.accesskit_node().label().is_some_and(|label| label.starts_with(name)))
        .map(|tab| tab.rect())
        .unwrap_or_else(|| panic!("a {name} tab"))
}

#[test]
fn every_docked_panel_header_ends_in_its_own_close_button() {
    let harness = app();
    let names = ["Assets", "Texture Sets", "Layers", "Properties - "];
    let tabs: Vec<egui::Rect> = names.iter().map(|name| tab_rect(&harness, name)).collect();
    for (name, tab) in names.iter().zip(&tabs) {
        let close = panel_close_button(&harness, *tab);

        // No other group's tab between a tab and its close button: the button is on its own bar
        let between = tabs.iter().any(|other| other.y_range().contains(close.center().y) && other.min.x > tab.max.x && other.max.x < close.min.x);
        assert!(!between, "{name} has no close button on its own header");
    }
}

#[test]
fn the_header_close_button_closes_its_panel() {
    let mut harness = app();
    assert!(harness.state().ui.dock.is_visible(Panel::Properties));

    let close = panel_close_button(&harness, tab_rect(&harness, "Properties - "));
    click_at(&mut harness, close.center());

    assert!(!harness.state().ui.dock.is_open(Panel::Properties));
    assert!(harness.state().ui.dock.is_visible(Panel::Layers), "only the panel under the bar closes");
}

/// With an effect selected the Properties header names its type, not the effect's own name (every filter reads "Filter")
#[test]
fn the_properties_header_names_the_selected_effects_type() {
    let mut harness = app();
    let session = &mut harness.state_mut().session;
    let layer = texturecraft_engine::execute(session, "layer.add.fill", &serde_json::Value::Null).unwrap()["layer"].as_u64().unwrap();
    let blur = texturecraft_engine::execute(session, "effect.add.filter", &serde_json::json!({"layer": layer, "target": "content", "filter": "blur"})).unwrap()
        ["effect"]
        .as_u64()
        .unwrap();
    texturecraft_engine::execute(session, "effect.set", &serde_json::json!({"effect": blur, "name": "Soften edges"})).unwrap();
    texturecraft_engine::execute(session, "layer.select", &serde_json::json!({"layer": layer, "target": blur})).unwrap();
    harness.run_steps(SETTLE_FRAMES);

    tab_rect(&harness, "Properties - Filter");

    let level = texturecraft_engine::execute(&mut harness.state_mut().session, "effect.add.levels", &serde_json::json!({"layer": layer, "target": "content"}))
        .unwrap()["effect"]
        .as_u64()
        .unwrap();
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.select", &serde_json::json!({"layer": layer, "target": level})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    tab_rect(&harness, "Properties - Levels");
}

/// How far a header under another panel reaches above its tab strip: the 6 px gutter less a 2 px separation (docking/header_band.rs)
const HEADER_RISE: f32 = 4.0;

/// Room between a later tab's box and its tab's left edge (docking/shown_group_tab.rs, private to the crate)
const BOX_INSET: f32 = 4.0;

/// Rounded boxes painted this frame over `tab` whose top is `rise` above it: the raised box of a shown tab
fn tab_boxes(harness: &Harness<'_, TextureCraftApp>, tab: egui::Rect, rise: f32) -> Vec<egui::Rect> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) => {
                let rises = (rect.rect.top() - (tab.top() - rise)).abs() < 0.5;
                let raised = rises && rect.corner_radius != egui::CornerRadius::ZERO && tab.x_range().contains(rect.rect.center().x);
                raised.then_some(rect.rect)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_groups_shown_tab_rises_as_a_box_and_the_others_and_a_lone_panels_do_not() {
    let mut harness = app();
    let layers = tab_rect(&harness, "Layers");
    let settings = tab_rect(&harness, "Texture Set Settings");

    assert_eq!(tab_boxes(&harness, layers, HEADER_RISE).len(), 1, "the shown Layers tab has its box, from the top of its header band");
    assert_eq!(tab_boxes(&harness, settings, HEADER_RISE).len(), 0, "the hidden tab is its name only");
    assert_eq!(tab_boxes(&harness, tab_rect(&harness, "Properties - "), HEADER_RISE).len(), 0, "a panel alone keeps its plain header");

    click_at(&mut harness, settings.center());
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(tab_boxes(&harness, tab_rect(&harness, "Texture Set Settings"), HEADER_RISE).len(), 1, "the box goes with the shown tab");
    assert_eq!(tab_boxes(&harness, tab_rect(&harness, "Layers"), HEADER_RISE).len(), 0);
}

#[test]
fn the_selected_dock_tab_keeps_its_keyboard_focus_outline_visible() {
    let mut harness = app();
    let layers = tab_rect(&harness, "Layers");
    let tab_id = egui::Id::unique("texturecraft-docking").with(("tab", Panel::Layers));
    harness.ctx.memory_mut(|memory| memory.request_focus(tab_id));
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(harness.ctx.memory(|memory| memory.focused()), Some(tab_id));
    let raised_box = tab_boxes(&harness, layers, HEADER_RISE)[0];
    let shapes = &harness.output().shapes;
    let box_index = shapes.iter().position(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.rect == raised_box)).unwrap();
    assert!(
        shapes[box_index + 1..].iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(rect)
            if rect.stroke.width > 0.0 && layers.contains_rect(rect.rect) && rect.rect.contains(layers.center()))
        }),
        "the focus outline is painted above the selected tab"
    );
}

#[test]
fn the_first_tabs_box_starts_on_the_panels_edge_and_a_later_tabs_box_keeps_its_inset() {
    let mut harness = app();
    let layers = tab_rect(&harness, "Layers");
    let boxes = tab_boxes(&harness, layers, HEADER_RISE);
    assert!(!boxes.is_empty(), "the shown Layers tab has its box");
    assert!(boxes.iter().all(|shown| (shown.left() - layers.left()).abs() < 0.5), "{boxes:?} starts on the panel's left edge {}", layers.left());

    let settings = tab_rect(&harness, "Texture Set Settings");

    click_at(&mut harness, settings.center());
    harness.run_steps(SETTLE_FRAMES);

    let boxes = tab_boxes(&harness, settings, HEADER_RISE);
    assert!(!boxes.is_empty(), "the shown Texture Set Settings tab has its box");
    assert!(boxes.iter().all(|shown| (shown.left() - (settings.left() + BOX_INSET)).abs() < 0.5), "{boxes:?} keeps its air from the Layers tab");
}

/// The middle of the first capital's ink in each painted text reading `name`
fn capital_middles(harness: &Harness<'_, TextureCraftApp>, name: &str) -> Vec<f32> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == name => {
                let row = text.galley.rows.first()?;
                let glyph = row.glyphs.iter().find(|glyph| glyph.chr.is_uppercase())?;
                Some(text.pos.y + row.pos.y + glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y / 2.0)
            }
            _ => None,
        })
        .collect()
}

/// Names' capitals and the close × share the header band's middle: the tab strip plus, under another panel, the gutter above it less the separation
#[test]
fn header_names_capitals_and_close_buttons_sit_on_the_header_bands_middle() {
    let harness = app();
    let headers = [
        ("Texture Sets", "TEXTURE SETS", 0.0),
        ("Layers", "LAYERS", HEADER_RISE),
        ("Texture Set Settings", "TEXTURE SET SETTINGS", HEADER_RISE),
        ("Properties - ", "PROPERTIES - PAINT", HEADER_RISE),
    ];
    for (tab, name, rise) in headers {
        let tab = tab_rect(&harness, tab);
        let middle = (tab.top() - rise + tab.bottom()) / 2.0;
        let capitals = capital_middles(&harness, name);
        assert!(!capitals.is_empty(), "{name} is painted");
        assert!(capitals.iter().all(|capital| (capital - middle).abs() < 0.75), "{name}'s capitals at {capitals:?}, its band's middle at {middle}");
    }

    let properties = tab_rect(&harness, "Properties - ");
    let close = panel_close_button(&harness, properties);
    let middle = (properties.top() - HEADER_RISE + properties.bottom()) / 2.0;
    assert!((close.center().y - middle).abs() < 0.5, "the close × at {}, the band's middle at {middle}", close.center().y);
}

/// A floating group's names sit on one line: the shown name, painted again over its tab, and the dimmed one beside it (the dimmed one kept its galley's middle)
#[test]
fn a_floating_groups_names_share_one_line() {
    let mut harness = app();
    let mut workspace = harness.state().ui.dock.workspace().clone();
    let actions = [
        Action::Float { panel: Panel::Properties, rect: FLOATING_RECT },
        Action::Move { panel: Panel::TextureSetSettings, anchor: Panel::Properties, placement: Placement::Tab { before: None } },
        Action::Activate { panel: Panel::Properties },
    ];
    workspace.apply_all(actions, permissions).unwrap();
    harness.state_mut().ui.dock = Dock::from_value(serde_json::to_value(workspace).unwrap());
    harness.run_steps(SETTLE_FRAMES);

    let shown = capital_middles(&harness, "PROPERTIES - PAINT");
    let dimmed = capital_middles(&harness, "TEXTURE SET SETTINGS");
    assert!(!shown.is_empty() && !dimmed.is_empty(), "both names are painted");
    assert!(shown.iter().chain(&dimmed).all(|capital| (capital - shown[0]).abs() < 0.5), "the shown name's capitals at {shown:?}, the other's at {dimmed:?}");
}

/// The tab menu's panel names keep their plain layout: only the header bar pads a name to centre its capitals on the band
#[test]
fn a_tab_menus_panel_names_keep_their_plain_layout() {
    let mut harness = app();
    let properties = tab_rect(&harness, "Properties - ");
    let button = egui::PointerButton::Secondary;
    harness.hover_at(properties.center());
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos: properties.center(), button, pressed, modifiers: egui::Modifiers::NONE });
        harness.run_steps(1);
    }
    harness.run_steps(SETTLE_FRAMES);

    let move_to = harness.get_by_role_and_label(Role::Button, "Move Panel to Group").rect();
    harness.hover_at(move_to.center());
    harness.run_steps(SETTLE_FRAMES);

    let entry = harness.get_by_role_and_label(Role::Button, "Layers").rect();
    let padded: Vec<f32> = harness
        .output()
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == "LAYERS" && entry.contains(text.pos) => Some(text.galley.job.first_row_min_height),
            _ => None,
        })
        .collect();

    assert!(!padded.is_empty(), "the menu paints the Layers entry");
    assert!(padded.iter().all(|height| *height == 0.0), "the Layers entry is padded to {padded:?}");
}

/// Where the label on `name`'s row and to its right ends, for each label named `name`
fn row_ends(harness: &Harness<'_, TextureCraftApp>, name: &str) -> Vec<f32> {
    let names = harness.get_all_by_role_and_label(Role::Label, name).map(|node| node.rect());
    names
        .filter_map(|name| {
            let on_row = harness.get_all_by_role(Role::Label).map(|node| node.rect());
            on_row.filter(|rect| rect.left() > name.right() && (rect.center().y - name.center().y).abs() < 1.0).map(|rect| rect.right()).reduce(f32::max)
        })
        .collect()
}

/// Texture Set Settings keeps Base color's remove room, so its format ends where the other channels' formats do (it ended 24 px right)
#[test]
fn every_channels_format_ends_on_one_column() {
    let mut harness = app();
    let settings = tab_rect(&harness, "Texture Set Settings");
    click_at(&mut harness, settings.center());
    harness.run_steps(SETTLE_FRAMES);

    let base_color = row_ends(&harness, "Base color");
    let roughness = row_ends(&harness, "Roughness");

    assert!(base_color.len() == 1 && base_color == roughness, "Base color's format ends at {base_color:?}, Roughness's at {roughness:?}");
}
