//! The color selection in Properties: the ID Mask slot, output value, background, one row per ID color, remove, and the pick mode

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use texturecraft_engine::core::{BakedMap, EffectKind, LayerId, SelectionBackground};
use texturecraft_engine::{StackTarget, ViewMode, execute};
use texturecraft_ui_egui::TextureCraftApp;

use crate::{BAKE_RESOLUTION, PROPERTIES_WINDOW, REDRAW_FRAMES, SETTLE_FRAMES, STARTUP_FRAMES, TEXTURE_RESOLUTION};

const FIRST: [f32; 3] = [200.0 / 255.0, 0.0, 0.0];
const SECOND: [f32; 3] = [10.0 / 255.0, 1.0, 0.0];
const THIRD: [f32; 3] = [30.0 / 255.0, 0.0, 1.0];
const ID_PART: [u8; 3] = [90, 160, 220];

/// Most air between a control and the text of its tooltip above it: the tooltip's gap plus its frame's margin
const TIP_GAP: f32 = 16.0;

/// Frames past the tooltip delay
const TOOLTIP_WAIT_FRAMES: usize = 60;

/// The Colors band's button that starts a pick
const PICK_COLOR: &str = "Pick color";

/// An ID map of one color, as a PNG
fn id_map_png() -> Vec<u8> {
    let [red, green, blue] = ID_PART;
    let image = image::RgbaImage::from_pixel(4, 4, image::Rgba([red, green, blue, 255]));
    let mut bytes = Vec::new();
    image.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    bytes
}

/// A selected color selection of three colors on a fill layer's mask
fn color_selection_session(with_id_map: bool) -> (texturecraft_engine::Session, LayerId) {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    if with_id_map {
        let image = session.import_resource_bytes("id.png", id_map_png()).unwrap();
        execute(&mut session, "textureSet.meshMap.set", &json!({"map": "id", "image": image})).unwrap();
    } else {
        // The sample bakes an ID map with the others
        execute(&mut session, "textureSet.meshMap.clear", &json!({"map": "id"})).unwrap();
    }

    let layer = LayerId(execute(&mut session, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    execute(&mut session, "mask.add.colorSelection", &Value::Null).unwrap();
    let effect = session.doc.project.layer(layer).unwrap().mask.as_ref().unwrap().effects[0].id;
    execute(&mut session, "effect.set", &json!({"effect": effect.0, "colors": [FIRST, SECOND, THIRD]})).unwrap();
    session.target = StackTarget::Effect(effect);
    (session, effect)
}

fn app_with_color_selection(with_id_map: bool) -> (Harness<'static, TextureCraftApp>, LayerId) {
    let (session, effect) = color_selection_session(with_id_map);
    let mut harness = Harness::builder().with_size(PROPERTIES_WINDOW).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(STARTUP_FRAMES);
    (harness, effect)
}

fn colors(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> Vec<[f32; 3]> {
    match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::ColorSelection(selection) => selection.colors.clone(),
        other => panic!("not a color selection: {other:?}"),
    }
}

fn click_at(harness: &mut Harness<'_, TextureCraftApp>, position: Pos2) {
    harness.event(Event::PointerMoved(position));
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: position, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    harness.run_steps(1);
    harness.event(Event::PointerButton { pos: position, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    harness.run_steps(SETTLE_FRAMES);
}

/// Whether the pointer at `pointer` is the drawn eyedropper: the system pointer hidden and a picture painted under it
fn eyedropper_at(harness: &Harness<'_, TextureCraftApp>, pointer: Pos2) -> bool {
    let output = harness.output();
    output.platform_output.cursor_icon == egui::CursorIcon::None
        && output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Mesh(mesh) => {
                harness.ctx.tex_manager().read().meta(mesh.texture_id).is_some_and(|texture| texture.name == "eyedropper_cursor")
                    && mesh.calc_bounds().expand(1.0).contains(pointer)
            }
            _ => false,
        })
}

/// The accessible name of `color`'s row button `action` ("Pick color" on the swatch, "Remove" on the ×)
fn row_label(action: &str, color: [f32; 3]) -> String {
    let [red, green, blue] = color.map(|value| (value * 255.0).round() as u8);
    format!("{action} R {red} G {green} B {blue}")
}

#[test]
fn each_color_gets_a_row_and_remove_sends_the_list_without_it() {
    let (mut harness, effect) = app_with_color_selection(false);
    for color in [FIRST, SECOND, THIRD] {
        assert!(harness.query_by_label(&row_label("Pick color", color)).is_some(), "{color:?} row");
    }

    let remove = row_button(&harness, SECOND, "Remove");
    click_at(&mut harness, remove);

    assert_eq!(colors(&harness, effect), vec![FIRST, THIRD]);
}

#[test]
fn pick_mode_shows_the_id_map_and_a_click_on_the_mesh_adds_its_color() {
    let (mut harness, effect) = app_with_color_selection(true);
    harness.get_by_label(PICK_COLOR).click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_id_color.is_some_and(|pick| pick.effect == effect && pick.index.is_none()));
    assert_eq!(harness.state().session.view, ViewMode::MeshMap(BakedMap::Id));

    let center = harness.state().viewport_rect.center();
    harness.event(Event::PointerMoved(center));
    harness.run_steps(2);

    assert!(eyedropper_at(&harness, center), "the pointer is an eyedropper");

    click_at(&mut harness, center);

    let picked = colors(&harness, effect);
    assert_eq!(picked.len(), 4, "the click adds a row");
    assert_eq!(picked[3], ID_PART.map(|channel| channel as f32 / 255.0));
    assert!(harness.state().picking_id_color.is_none(), "one click ends pick mode");
    assert_eq!(harness.state().session.view, ViewMode::Material, "the earlier view comes back");
}

#[test]
fn without_an_id_map_picking_is_off() {
    let (mut harness, effect) = app_with_color_selection(false);
    harness.get_by_label(PICK_COLOR).click();
    harness.run_steps(SETTLE_FRAMES);
    let repick = row_button(&harness, FIRST, "Pick color");
    click_at(&mut harness, repick);

    assert!(harness.state().picking_id_color.is_none());
    assert_eq!(colors(&harness, effect).len(), 3);
}

/// The centre of `color`'s row button `action`
fn row_button(harness: &Harness<'_, TextureCraftApp>, color: [f32; 3], action: &str) -> Pos2 {
    harness.get_by_label(&row_label(action, color)).rect().center()
}

#[test]
fn removing_a_row_cancels_a_pending_replacement() {
    let (mut harness, effect) = app_with_color_selection(true);
    let repick = row_button(&harness, SECOND, "Pick color");
    click_at(&mut harness, repick);

    assert_eq!(harness.state().picking_id_color.unwrap().index, Some(1));

    let remove = row_button(&harness, FIRST, "Remove");
    click_at(&mut harness, remove);

    assert_eq!(colors(&harness, effect), vec![SECOND, THIRD]);
    assert!(harness.state().picking_id_color.is_none(), "removing a row must not leave a replacement targeting a different color");
    assert_eq!(harness.state().session.view, ViewMode::Material);
}

#[test]
fn escape_cancels_the_pick_without_editing_colors() {
    let (mut harness, effect) = app_with_color_selection(true);
    harness.get_by_label(PICK_COLOR).click();
    harness.run_steps(SETTLE_FRAMES);
    let history = harness.state().session.history_labels();

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_id_color.is_none());
    assert_eq!(harness.state().session.view, ViewMode::Material);
    assert_eq!(colors(&harness, effect), vec![FIRST, SECOND, THIRD]);
    assert_eq!(harness.state().session.history_labels(), history);
}

#[test]
fn escape_cancels_id_picking_with_a_hidden_rail_popup() {
    let (mut harness, effect) = app_with_color_selection(true);
    harness.get_by_label(PICK_COLOR).click();
    harness.run_steps(SETTLE_FRAMES);
    let context = harness.ctx.clone();
    texturecraft_ui_egui::menus::invoke(harness.state_mut(), &context, "ui.view.log");
    harness.run_steps(SETTLE_FRAMES);
    harness.get_by_role_and_label(egui::Role::Button, "Close");
    harness.key_press(egui::Key::Tab);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().ui.hide_ui);
    assert!(harness.query_by_role_and_label(egui::Role::Button, "Close").is_none());
    assert!(harness.state().picking_id_color.is_some());

    let history = harness.state().session.history_labels();

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_id_color.is_none(), "one Escape cancels ID picking despite the hidden popup");
    assert_eq!(harness.state().session.view, ViewMode::Material);
    assert_eq!(colors(&harness, effect), vec![FIRST, SECOND, THIRD]);
    assert_eq!(harness.state().session.history_labels(), history);
}

#[test]
fn picking_from_the_popup_closes_it_before_the_viewport_click() {
    use texturecraft_ui_egui::properties_popup::{self, PropertiesPopup};

    let (mut harness, effect) = app_with_color_selection(true);
    let bounds = harness.state().viewport_rect;
    harness.state_mut().properties_popup = Some(PropertiesPopup::Open { at: bounds.center() - egui::vec2(100.0, 100.0), bounds });
    harness.run_steps(REDRAW_FRAMES);
    let popup = harness.ctx.memory(|memory| memory.area_rect(properties_popup::area_id())).unwrap();
    let pick = harness.query_all_by_label(PICK_COLOR).find(|button| popup.contains_rect(button.rect())).unwrap().rect().center();
    click_at(&mut harness, pick);

    assert!(harness.state().picking_id_color.is_some());

    let center = harness.state().viewport_rect.center();
    assert!(popup.contains(center), "pick a point the popup would otherwise cover");

    click_at(&mut harness, center);

    assert_eq!(colors(&harness, effect).len(), 4, "the next viewport click must pick, even under the former popup");
    assert_eq!(harness.state().properties_popup, None);
}

#[test]
fn a_2d_pick_replaces_the_chosen_row_and_undo_restores_it() {
    use texturecraft_ui_egui::state::ViewLayout;

    let (mut harness, effect) = app_with_color_selection(true);
    harness.state_mut().ui.layout = ViewLayout::Only3D;
    harness.run_steps(SETTLE_FRAMES);
    let center = harness.state().viewport_rect.center();
    harness.state_mut().ui.layout = ViewLayout::Only2D;
    harness.run_steps(SETTLE_FRAMES);
    let repick = row_button(&harness, SECOND, "Pick color");
    click_at(&mut harness, repick);
    click_at(&mut harness, center);

    assert_eq!(colors(&harness, effect), vec![FIRST, ID_PART.map(|channel| channel as f32 / 255.0), THIRD]);
    assert_eq!(harness.state().session.view, ViewMode::Material);
    assert!(!harness.state().session.stroke_active());

    harness.state_mut().session.undo().unwrap();

    assert_eq!(colors(&harness, effect), vec![FIRST, SECOND, THIRD]);
}

#[test]
fn deselection_cancels_picking_and_preserves_a_view_chosen_meanwhile() {
    let (mut harness, effect) = app_with_color_selection(true);
    harness.get_by_label(PICK_COLOR).click();
    harness.run_steps(SETTLE_FRAMES);
    execute(&mut harness.state_mut().session, "viewport.view", &json!({"mode": "uvChecker"})).unwrap();
    harness.state_mut().session.target = StackTarget::Content;
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_id_color.is_none());
    assert_eq!(harness.state().session.view, ViewMode::UvChecker);
    assert_eq!(colors(&harness, effect), vec![FIRST, SECOND, THIRD]);
}

#[test]
fn removing_an_image_during_id_picking_falls_back_to_material() {
    let (mut harness, _) = app_with_color_selection(true);
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(1, 1).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let session = &mut harness.state_mut().session;
    let name = session.import_resource_bytes("preview.png", png.into_inner()).unwrap();
    execute(session, "viewport.view", &json!({"mode": "image", "image": name})).unwrap();
    harness.get_by_label(PICK_COLOR).click();
    harness.run_steps(3);

    assert_eq!(harness.state().session.view, ViewMode::MeshMap(BakedMap::Id));
    assert!(harness.state().picking_id_color.is_some());

    let app = harness.state_mut();
    execute(&mut app.session, "resource.remove", &json!({"name": name})).unwrap();
    texturecraft_ui_egui::id_color_pick::IdColorPick::end(app);

    assert_eq!(app.session.view, ViewMode::Material);
    assert!(app.picking_id_color.is_none());
}

#[test]
#[ignore = "requires a GPU adapter; set TC_UI_SNAPSHOTS to save the rendered views"]
fn color_selection_snapshots_and_quick_mask_pick() {
    let (session, _) = color_selection_session(true);
    let mut harness = Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(1.0).wgpu().build_eframe(|context| {
        let mut app = TextureCraftApp::new(context, session);
        app.ui.layout = texturecraft_ui_egui::state::ViewLayout::Only3D;
        app
    });
    let adapter = harness.state().render_state.as_ref().unwrap().adapter.get_info();
    eprintln!("Offscreen adapter: {adapter:?}");

    let save = |harness: &mut Harness<'_, TextureCraftApp>, name: &str| {
        harness.run_steps(8);
        let image = harness.render().unwrap();
        if let Some(directory) = std::env::var_os("TC_UI_SNAPSHOTS") {
            let path = std::path::Path::new(&directory).join(format!("{name}.png"));
            image.save(&path).unwrap();
            eprintln!("wrote {}", path.display());
        }
        image
    };

    save(&mut harness, "color-selection-editor");
    harness.get_by_label(PICK_COLOR).click();
    let center = harness.state().viewport_rect.center();
    harness.hover_at(center + egui::vec2(80.0, 0.0));
    let expected = save(&mut harness, "color-selection-picking");

    execute(&mut harness.state_mut().session, "viewport.quickMask", &json!({"enabled": true})).unwrap();
    execute(&mut harness.state_mut().session, "viewport.quickMask.edit", &json!({"editing": true})).unwrap();
    let with_quick_mask = save(&mut harness, "color-selection-picking-quick-mask");

    assert_eq!(
        with_quick_mask.get_pixel(center.x as u32, center.y as u32),
        expected.get_pixel(center.x as u32, center.y as u32),
        "picking must display the ID map while quick-mask editing is enabled"
    );

    click_at(&mut harness, center);

    assert!(harness.state().session.quick_mask.editing, "picking preserves quick-mask editing");

    save(&mut harness, "color-selection-quick-mask-restored");

    execute(&mut harness.state_mut().session, "viewport.quickMask.edit", &json!({"editing": false})).unwrap();
    harness.state_mut().ui.layout = texturecraft_ui_egui::state::ViewLayout::Only2D;
    harness.get_by_label(PICK_COLOR).click();
    save(&mut harness, "color-selection-picking-2d");
    click_at(&mut harness, center);

    assert!(harness.state().picking_id_color.is_none());

    execute(&mut harness.state_mut().session, "textureSet.meshMap.clear", &json!({"map": "id"})).unwrap();
    save(&mut harness, "color-selection-no-id-map");
}

fn selection(harness: &Harness<'_, TextureCraftApp>, effect: LayerId) -> texturecraft_engine::core::ColorSelection {
    match &harness.state().session.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::ColorSelection(selection) => selection.clone(),
        other => panic!("not a color selection: {other:?}"),
    }
}

#[test]
fn the_background_list_and_the_output_invert_edit_one_undo_step_each() {
    let (mut harness, effect) = app_with_color_selection(true);
    for slider in ["Output value", "Hardness", "Tolerance"] {
        assert!(harness.query_by_role_and_label(Role::Slider, slider).is_some(), "{slider} row");
    }
    let steps = harness.state().session.history_labels().len();

    harness.get_by_role_and_label(Role::ComboBox, "Black").click();
    harness.run_steps(SETTLE_FRAMES);
    harness.get_by_role_and_label(Role::ListBoxOption, "White").click();
    harness.run_steps(REDRAW_FRAMES);

    assert_eq!(selection(&harness, effect).background, SelectionBackground::White);
    assert_eq!(harness.state().session.history_labels().len(), steps + 1);

    let invert = harness.get_by_role_and_label(Role::Button, "Invert").rect().center();
    click_at(&mut harness, invert);

    assert_eq!(selection(&harness, effect).output, 0.0);
    assert_eq!(harness.state().session.history_labels().len(), steps + 2);
}

#[test]
fn the_background_list_leaves_the_view_its_wheel_and_closes_on_escape_or_a_click_outside() {
    let (mut harness, effect) = app_with_color_selection(true);
    crate::assert_list_leaves_the_wheel_and_closes(&mut harness, |harness| {
        harness.get_by_role_and_label(Role::ComboBox, "Black").click();
        harness.run_steps(SETTLE_FRAMES);
        harness.get_by_role_and_label(Role::ComboBox, "Black").rect()
    });

    assert_eq!(selection(&harness, effect).background, SelectionBackground::Black, "nothing changed");
}

#[test]
fn the_id_mask_slot_picks_a_texture_and_its_x_goes_back_to_the_mesh_map() {
    let (mut session, effect) = color_selection_session(true);
    let texture = session.import_resource_bytes("parts.png", id_map_png()).unwrap();
    let mut harness = Harness::builder().with_size(PROPERTIES_WINDOW).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(STARTUP_FRAMES);

    harness.get_by_role_and_label(Role::ComboBox, "ID Mask").click();
    harness.run_steps(SETTLE_FRAMES);
    harness.get_by_role_and_label(Role::ListBoxOption, &texture).click();
    harness.run_steps(REDRAW_FRAMES);

    assert_eq!(selection(&harness, effect).id_map.as_deref(), Some(texture.as_str()));

    let clear = harness.get_by_role_and_label(Role::Button, "Clear").rect().center();
    click_at(&mut harness, clear);

    assert_eq!(selection(&harness, effect).id_map, None);
    assert!(harness.query_by_role_and_label(Role::Button, "Clear").is_none(), "the mesh map has no ×");
}

/// Whether the ID Mask picker's tile `label` shows as the current one (egui reports a selected tile as toggled)
fn tile_is_current(harness: &Harness<'_, TextureCraftApp>, label: &str) -> bool {
    harness.get_by_role_and_label(Role::ListBoxOption, label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn reading_the_mesh_map_shows_no_tile_as_the_current_id_mask() {
    let (mut session, effect) = color_selection_session(true);
    let texture = session.import_resource_bytes("parts.png", id_map_png()).unwrap();
    let mut harness = Harness::builder().with_size(PROPERTIES_WINDOW).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(STARTUP_FRAMES);

    assert_eq!(selection(&harness, effect).id_map, None, "the selection reads the ID mesh map");

    harness.get_by_role_and_label(Role::ComboBox, "ID Mask").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(!tile_is_current(&harness, "Import..."), "Import... is an action, not the map read");
    assert!(!tile_is_current(&harness, &texture));

    harness.get_by_role_and_label(Role::ListBoxOption, &texture).click();
    harness.run_steps(REDRAW_FRAMES);
    harness.get_by_role_and_label(Role::ComboBox, "ID Mask").click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(tile_is_current(&harness, &texture), "a chosen texture is the current tile");
    assert!(!tile_is_current(&harness, "Import..."));
}

#[test]
fn the_invert_tooltip_shows_right_above_its_button() {
    let (mut harness, _) = app_with_color_selection(true);
    let invert = harness.get_by_role_and_label(Role::Button, "Invert").rect();
    harness.hover_at(invert.center());

    harness.run_steps(TOOLTIP_WAIT_FRAMES);
    let tip =
        harness.get_all_by_label("Invert").find(|node| node.accesskit_node().role() == Role::Label).map(|node| node.rect()).expect("the button's tooltip");
    assert!(tip.bottom() <= invert.top() && invert.top() - tip.bottom() <= TIP_GAP, "the tooltip sits right above the button: {tip:?} {invert:?}");
    assert!(tip.right() >= invert.left() && tip.left() <= invert.right(), "the tooltip overlaps the button's columns, not off to its side: {tip:?} {invert:?}");
}

#[test]
#[ignore = "requires a GPU adapter; set TC_UI_SNAPSHOTS to save the rendered view"]
fn invert_tooltip_snapshot() {
    let (session, _) = color_selection_session(true);
    let mut harness =
        Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_pixels_per_point(2.0).wgpu().build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(STARTUP_FRAMES);
    let invert = harness.get_by_role_and_label(Role::Button, "Invert").rect();
    harness.hover_at(invert.center());
    harness.run_steps(TOOLTIP_WAIT_FRAMES);
    let image = harness.render().unwrap();
    if let Some(directory) = std::env::var_os("TC_UI_SNAPSHOTS") {
        image.save(std::path::Path::new(&directory).join("invert-tooltip.png")).unwrap();
    }
}

#[test]
fn a_hand_picked_id_mask_picks_without_a_bake() {
    let (mut session, effect) = color_selection_session(false);
    session.doc.baked.clear();
    let texture = session.import_resource_bytes("parts.png", id_map_png()).unwrap();
    execute(&mut session, "effect.set", &json!({"effect": effect.0, "idMap": texture})).unwrap();
    let mut harness = Harness::builder().with_size(PROPERTIES_WINDOW).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(STARTUP_FRAMES);

    harness.get_by_label(PICK_COLOR).click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_id_color.is_some_and(|pick| pick.effect == effect), "Pick color starts picking with no bake");

    let center = harness.state().viewport_rect.center();
    click_at(&mut harness, center);

    let picked = colors(&harness, effect);
    assert_eq!(picked.len(), 4, "the click adds a row");
    assert_eq!(picked[3], ID_PART.map(|channel| channel as f32 / 255.0), "read from the chosen image");
}

#[test]
fn clearing_the_only_id_mask_ends_a_waiting_pick() {
    let (mut session, effect) = color_selection_session(false);
    session.doc.baked.clear();
    let texture = session.import_resource_bytes("parts.png", id_map_png()).unwrap();
    execute(&mut session, "effect.set", &json!({"effect": effect.0, "idMap": texture})).unwrap();
    let mut harness = Harness::builder().with_size(PROPERTIES_WINDOW).build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.run_steps(STARTUP_FRAMES);
    harness.get_by_label(PICK_COLOR).click();
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_id_color.is_some(), "the pick waits");

    let clear = harness.get_by_role_and_label(Role::Button, "Clear").rect().center();
    click_at(&mut harness, clear);

    assert_eq!(selection(&harness, effect).id_map, None);
    assert!(harness.state().picking_id_color.is_none(), "with no ID map left the pick ends");
    assert_eq!(harness.state().session.view, ViewMode::Material, "and the earlier view comes back");
}
