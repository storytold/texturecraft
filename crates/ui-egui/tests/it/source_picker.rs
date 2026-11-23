//! A fill channel's source picker: Resources | Anchor points tabs, Procedurals / Textures / Alphas / Uniform tiles, a search, opened on the current source

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::core::{Channel, LayerId, LayerKind, ProceduralKind, Source};
use texturecraft_ui_egui::TextureCraftApp;

use crate::{BAKE_RESOLUTION, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION, assert_list_leaves_the_wheel_and_closes, click_widget};

/// Tall enough that Properties shows a fill's cards and an open picker without scrolling the panel
const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 1600.0);
/// Frames for the grid's scroll to the current tile to finish its animation
const SCROLL_FRAMES: usize = 25;
/// A channel slot low in the Material card, with too little room under it for the picker in [`WINDOW_SIZE`]
const SLOT_LOW_IN_THE_CARD: &str = "Height";
/// The tile the picker shows first on its Resources tab
const FIRST_TILE: &str = "Clouds";
/// A window with room for the picker under every slot
const TALL_WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 3200.0);
/// A window too short for the picker under Roughness's slot, so the snapshot shows it opened above
const SNAPSHOT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 1000.0);
/// Frames for the hovered tile's fill to show before the snapshot
const HOVER_FRAMES: usize = 8;
/// A wheel turn down over the grid, enough to scroll it past its first row
const GRID_WHEEL: egui::Vec2 = egui::vec2(0.0, -400.0);

/// The app with a new fill layer selected on its Material tab, the layer's id; `prepare` runs on the session first
fn app_with_fill(prepare: impl FnOnce(&mut texturecraft_engine::Session, LayerId)) -> (Harness<'static, TextureCraftApp>, LayerId) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    let added = texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({})).unwrap();
    let layer = LayerId(added["layer"].as_u64().unwrap());
    prepare(&mut session, layer);
    let mut harness = Harness::builder().with_size(WINDOW_SIZE).build_eframe(|cc| TextureCraftApp::new(cc, session));

    // The labels below are English; a first run would speak the system's language
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    click_widget(&mut harness, Role::Tab, "Material");
    (harness, layer)
}

fn source(harness: &Harness<'_, TextureCraftApp>, layer: LayerId, channel: Channel) -> Source {
    match &harness.state().session.doc.project.layer(layer).unwrap().kind {
        LayerKind::Fill(fill) => fill.source(channel),
        other => panic!("not a fill: {other:?}"),
    }
}

/// Clicks the slot named `slot`, which opens or closes its picker
fn click_slot(harness: &mut Harness<'_, TextureCraftApp>, slot: &str) {
    click_widget(harness, Role::ComboBox, slot);
}

/// Clicks the picker's tile labelled `label`
fn click_tile(harness: &mut Harness<'_, TextureCraftApp>, label: &str) {
    click_widget(harness, Role::ListBoxOption, label);
}

fn type_search(harness: &mut Harness<'_, TextureCraftApp>, text: &str) {
    harness.event(egui::Event::Text(text.into()));
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn a_procedural_tile_sets_the_channel_and_closes_the_picker() {
    let (mut harness, layer) = app_with_fill(|_, _| {});
    click_slot(&mut harness, "Roughness");

    assert!(harness.query_by_role_and_label(Role::Tab, "Resources").is_some(), "the picker has its Resources tab");
    assert!(harness.query_by_role_and_label(Role::Tab, "Anchor points").is_some(), "and its Anchor points tab");

    click_tile(&mut harness, FIRST_TILE);
    let Source::Procedural { pattern, .. } = source(&harness, layer, Channel::Roughness) else { panic!("a procedural") };
    assert_eq!(pattern.kind, ProceduralKind::Clouds);
    assert!(!egui::Popup::is_any_open(&harness.ctx), "a pick closes the picker");

    // The Uniform tile at the end of Resources puts the channel back to a uniform value
    click_slot(&mut harness, "Roughness");
    type_search(&mut harness, "uniform");
    click_tile(&mut harness, "Uniform value");

    assert_eq!(source(&harness, layer, Channel::Roughness), Source::uniform_of(Channel::Roughness));
}

#[test]
fn an_alpha_from_the_search_becomes_the_channels_source() {
    let (mut harness, layer) = app_with_fill(|_, _| {});
    click_slot(&mut harness, "Roughness");

    // The search field has the focus as the picker opens and searches every Resources group
    type_search(&mut harness, "crack");

    assert!(harness.query_by_role_and_label(Role::ListBoxOption, "Clouds").is_none(), "the search hides what doesn't match");

    click_widget(&mut harness, Role::Button, "Clear search");

    assert!(harness.query_by_role_and_label(Role::ListBoxOption, "Clouds").is_some(), "× clears the search");

    type_search(&mut harness, "crack");

    click_tile(&mut harness, "Cracks");
    let Source::Image { name, .. } = source(&harness, layer, Channel::Roughness) else { panic!("an alpha is read as an image") };
    assert_eq!(texturecraft_engine::fill_alpha::library_alpha_of(&name).map(|alpha| alpha.label.as_str()), Some("Cracks"));
}

#[test]
fn the_picker_opens_on_the_current_sources_tab_scrolled_to_it() {
    let (mut harness, layer) = app_with_fill(|session, layer| {
        let stencil = json!({"layer": layer.0, "channel": "roughness", "alpha": "letter_z"});
        texturecraft_engine::execute(session, "layer.fill.set", &stencil).unwrap();
    });

    assert!(matches!(source(&harness, layer, Channel::Roughness), Source::Image { .. }));

    click_slot(&mut harness, "Roughness");
    harness.run_steps(SCROLL_FRAMES);

    // The last alpha sits far below the picker's first rows; it shows, outlined as the current
    let tile = harness.get_by_role_and_label(Role::ListBoxOption, "Stencil Z");
    assert_eq!(tile.accesskit_node().toggled(), Some(egui::accesskit::Toggled::True), "the current tile is outlined");

    let picker = picker_rect(&harness, "Stencil Z");
    assert!(picker.contains_rect(tile.rect()), "the current tile is scrolled into view: {:?} in {picker:?}", tile.rect());
}

#[test]
fn the_anchor_points_tab_lists_the_stacks_anchors() {
    // An anchor on the first fill, then a second fill above it reads the anchor
    let (mut harness, _) = app_with_fill(|session, layer| {
        texturecraft_engine::execute(session, "effect.add.anchor", &json!({"layer": layer.0, "name": "Edge wear"})).unwrap();
        texturecraft_engine::execute(session, "layer.add.fill", &json!({})).unwrap();
    });
    let reader = harness.state().session.selected_layer().unwrap();
    click_slot(&mut harness, "Roughness");
    click_widget(&mut harness, Role::Tab, "Anchor points");

    assert!(harness.query_by_role_and_label(Role::ListBoxOption, "Clouds").is_none(), "the tab shows anchors only");

    let anchor = harness.get_all_by_role(Role::ListBoxOption).find_map(|node| node.accesskit_node().label().filter(|label| label.contains("Edge wear")));
    click_tile(&mut harness, &anchor.expect("the anchor below is offered"));

    assert!(matches!(source(&harness, reader, Channel::Roughness), Source::Anchor { .. }));
}

#[test]
fn the_anchor_points_tab_lists_mask_anchor_points_by_name() {
    let (mut harness, _) = app_with_fill(|session, layer| {
        texturecraft_engine::execute(session, "effect.add.anchor", &json!({"layer": layer.0, "target": "mask", "name": "Dirt mask"})).unwrap();
        texturecraft_engine::execute(session, "layer.add.fill", &json!({})).unwrap();
    });
    let reader = harness.state().session.selected_layer().unwrap();
    click_slot(&mut harness, "Roughness");
    click_widget(&mut harness, Role::Tab, "Anchor points");
    let anchor = harness.get_all_by_role(Role::ListBoxOption).find_map(|node| node.accesskit_node().label().filter(|label| label.starts_with("Dirt mask")));
    click_tile(&mut harness, &anchor.expect("the mask anchor point below is offered under its own name"));

    assert!(matches!(source(&harness, reader, Channel::Roughness), Source::Anchor { .. }));
}

#[test]
fn escape_or_a_click_outside_closes_it_and_the_wheel_still_zooms_the_view() {
    let (mut harness, layer) = app_with_fill(|_, _| {});
    assert_list_leaves_the_wheel_and_closes(&mut harness, |harness| {
        click_slot(harness, "Roughness");

        // The search takes the focus, which would keep Escape from the list
        harness.ctx.memory_mut(|memory| memory.stop_text_input());
        harness.get_by_role_and_label(Role::ComboBox, "Roughness").rect()
    });

    // Opened again from Uniform after a scroll down, it starts at the top
    click_slot(&mut harness, "Roughness");
    let clouds = |harness: &Harness<'_, TextureCraftApp>| harness.get_by_role_and_label(Role::ListBoxOption, FIRST_TILE).rect();
    let first_tile = clouds(&harness);
    harness.hover_at(first_tile.center());
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: GRID_WHEEL,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(SCROLL_FRAMES);

    assert!(clouds(&harness).top() < first_tile.top(), "the wheel over the picker scrolls its grid");

    click_slot(&mut harness, "Roughness");
    click_slot(&mut harness, "Roughness");

    assert_eq!(clouds(&harness), first_tile, "the first tile shows again");
    assert_eq!(source(&harness, layer, Channel::Roughness), Source::uniform_of(Channel::Roughness), "nothing changed");
}

/// The open picker's popup: the area under its shown tile `label`
fn picker_rect(harness: &Harness<'_, TextureCraftApp>, label: &str) -> egui::Rect {
    let tile = harness.get_by_role_and_label(Role::ListBoxOption, label).rect().center();
    let layer = harness.ctx.layer_id_at(tile).expect("a layer under the tile");
    harness.ctx.memory(|memory| memory.area_rect(layer.id)).expect("the picker's area")
}

#[test]
fn near_the_windows_bottom_the_picker_opens_above_its_slot_at_full_size() {
    let (mut harness, _) = app_with_fill(|_, _| {});
    click_slot(&mut harness, SLOT_LOW_IN_THE_CARD);
    let slot = harness.get_by_role_and_label(Role::ComboBox, SLOT_LOW_IN_THE_CARD).rect();
    let shown = picker_rect(&harness, FIRST_TILE);
    assert!(shown.bottom() <= slot.top(), "with no room under the slot it opens above it, never over it: {shown:?} {slot:?}");
    assert!(harness.ctx.content_rect().contains_rect(shown), "inside the window: {shown:?}");

    click_slot(&mut harness, SLOT_LOW_IN_THE_CARD);

    harness.set_size(TALL_WINDOW_SIZE);
    harness.run_steps(SETTLE_FRAMES);
    click_slot(&mut harness, SLOT_LOW_IN_THE_CARD);
    let slot = harness.get_by_role_and_label(Role::ComboBox, SLOT_LOW_IN_THE_CARD).rect();
    let roomy = picker_rect(&harness, FIRST_TILE);
    assert!(roomy.top() >= slot.bottom(), "a tall window has room under the slot: {roomy:?} {slot:?}");
    assert!((shown.height() - roomy.height()).abs() < 0.5, "above the slot the picker keeps its full height: {shown:?} vs {roomy:?}");
}

#[test]
#[ignore = "requires a GPU adapter; set TC_UI_SNAPSHOTS to save the rendered view"]
fn picker_above_its_slot_snapshot() {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    texturecraft_engine::execute(&mut session, "layer.add.fill", &json!({})).unwrap();
    let mut harness = Harness::builder().with_size(SNAPSHOT_WINDOW_SIZE).with_pixels_per_point(2.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    click_widget(&mut harness, Role::Tab, "Material");
    click_slot(&mut harness, "Roughness");
    harness.hover_at(harness.get_by_role_and_label(Role::ListBoxOption, FIRST_TILE).rect().center());
    harness.run_steps(HOVER_FRAMES);
    let image = harness.render().unwrap();
    if let Some(directory) = std::env::var_os("TC_UI_SNAPSHOTS") {
        image.save(std::path::Path::new(&directory).join("picker-above-slot.png")).unwrap();
    }
}
