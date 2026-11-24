//! A fill's Material section: the channel strip turns channels on and off, each slot's row lists the sources, ⟲ and × restore and clear, the ramp inverts, a linked material maps its channels and picks its resolution

use std::sync::Arc;

use egui::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::core::{Channel, CookResolution, LayerId, LayerKind, LinkParam, MaterialLink, Procedural, ProceduralKind, Source};
use texturecraft_ui_egui::TextureCraftApp;
use texturecraft_ui_egui::screen_pick::{PickOutcome, ScreenPick};

use crate::fill_support::{click_node, fill_app, fill_of};
use crate::filter_support::{along, click_at, drag, undo_depth};
use crate::{SETTLE_FRAMES, TEXTURE_RESOLUTION};

/// Tall enough that Properties shows the whole fill and an open source list without scrolling
const WINDOW_SIZE: egui::Vec2 = egui::vec2(1600.0, 1600.0);

/// Frames for the shelf's and the picker's thumbnails to render a few per frame, enough for every pattern were all of them asked for
const THUMBNAIL_FRAMES: usize = 200;

/// Frames for a scroll to bring a control into view
const SCROLL_FRAMES: usize = 25;

/// The app with a new fill layer (`settings` given to layer.add.fill) selected and its Material tab clicked, and the layer's id
fn app_with_fill(settings: serde_json::Value) -> (Harness<'static, TextureCraftApp>, LayerId) {
    fill_app(settings, None, Some("Material"), WINDOW_SIZE)
}

/// The source `channel` of the fill `layer` reads
fn source(harness: &Harness<'_, TextureCraftApp>, layer: LayerId, channel: Channel) -> Source {
    fill_of(harness, layer).source(channel)
}

fn click(harness: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    // A link without a package fails to re-cook with a toast, which must not take the click
    harness.state_mut().toasts.clear();
    click_node(harness, role, label);
}

/// Scrolls the `role` named `label` into view (the scroll animates for about twenty frames), then clicks it
fn click_below(harness: &mut Harness<'_, TextureCraftApp>, role: Role, label: &str) {
    harness.get_by_role_and_label(role, label).scroll_to_me();
    harness.run_steps(SCROLL_FRAMES);
    click(harness, role, label);
}

/// Clicks the eyedropper at the end of the Base color bar's row in the panel (the open picker has its own)
fn click_bar_eyedropper(harness: &mut Harness<'_, TextureCraftApp>) {
    let bar = harness.get_by_role_and_label(Role::Button, "Base color").rect();
    let eyedropper = harness
        .get_all_by_role_and_label(Role::Button, "Pick color")
        .find(|node| bar.y_range().contains(node.rect().center().y) && node.rect().left() >= bar.right())
        .unwrap()
        .rect();
    click_at(harness, eyedropper.center());
}

/// Clicks the open picker's own eyedropper, beside its hex field
fn click_picker_eyedropper(harness: &mut Harness<'_, TextureCraftApp>) {
    let hex = harness.get_by_role_and_label(Role::TextInput, "Hex color").rect();
    let title = harness.get_by_role_and_label(Role::TitleBar, "Base color").rect();
    let in_picker_row = |at: egui::Pos2| hex.y_range().contains(at.y) && at.x >= hex.right() && at.x <= title.right() + CLOSE_ROOM;
    let eyedropper = harness.get_all_by_role_and_label(Role::Button, "Pick color").map(|node| node.rect().center()).find(|at| in_picker_row(*at)).unwrap();
    click_at(harness, eyedropper);
}

/// Clicks the `role` named `label` that sits in the row of the slot named `slot` (bands above have their own ⟲)
fn click_in_slot(harness: &mut Harness<'_, TextureCraftApp>, slot: &str, role: Role, label: &str) {
    let row = harness.get_by_role_and_label(Role::ComboBox, slot).rect();
    let at = harness.get_all_by_role_and_label(role, label).map(|node| node.rect().center()).find(|at| row.contains(*at)).unwrap();
    click_at(harness, at);
}

#[test]
fn the_channel_strip_turns_a_channel_and_its_card_off_and_on() {
    let (mut harness, layer) = app_with_fill(json!({}));
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Metallic").is_some(), "Metallic is on: its card shows");

    click(&mut harness, Role::CheckBox, "Metal");
    let channels = &harness.state().session.doc.project.layer(layer).unwrap().channels;

    assert!(!channels.contains_key(&Channel::Metallic), "off in the strip, the fill stops writing Metallic");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Metallic").is_none(), "a channel that is off has no card");

    click(&mut harness, Role::CheckBox, "Metal");

    assert!(harness.state().session.doc.project.layer(layer).unwrap().channels.contains_key(&Channel::Metallic));
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Metallic").is_some());
}

#[test]
fn a_slot_lists_its_sources_restores_a_procedural_and_clears_back_to_uniform() {
    let (mut harness, layer) = app_with_fill(json!({}));
    click(&mut harness, Role::ComboBox, "Metallic");
    click(&mut harness, Role::ListBoxOption, ProceduralKind::Clouds.label());
    let Source::Procedural { pattern, .. } = source(&harness, layer, Channel::Metallic) else { panic!("a procedural") };
    assert_eq!(pattern.kind, ProceduralKind::Clouds);
    assert!(harness.query_by_role_and_label(Role::Slider, "Contrast").is_some(), "a procedural shows its Scale and Contrast");

    texturecraft_engine::execute(
        &mut harness.state_mut().session,
        "layer.fill.set",
        &json!({"layer": layer.0, "channel": "metallic", "procedural": "Clouds", "scale": 9.0, "seed": 7}),
    )
    .unwrap();
    harness.run_steps(SETTLE_FRAMES);
    click_in_slot(&mut harness, "Metallic", Role::Button, "Restore defaults");
    let Source::Procedural { pattern, .. } = source(&harness, layer, Channel::Metallic) else { panic!("still a procedural") };
    assert_eq!(pattern, Procedural::new(ProceduralKind::Clouds), "⟲ keeps the pattern and restores its settings");

    click_in_slot(&mut harness, "Metallic", Role::Button, "Clear");

    assert_eq!(source(&harness, layer, Channel::Metallic), Source::uniform_of(Channel::Metallic), "× goes back to a uniform value");
}

/// A pattern's thumbnail is made only once its tile is drawn: a search renders the patterns it shows, not every pattern
#[test]
fn searching_the_source_picker_renders_only_the_shown_pattern_thumbnails() {
    let (mut harness, layer) = app_with_fill(json!({}));
    let pattern_thumbnails = |harness: &Harness<'_, TextureCraftApp>| {
        harness.ctx.tex_manager().read().allocated().filter(|(_, texture)| texture.name.starts_with("thumb_proc:")).count()
    };
    click(&mut harness, Role::ComboBox, "Metallic");
    harness.event(egui::Event::Text("Smooth fur".into()));
    harness.run_steps(THUMBNAIL_FRAMES);

    let after_search = pattern_thumbnails(&harness);
    let rendered = |name: &str| harness.ctx.tex_manager().read().allocated().any(|(_, texture)| texture.name == name);
    assert!(rendered("thumb_proc:SmoothFur"), "the pattern the search shows gets its thumbnail");
    assert!(after_search < ProceduralKind::ALL.len(), "{after_search} of {} pattern thumbnails made", ProceduralKind::ALL.len());

    click(&mut harness, Role::ListBoxOption, ProceduralKind::SmoothFur.label());
    let Source::Procedural { pattern, .. } = source(&harness, layer, Channel::Metallic) else { panic!("a procedural") };
    assert_eq!(pattern.kind, ProceduralKind::SmoothFur);
    assert_eq!(pattern_thumbnails(&harness), after_search, "the pick reuses the thumbnail it showed");

    let context = harness.ctx.clone();
    drop(harness);
    let left = context.tex_manager().read().allocated().filter(|(_, texture)| texture.name.starts_with("thumb_proc:")).count();
    assert_eq!(left, 0, "closing the app releases the pattern thumbnails");
}

#[test]
fn searching_the_source_picker_does_not_render_hidden_alpha_thumbnails() {
    let (mut harness, _) = app_with_fill(json!({}));
    let alpha_thumbnails = |harness: &Harness<'_, TextureCraftApp>| {
        let texture_manager = harness.ctx.tex_manager();
        let textures = texture_manager.read();
        textures
            .allocated()
            .filter(|(_, texture)| texture.name.starts_with("thumb_alpha:"))
            .fold((0, 0), |(count, bytes), (_, texture)| (count + 1, bytes + texture.bytes_used()))
    };
    let before = alpha_thumbnails(&harness);

    click(&mut harness, Role::ComboBox, "Metallic");
    harness.event(egui::Event::Text("Smooth fur".into()));
    harness.run_steps(THUMBNAIL_FRAMES);

    let after = alpha_thumbnails(&harness);
    eprintln!("alpha thumbnails and pixel bytes before picker {before:?}, after search {after:?}");
    assert!(harness.query_by_role_and_label(Role::ListBoxOption, "Smooth fur").is_some());
    assert_eq!(after, before, "only a matching, visible tile should request its preview");

    let alpha = &texturecraft_engine::paint::alphas::library()[0];
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(alpha.label.clone()));
    harness.run_steps(SETTLE_FRAMES);
    assert!(harness.query_by_role_and_label(Role::ListBoxOption, &alpha.label).is_some());
    assert!(alpha_thumbnails(&harness).0 > after.0, "a visible alpha still gets its preview");
}

#[test]
fn closing_the_project_stops_its_eyedropper() {
    let (mut harness, _) = app_with_fill(json!({}));
    click_bar_eyedropper(&mut harness);
    let bar = harness.state().picking_color.expect("the eyedropper is armed");
    harness.state_mut().screen_pick = Some(ScreenPick::waiting(bar));

    texturecraft_engine::execute(&mut harness.state_mut().session, "file.close", &json!({})).unwrap();
    harness.run_steps(SETTLE_FRAMES);

    assert!(!harness.state().session.project_open);
    assert!(harness.state().picking_color.is_none(), "a closed project cannot consume the next viewport click");
    assert!(harness.state().screen_pick.is_none(), "the native pick must be dropped");
}

/// A project image's thumbnail is made once its tile scrolls into view, not when the picker opens, and goes with the image
#[test]
fn source_picker_images_render_on_scroll_and_release_on_removal() {
    const IMAGE_COUNT: usize = 128;
    const IMAGE_SIDE: u32 = 64;
    let (mut harness, layer) = app_with_fill(json!({}));
    let session = &mut harness.state_mut().session;
    for index in 0..IMAGE_COUNT {
        let name = format!("picker-image-{index:03}.png");
        let texels = (IMAGE_SIDE * IMAGE_SIDE) as usize;
        session.doc.resources.insert(name.clone(), Arc::new(vec![0]));
        session.images.insert(name, Arc::new(ImageRes::new(IMAGE_SIDE, IMAGE_SIDE, vec![[0.5; 4]; texels], false)));
    }
    let image_thumbnails = |harness: &Harness<'_, TextureCraftApp>| {
        harness.ctx.tex_manager().read().allocated().filter(|(_, texture)| texture.name.starts_with("res_picker-image-")).count()
    };

    click(&mut harness, Role::ComboBox, "Metallic");
    harness.run_steps(THUMBNAIL_FRAMES);
    let opened = image_thumbnails(&harness);
    assert!(opened < IMAGE_COUNT, "opening the picker makes thumbnails only for the tiles in view: {opened}");

    let last = format!("picker-image-{:03}.png", IMAGE_COUNT - 1);
    harness.get_by_role_and_label(Role::ListBoxOption, &last).scroll_to_me();
    harness.run_steps(THUMBNAIL_FRAMES);
    let scrolled = image_thumbnails(&harness);
    assert!(scrolled > opened, "a scroll makes the thumbnails of the tiles it brings into view: {opened} -> {scrolled}");

    click(&mut harness, Role::ListBoxOption, &last);

    assert_eq!(source(&harness, layer, Channel::Metallic), Source::image(last));

    let app = harness.state_mut();
    app.session.images.clear();
    app.thumbs.sync_images(&app.session.images, app.session.revision + 1);

    assert_eq!(image_thumbnails(&harness), 0, "removed images release their picker thumbnails");
}

/// Dragging the pattern's Contrast changes only the contrast: the pattern keeps its balance, invert, seed, scale and values
#[test]
fn dragging_a_pattern_slider_changes_only_its_own_parameter() {
    let (mut harness, layer) = app_with_fill(json!({}));
    let pattern = json!({
        "layer": layer.0, "channel": "metallic", "procedural": "Clouds", "scale": 9.0, "contrast": 1.0, "seed": 7,
        "balance": 0.25, "invert": true, "low": [0.2, 0.2, 0.2], "high": [0.8, 0.8, 0.8],
    });
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.fill.set", &pattern).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    let Source::Procedural { pattern: before, low, high, .. } = source(&harness, layer, Channel::Metallic) else { panic!("a procedural") };

    harness.get_by_role_and_label(Role::Slider, "Contrast").scroll_to_me();
    harness.run_steps(SCROLL_FRAMES);
    let track = harness.get_by_role_and_label(Role::Slider, "Contrast").rect();
    let from = track.center();
    drag(&mut harness, from, from + egui::vec2(track.width() / 4.0, 0.0));

    let Source::Procedural { pattern: after, low: after_low, high: after_high, .. } = source(&harness, layer, Channel::Metallic) else {
        panic!("still a procedural")
    };

    assert_ne!(after.contrast, before.contrast, "the drag changes the contrast");
    assert_eq!(Procedural { contrast: before.contrast, ..after }, before, "the rest of the pattern stays");
    assert_eq!((after_low, after_high), (low, high), "the pattern's values stay");
}

#[test]
fn a_uniform_gray_ramp_inverts_its_value() {
    let (mut harness, layer) = app_with_fill(json!({"roughness": 0.3}));
    texturecraft_engine::execute(&mut harness.state_mut().session, "layer.fill.set", &json!({"layer": layer.0, "channel": "roughness", "solo": true})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    click(&mut harness, Role::Button, "Invert");
    let Source::Uniform(value) = source(&harness, layer, Channel::Roughness) else { panic!("uniform") };
    assert!((value[0] - 0.7).abs() < 1e-5, "1 - 0.3: {value:?}");
}

#[test]
fn a_linked_material_shows_its_inputs_and_the_switch_shows_its_channels() {
    let (mut harness, layer) = app_with_fill(json!({}));
    assert!(harness.query_by_role_and_label(Role::RadioButton, "Material").is_some(), "every fill has the switch");

    let wear = float_param("wear", "Wear", 0.5);
    link_fill(&mut harness, layer, vec![wear], &[]);

    assert!(harness.query_by_role_and_label(Role::ComboBox, "Painted steel").is_some(), "the Material side shows the whole material");
    assert!(harness.query_by_role_and_label(Role::Slider, "Wear").is_some(), "with its inputs");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Metallic").is_none(), "and no channel cards");

    click(&mut harness, Role::RadioButton, "Channels");

    assert!(harness.query_by_role_and_label(Role::ComboBox, "Metallic").is_some(), "the Channels side shows a card per channel");
    assert!(harness.query_by_role_and_label(Role::Slider, "Wear").is_none());
}

/// A drag on a linked material's number input is one undo step, however many frames it spans
#[test]
fn dragging_a_linked_material_input_adds_one_undo_step() {
    let (mut harness, layer) = app_with_fill(json!({}));
    let wear = float_param("wear", "Wear", 0.25);
    link_fill(&mut harness, layer, vec![wear], &[]);
    let steps = undo_depth(&harness);

    let track = harness.get_by_role_and_label(Role::Slider, "Wear").rect();
    let from = track.center();
    drag(&mut harness, from, from + egui::vec2(track.width() / 4.0, 0.0));

    let fill = fill_of(&harness, layer);
    assert_ne!(fill.link.as_deref().unwrap().params[0].value, json!({"t": "float", "v": 0.25}), "the drag changes the input");
    assert_eq!(undo_depth(&harness), steps + 1, "every frame of the drag merges into one step");
}

/// A click inside a linked whole-number input's current band is no edit (no undo step, no re-cook); a drag across whole numbers is one step
#[test]
fn a_linked_whole_number_input_changes_only_on_a_new_whole_number() {
    let (mut harness, layer) = app_with_fill(json!({}));
    let rings = LinkParam { value: json!({"t": "int", "v": 3}), default: json!({"t": "int", "v": 3}), max: 10.0, ..float_param("rings", "Rings", 0.0) };
    link_fill(&mut harness, layer, vec![rings], &[]);
    harness.state_mut().toasts.clear();
    let steps = undo_depth(&harness);
    let track = harness.get_by_role_and_label(Role::Slider, "Rings").rect();

    // A link without a package fails to re-cook with a toast: no toast, no cook
    click_at(&mut harness, along(track, 0.32));
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(undo_depth(&harness), steps, "3.2 rounds to the 3 the input holds: no undo step");
    assert!(harness.state().toasts.is_empty(), "and no re-cook");

    drag(&mut harness, along(track, 0.3), along(track, 0.53));
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(fill_of(&harness, layer).link.unwrap().params[0].value, json!({"t": "int", "v": 5}), "the drag ends on 5");
    assert_eq!(undo_depth(&harness), steps + 1, "the drag's whole numbers merge into one step");
}

/// A drag in a linked material's colour picker is one undo step, however many frames it spans
#[test]
fn dragging_in_a_linked_colour_picker_adds_one_undo_step() {
    let (mut harness, layer) = app_with_fill(json!({}));
    let gray = json!({"t": "color", "v": [0.5, 0.5, 0.5, 1.0]});
    let tint = LinkParam { value: gray.clone(), default: gray.clone(), ..float_param("tint", "Tint", 0.0) };
    link_fill(&mut harness, layer, vec![tint], &[]);
    let steps = undo_depth(&harness);

    // The swatch sits right of its label; its picker's saturation and value square lies under the R, G, B row
    let label = harness.get_by_label("Tint").rect();
    let swatch = egui::pos2(label.right() + SWATCH_REACH, label.center().y);
    harness.drag_at(swatch);
    harness.drop_at(swatch);
    harness.run_steps(SETTLE_FRAMES);
    let copy = harness.get_by_role_and_label(Role::Button, "Copy RGB values").rect();
    let red = harness
        .get_all_by_role(Role::SpinButton)
        .map(|node| node.rect())
        .filter(|field| field.center().y < copy.bottom())
        .min_by(|a, b| a.left().total_cmp(&b.left()))
        .unwrap();
    let from = egui::pos2(red.left() + SQUARE_REACH, copy.bottom() + SQUARE_REACH);
    drag(&mut harness, from, from + egui::vec2(SQUARE_REACH / 2.0, SQUARE_REACH / 2.0));

    let fill = fill_of(&harness, layer);
    assert_ne!(fill.link.as_deref().unwrap().params[0].value, gray, "the drag changes the colour");
    assert_eq!(undo_depth(&harness), steps + 1, "every frame of the drag merges into one step");
}

/// From a label's right edge into the colour swatch beside it
const SWATCH_REACH: f32 = 30.0;

/// From the picker's top-left content corner into its saturation and value square, and half of it for the drag
const SQUARE_REACH: f32 = 80.0;

#[test]
fn a_linked_material_maps_its_channels_and_picks_its_resolution() {
    let (mut harness, layer) = app_with_fill(json!({}));
    link_fill(&mut harness, layer, Vec::new(), &[("baseColor", "steel_base.png"), ("roughness", "steel_rough.png")]);
    let link = |harness: &Harness<'_, TextureCraftApp>| *fill_of(harness, layer).link.unwrap();

    // Channels mapping: the Roughness row lists the outputs; picking one makes Roughness read it
    click_below(&mut harness, Role::ComboBox, "roughness");
    click(&mut harness, Role::ListBoxOption, "baseColor");

    assert_eq!(link(&harness).output_of(Channel::Roughness), "baseColor");
    assert!(matches!(source(&harness, layer, Channel::Roughness), Source::Image { name, .. } if name == "steel_base.png"));

    // Resolution: folded until clicked; Fixed keeps the size, the list picks another, Relative goes back to a factor
    assert!(harness.query_by_role_and_label(Role::RadioButton, "Fixed").is_none(), "Resolution starts folded");

    click_below(&mut harness, Role::Button, "Resolution");
    click_below(&mut harness, Role::RadioButton, "Fixed");

    assert_eq!(link(&harness).resolution, Some(CookResolution::Fixed(TEXTURE_RESOLUTION)));

    click_below(&mut harness, Role::ComboBox, "256 × 256");
    click(&mut harness, Role::ListBoxOption, "1024 × 1024");

    assert_eq!(link(&harness).resolution, Some(CookResolution::Fixed(1024)));

    click_below(&mut harness, Role::RadioButton, "Relative");

    assert_eq!(link(&harness).resolution, Some(CookResolution::Relative(2)), "1024 is the texture set's ×4");
}

/// Links the fill `layer` to a material with `params` and image `outputs` (output, image), as a cooked link leaves it
fn link_fill(harness: &mut Harness<'_, TextureCraftApp>, layer: LayerId, params: Vec<LinkParam>, outputs: &[(&str, &str)]) {
    let link = MaterialLink {
        name: "Painted steel".into(),
        package: String::new(),
        graph: "main".into(),
        params,
        presets: Vec::new(),
        inputs: Vec::new(),
        size: 256,
        resolution: None,
        mapping: Default::default(),
        outputs: outputs.iter().map(|(output, image)| ((*output).to_owned(), (*image).to_owned())).collect(),
    };
    if let Some(LayerKind::Fill(fill)) = harness.state_mut().session.doc.project.layers.get_mut(&layer).map(|layer| &mut layer.kind) {
        fill.link = Some(Box::new(link));
    }
    harness.run_steps(SETTLE_FRAMES);
}

/// A 0..1 number input `id` shown as `label`, at `value` and by default
fn float_param(id: &str, label: &str, value: f64) -> LinkParam {
    let value = json!({"t": "float", "v": value});
    LinkParam { id: id.into(), label: label.into(), group: String::new(), value: value.clone(), default: value, min: 0.0, max: 1.0, options: Vec::new() }
}

/// Types the sRGB code `hex` into the open picker's Hex field and confirms it
fn type_hex(harness: &mut Harness<'_, TextureCraftApp>, hex: &str) {
    click(harness, Role::TextInput, "Hex color");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(hex.into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(SETTLE_FRAMES);
}

/// The fill's uniform base colour (linear)
fn base_color(harness: &Harness<'_, TextureCraftApp>, layer: LayerId) -> [f32; 3] {
    let Source::Uniform(color) = source(harness, layer, Channel::BaseColor) else { panic!("a uniform colour") };
    color
}

#[test]
fn the_colour_bar_opens_the_picker_which_types_restores_and_waits_for_an_eyedropper_click() {
    let (mut harness, layer) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    let before = base_color(&harness, layer);
    click(&mut harness, Role::Button, "Base color");

    assert!(harness.query_by_role_and_label(Role::ComboBox, "sRGB").is_some(), "the picker shows its colour space");

    type_hex(&mut harness, "FF8000");
    let typed = base_color(&harness, layer);
    let expected = [1.0, texturecraft_engine::texel::srgb_to_linear(128.0 / 255.0), 0.0];
    assert!(typed.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-3), "the typed sRGB code is stored linear: {typed:?}");

    click(&mut harness, Role::ComboBox, "HSV");
    click(&mut harness, Role::ListBoxOption, "RGB");

    assert!(harness.query_by_role_and_label(Role::Slider, "R").is_some(), "the sliders edit R, G and B");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "sRGB").is_some(), "picking from its own list keeps the picker open");

    click(&mut harness, Role::Button, "Previous color");
    let restored = base_color(&harness, layer);
    assert!(restored.iter().zip(before).all(|(a, b)| (a - b).abs() < 1e-4), "a click on the previous colour restores it: {restored:?} {before:?}");

    click_picker_eyedropper(&mut harness);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_color.is_some(), "the eyedropper waits for a click on the model");
}

#[test]
fn the_picker_is_a_window_beside_the_panel_that_its_title_drags_and_its_close_or_escape_closes() {
    let (mut harness, _) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    click(&mut harness, Role::Button, "Base color");
    let bar = harness.get_by_role_and_label(Role::Button, "Base color").rect();
    let title = harness.get_by_role_and_label(Role::TitleBar, "Base color").rect();
    let space = harness.get_by_role_and_label(Role::ComboBox, "sRGB").rect();
    assert!(title.right() <= bar.left(), "the window stands beside the panel, off its rows: {title:?} {bar:?}");
    assert!(title.top() <= bar.top() + 1.0, "level with the bar, or raised to stay on screen: {title:?} {bar:?}");
    assert!(space.width() < title.width() / 3.0, "the colour space dropdown is as wide as its text: {space:?}");

    let from = title.left_center() + egui::vec2(20.0, 0.0);
    let by = egui::vec2(-120.0, 90.0);
    harness.hover_at(from);
    harness.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    harness.run_steps(1);
    for step in 1..=4 {
        harness.hover_at(from + by * (step as f32 / 4.0));
        harness.run_steps(1);
    }
    harness.event(egui::Event::PointerButton { pos: from + by, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.run_steps(SETTLE_FRAMES);
    let moved = harness.get_by_role_and_label(Role::TitleBar, "Base color").rect();
    assert!((moved.min - (title.min + by)).length() < 1.0, "the title bar drags the window: {moved:?} from {title:?}");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "sRGB").is_some(), "a drag on the title keeps it open");

    let close = harness
        .get_all_by_role_and_label(Role::Button, "Close")
        .find(|node| moved.y_range().contains(node.rect().center().y) && node.rect().left() >= moved.right())
        .unwrap()
        .rect();
    click_at(&mut harness, close.center());

    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_none(), "× closes it");

    click(&mut harness, Role::Button, "Base color");
    let reopened = harness.get_by_role_and_label(Role::TitleBar, "Base color").rect();
    assert!((reopened.min - moved.min).length() < 1.0, "it opens again where it was dragged: {reopened:?}");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_none(), "Escape closes it");
}

#[test]
fn escape_ends_an_eyedropper_pick_before_the_picker_and_an_alt_orbit_keeps_the_picker_open() {
    let (mut harness, _) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    click(&mut harness, Role::Button, "Base color");
    click_bar_eyedropper(&mut harness);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_color.is_some(), "the eyedropper waits for a click on the model");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_color.is_none(), "Escape ends the pick");
    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_some(), "the same Escape leaves the picker open");

    let view = harness.state().viewport_rect.center();
    let alt = egui::Modifiers::ALT;
    harness.event(egui::Event::ModifiersChanged(alt));
    harness.hover_at(view);
    harness.event(egui::Event::PointerButton { pos: view, button: egui::PointerButton::Primary, pressed: true, modifiers: alt });
    harness.run_steps(1);
    harness.hover_at(view + egui::vec2(40.0, 10.0));
    harness.run_steps(1);
    harness.event(egui::Event::PointerButton { pos: view + egui::vec2(40.0, 10.0), button: egui::PointerButton::Primary, pressed: false, modifiers: alt });
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_some(), "an Alt+drag on the model orbits and keeps the picker open");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_none(), "the next Escape closes the picker");
}

/// A secondary-button press and release at `pos`, a frame apart, as a real right-click arrives
fn right_click(harness: &mut Harness<'_, TextureCraftApp>, pos: egui::Pos2) {
    let button = egui::PointerButton::Secondary;
    harness.hover_at(pos);
    harness.run_steps(1);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE });
        harness.run_steps(1);
    }
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn the_swatches_row_keeps_the_picker_colour_in_the_project_applies_it_on_a_click_and_removes_it_from_its_menu() {
    let (mut harness, layer) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    let before = base_color(&harness, layer);
    click(&mut harness, Role::Button, "Base color");
    type_hex(&mut harness, "FF8000");
    let orange = base_color(&harness, layer);

    let add = harness.get_by_role_and_label(Role::Button, "Add swatch").rect();
    let space = harness.get_by_role_and_label(Role::ComboBox, "sRGB").rect();
    assert!(add.top() > space.bottom() && (add.left() - space.left()).abs() < 1.0, "the row is under the picker, at its left edge: {add:?}");

    click_at(&mut harness, add.center());

    assert_eq!(harness.state().session.doc.project.swatches, vec![orange], "+ keeps the picker's colour in the project");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "sRGB").is_some(), "the picker stays open");

    let depth = undo_depth(&harness);
    let kept = harness.get_by_role_and_label(Role::Button, "#FF8000").rect();
    click_at(&mut harness, kept.center());

    assert_eq!(undo_depth(&harness), depth, "a swatch the picker already shows changes nothing");

    click(&mut harness, Role::Button, "Previous color");

    assert_eq!(base_color(&harness, layer), before);

    let chip = harness.get_by_role_and_label(Role::Button, "#FF8000").rect();
    let moved_add = harness.get_by_role_and_label(Role::Button, "Add swatch").rect();
    assert!((chip.left() - add.left()).abs() < 1.0 && moved_add.left() > chip.right(), "the swatch takes the + chip's place, + moves right");

    click_at(&mut harness, chip.center());

    assert_eq!(base_color(&harness, layer), orange, "a click on a swatch applies it");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "sRGB").is_some(), "applying keeps the picker open");

    right_click(&mut harness, chip.center());
    let remove = harness.get_by_role_and_label(Role::Button, "Remove swatch").rect();
    click_at(&mut harness, remove.center());

    assert!(harness.state().session.doc.project.swatches.is_empty(), "its right-click menu removes it");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "sRGB").is_some(), "a pick in the menu keeps the picker open");
}

#[test]
fn a_hex_code_left_as_it_opened_is_no_edit() {
    let (mut harness, layer) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    click(&mut harness, Role::Button, "Base color");
    let depth = undo_depth(&harness);
    click(&mut harness, Role::TextInput, "Hex color");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(base_color(&harness, layer), [0.2, 0.2, 0.2], "the colour is not rounded to the code it shows");
    assert_eq!(undo_depth(&harness), depth, "no undo step");
}

#[test]
fn a_hex_code_typed_is_applied_when_another_widget_takes_the_focus() {
    let (mut harness, layer) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    click(&mut harness, Role::Button, "Base color");
    let depth = undo_depth(&harness);
    click(&mut harness, Role::TextInput, "Hex color");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("FF8000".into()));
    harness.run_steps(1);

    // As a shortcut focusing another field would: the field hears it lost the focus only a pass later
    harness.ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique("another field")));
    harness.run_steps(SETTLE_FRAMES);
    let [red, green, blue] = base_color(&harness, layer);
    assert!(red > green && green > blue, "the code typed is the colour: {:?}", [red, green, blue]);
    assert_eq!(undo_depth(&harness), depth + 1, "one undo step");
}

#[test]
fn a_hex_code_half_typed_when_a_press_closes_the_picker_is_gone_when_it_opens_again() {
    let (mut harness, layer) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    click(&mut harness, Role::Button, "Base color");
    click(&mut harness, Role::TextInput, "Hex color");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("FF8000".into()));
    harness.run_steps(1);
    let view = harness.state().viewport_rect.center();
    click_at(&mut harness, view);

    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_none(), "the press closes the picker");
    assert_eq!(base_color(&harness, layer), [0.2, 0.2, 0.2], "closing applies nothing");

    let depth = undo_depth(&harness);
    click(&mut harness, Role::Button, "Base color");
    click(&mut harness, Role::TextInput, "Hex color");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(base_color(&harness, layer), [0.2, 0.2, 0.2], "the field shows the colour's own code, not the text typed before");
    assert_eq!(undo_depth(&harness), depth, "no undo step");
}

/// The open picker's frame: its title bar's top and sides, down to the air under the + chip
fn picker_face(harness: &Harness<'_, TextureCraftApp>) -> egui::Rect {
    // The air under the picker's last line (color_bar.rs PICKER_BOTTOM)
    const PICKER_BOTTOM: f32 = 10.0;
    let title = harness.get_by_role_and_label(Role::TitleBar, "Base color").rect();
    let add = harness.get_by_role_and_label(Role::Button, "Add swatch").rect();
    egui::Rect::from_min_max(title.min, egui::pos2(title.right() + CLOSE_ROOM, add.bottom() + PICKER_BOTTOM))
}

/// The title bar stops at the × (color_bar.rs: from the right edge to the close target's left edge: PAD + CLOSE_INK_HALF_WIDTH + half of CLOSE_TARGET)
const CLOSE_ROOM: f32 = 25.0;

/// The narrowest the picker resizes to and the shortest its colour field gets (color_bar.rs PICKER_MIN_WIDTH, color_picker_rows.rs FIELD_MIN_HEIGHT)
const PICKER_MIN_WIDTH: f32 = 240.0;
const FIELD_MIN_HEIGHT: f32 = 100.0;

#[test]
fn the_picker_is_titled_by_its_channel_resizes_from_its_edges_keeps_its_size_and_previews_nothing_on_hover() {
    let (mut harness, _) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    click(&mut harness, Role::Button, "Base color");

    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_some(), "the title is the channel's name alone");

    let face = picker_face(&harness);
    let field = harness.get_by_role_and_label(Role::Slider, "Saturation and value").rect();
    let hue = harness.get_by_role_and_label(Role::Slider, "Hue").rect();
    let track = harness.get_by_role_and_label(Role::Slider, "S").rect();
    let by = egui::vec2(80.0, 60.0);
    let corner = face.right_bottom() - egui::vec2(2.0, 2.0);
    drag(&mut harness, corner, corner + by);

    assert!(harness.query_by_role_and_label(Role::ComboBox, "sRGB").is_some(), "a resize keeps the picker open");

    let grown = picker_face(&harness);
    assert!((grown.min - face.min).length() < 1.0, "the bottom right corner leaves the top left where it was: {grown:?} {face:?}");
    assert!((grown.size() - (face.size() + by)).length() < 1.0, "the corner drags the size: {grown:?} from {face:?}");

    let grown_field = harness.get_by_role_and_label(Role::Slider, "Saturation and value").rect();
    let grown_hue = harness.get_by_role_and_label(Role::Slider, "Hue").rect();
    let grown_track = harness.get_by_role_and_label(Role::Slider, "S").rect();
    assert!((grown_field.size() - (field.size() + by)).length() < 1.0, "the field takes the new width and height: {grown_field:?} {field:?}");
    assert!((grown_hue.height() - (hue.height() + by.y)).abs() < 1.0 && (grown_hue.right() - hue.right() - by.x).abs() < 1.0, "the hue strip grows with it");
    assert!((grown_track.width() - (track.width() + by.x)).abs() < 1.0, "the sliders' tracks take the new width");

    let left_edge = egui::pos2(grown.left() + 2.0, grown.center().y);
    drag(&mut harness, left_edge, left_edge + egui::vec2(-50.0, 0.0));
    let wider = picker_face(&harness);
    assert!(
        (wider.left() - (grown.left() - 50.0)).abs() < 1.0 && (wider.right() - grown.right()).abs() < 1.0,
        "the left edge moves, the right stays: {wider:?}"
    );

    let wider_corner = wider.right_bottom() - egui::vec2(2.0, 2.0);
    drag(&mut harness, wider_corner, wider_corner + egui::vec2(-1000.0, -1000.0));
    let least = picker_face(&harness);
    let least_field = harness.get_by_role_and_label(Role::Slider, "Saturation and value").rect();
    assert!(least.width() >= PICKER_MIN_WIDTH - 1.0 && least_field.height() >= FIELD_MIN_HEIGHT - 1.0, "it stops at a usable size: {least:?} {least_field:?}");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);
    click(&mut harness, Role::Button, "Base color");
    let reopened = picker_face(&harness);
    assert!((reopened.size() - least.size()).length() < 1.0, "it opens again at the size it was left: {reopened:?} {least:?}");

    let value = |harness: &Harness<'_, TextureCraftApp>| harness.get_by_role_and_label(Role::Slider, "V").accesskit_node().numeric_value();
    let shown = value(&harness);
    assert!(shown.is_some(), "the V slider says its value");

    click(&mut harness, Role::ComboBox, "sRGB");
    harness.get_by_role_and_label(Role::ListBoxOption, "Linear").hover();
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(value(&harness), shown, "a hovered colour space changes nothing");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);
    click(&mut harness, Role::ComboBox, "HSV");
    harness.get_by_role_and_label(Role::ListBoxOption, "RGB").hover();
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_role_and_label(Role::Slider, "R").is_none(), "a hovered model changes nothing");

    click(&mut harness, Role::ListBoxOption, "RGB");

    assert!(harness.query_by_role_and_label(Role::Slider, "R").is_some(), "a click picks it");
}

/// The whole material the fill `layer` holds
fn whole_material(harness: &Harness<'_, TextureCraftApp>, layer: LayerId) -> Option<String> {
    fill_of(harness, layer).material
}

#[test]
fn a_plain_fill_switches_to_one_whole_material_picks_it_from_its_card_and_clears_it() {
    let (mut harness, layer) = app_with_fill(json!({}));
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Metallic").is_some(), "a plain fill opens on Channels: a card per channel");

    click_node(&mut harness, Role::RadioButton, "Material");

    assert!(harness.query_by_role_and_label(Role::ComboBox, "Metallic").is_none(), "the Material side has no channel cards");

    click_node(&mut harness, Role::ComboBox, "None");

    // The card opens the material picker, not a list: unfiltered it offers the plain materials but no smart ones
    assert!(harness.query_by_role_and_label(Role::ListBoxOption, "Steel").is_some(), "the picker offers the plain materials");
    assert!(harness.query_by_role_and_label(Role::ListBoxOption, "Steel Painted Worn").is_none(), "smart materials are folders of layers: not offered");

    // Its search keeps only the materials it names
    harness.event(egui::Event::Text("hammered".into()));
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_role_and_label(Role::ListBoxOption, "Steel").is_none(), "searching hides what it doesn't name");

    click_node(&mut harness, Role::ListBoxOption, "Copper Hammered");

    assert_eq!(whole_material(&harness, layer).as_deref(), Some("copper_hammered"), "the picker sets the whole material");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Copper Hammered").is_some(), "the card names it");
    assert!(harness.query_by_role(Role::ListBoxOption).is_none(), "the picker closed on the pick");

    click_node(&mut harness, Role::RadioButton, "Channels");

    assert!(matches!(source(&harness, layer, Channel::BaseColor), Source::Procedural { .. }), "the material set every channel");
    assert!(harness.query_by_role_and_label(Role::ComboBox, "Height").is_some(), "its channels show as cards");

    click_node(&mut harness, Role::RadioButton, "Material");
    click_in_slot(&mut harness, "Copper Hammered", Role::Button, "Clear");

    assert_eq!(whole_material(&harness, layer), None, "× drops the material");
    assert_eq!(source(&harness, layer, Channel::BaseColor), Source::uniform_of(Channel::BaseColor), "and returns the channels to their defaults");
}

/// Arms the base colour bar's eyedropper and gives its pick the click outside the app that ended as `outcome`
fn pick_outside(harness: &mut Harness<'_, TextureCraftApp>, outcome: PickOutcome) {
    click_bar_eyedropper(harness);
    harness.run_steps(SETTLE_FRAMES);
    let bar = harness.state().picking_color.expect("the eyedropper waits for a click");
    harness.state_mut().screen_pick = Some(ScreenPick::ended(bar, outcome));
    harness.run_steps(SETTLE_FRAMES);
}

#[test]
fn a_click_outside_the_app_hands_its_screen_colour_to_the_bar_and_a_cancelled_or_impossible_one_changes_nothing() {
    let (mut harness, layer) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    assert!(!harness.state().screen_picking, "tests have no real window: no system hook or grab");

    let before = base_color(&harness, layer);
    click(&mut harness, Role::Button, "Base color");

    pick_outside(&mut harness, PickOutcome::Unavailable);

    assert!(harness.state().picking_color.is_some(), "where the system cannot read the screen the pick stays on the model");
    assert_eq!(base_color(&harness, layer), before, "nothing changes until a click");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_color.is_none() && harness.state().screen_pick.is_none(), "Escape ends the pick and its wait");

    pick_outside(&mut harness, PickOutcome::Cancelled);

    assert!(harness.state().picking_color.is_none(), "dismissing the desktop's picker ends the pick");
    assert_eq!(base_color(&harness, layer), before, "a cancelled pick changes nothing");

    let undo_before = undo_depth(&harness);
    pick_outside(&mut harness, PickOutcome::Picked([255, 128, 0]));
    let picked = base_color(&harness, layer);
    let expected = [1.0, texturecraft_engine::texel::srgb_to_linear(128.0 / 255.0), 0.0];
    assert!(picked.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-3), "the screen's sRGB pixel is stored linear: {picked:?}");
    assert!(harness.state().picking_color.is_none() && harness.state().screen_pick.is_none(), "the click ends the pick");
    assert_eq!(undo_depth(&harness), undo_before + 1, "one undo step");
}

#[test]
fn an_armed_eyedropper_is_the_pointer_over_the_whole_window_and_the_app_losing_the_focus_ends_it() {
    let (mut harness, layer) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    let before = base_color(&harness, layer);
    click(&mut harness, Role::Button, "Base color");
    click_bar_eyedropper(&mut harness);
    harness.run_steps(SETTLE_FRAMES);
    let bar = harness.state().picking_color.expect("the eyedropper waits for a click");

    harness.get_by_role_and_label(Role::Tab, "Material").hover();
    harness.run_steps(SETTLE_FRAMES);
    let pointer = harness.output().platform_output.cursor_icon;
    assert_eq!(pointer, egui::CursorIcon::None, "over the Properties panel, not only the 3D view, the drawn eyedropper is the pointer");

    harness.state_mut().screen_pick = Some(ScreenPick::waiting(bar));
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_color.is_some(), "a focused app keeps waiting");

    harness.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(false);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.state().picking_color.is_none() && harness.state().screen_pick.is_none(), "another app came forward: the pick ends");
    assert_eq!(base_color(&harness, layer), before, "nothing changes");
    assert_ne!(harness.output().platform_output.cursor_icon, egui::CursorIcon::None, "the pointer is the user's again");
}

#[test]
fn escape_in_the_hex_field_ends_the_typing_and_a_second_escape_closes_the_picker() {
    let (mut harness, layer) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    click(&mut harness, Role::Button, "Base color");
    click(&mut harness, Role::TextInput, "Hex color");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("FF8000".into()));
    harness.run_steps(1);

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_some(), "the first Escape only ends the typing");
    assert_eq!(base_color(&harness, layer), [0.2, 0.2, 0.2], "and applies nothing");

    harness.key_press(egui::Key::Escape);
    harness.run_steps(SETTLE_FRAMES);

    assert!(harness.query_by_role_and_label(Role::TitleBar, "Base color").is_none(), "the next Escape closes the picker");
}

#[test]
fn a_hex_code_left_as_it_opened_keeps_a_colour_set_meanwhile() {
    let (mut harness, layer) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    click(&mut harness, Role::Button, "Base color");
    click(&mut harness, Role::TextInput, "Hex color");

    // An agent sets the colour while the field is open
    harness.state_mut().run("layer.fill.set", json!({"layer": layer.0, "channel": "baseColor", "color": [0.0, 0.0, 1.0]})).unwrap();
    harness.run_steps(2);
    let depth = undo_depth(&harness);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(SETTLE_FRAMES);

    assert_eq!(base_color(&harness, layer), [0.0, 0.0, 1.0], "the untouched code is not written back");
    assert_eq!(undo_depth(&harness), depth, "no undo step");
}

#[test]
fn a_swatch_menu_left_open_while_the_list_changes_never_removes_another_swatch() {
    let (mut harness, _) = app_with_fill(json!({"color": [0.2, 0.2, 0.2]}));
    let [red, green, blue] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for color in [red, green, blue] {
        harness.state_mut().run("swatch.add", json!({"color": color})).unwrap();
    }
    click(&mut harness, Role::Button, "Base color");
    let green_chip = harness.get_by_role_and_label(Role::Button, "#00FF00").rect();
    right_click(&mut harness, green_chip.center());

    assert!(harness.query_by_role_and_label(Role::Button, "Remove swatch").is_some(), "the green swatch's menu is open");

    // An agent removes the first swatch: green moves to its place, blue to green's
    harness.state_mut().run("swatch.remove", json!({"index": 0})).unwrap();
    harness.run_steps(SETTLE_FRAMES);
    if let Some(remove) = harness.query_by_role_and_label(Role::Button, "Remove swatch").map(|node| node.rect()) {
        click_at(&mut harness, remove.center());
    }
    let swatches = &harness.state().session.doc.project.swatches;
    assert!(swatches.contains(&blue), "blue stays: {swatches:?}");
    assert!(*swatches == vec![blue] || *swatches == vec![green, blue], "only the green swatch the menu was opened on may go: {swatches:?}");
}
