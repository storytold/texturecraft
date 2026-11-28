//! The Texture Set Settings panel: general properties, the set's channels and their mixing, and its mesh maps

use egui::{Align2, CornerRadius, FontId, Popup, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::core::{
    AoMixing, BakedMap, Channel, ChannelFormat, HeightToNormalMethod, MAX_RESOLUTION, MIN_RESOLUTION, NormalFormat, NormalMixing, TextureSet, UvPaddingMode,
};

use crate::TextureCraftApp;
use crate::i18n::{baked_map_label, tr, trf};
use crate::layer_toolbar::menu_frame;
use crate::picker_group::PickerGroup;
use crate::picker_tab::PickerTab;
use crate::property_sheet::{PropertySheet, option_row};
use crate::resource_picker;
use crate::resource_picker_item::ResourcePickerItem;
use crate::shader_settings::INSTANCE_NAME;
use crate::theme::{Tokens, semibold};

/// A channel row's height, and its name and format text sizes
const CHANNEL_ROW_HEIGHT: f32 = 26.0;
const CHANNEL_TEXT_SIZE: f32 = 12.0;

/// A channel row's gear and × buttons, and the air between them and the format text
const GLYPH_BUTTON: f32 = 20.0;
const GLYPH_SIZE: f32 = 13.0;
const GLYPH_GAP: f32 = 3.0;
const FORMAT_GAP: f32 = 6.0;
const GLYPH_ROUNDING: u8 = 3;

/// The channel format list's width and its padding
const FORMAT_LIST_WIDTH: f32 = 120.0;
const MENU_PADDING: f32 = 3.0;
/// Space between a channel's gear and its format list
const MENU_GAP: f32 = 1.0;

/// Height to normal strength's slider range
const HEIGHT_STRENGTH_RANGE: std::ops::RangeInclusive<f32> = 0.0..=10.0;

/// The empty slot's outline width (and inset), and the slot's corner rounding
const EMPTY_MAP_OUTLINE: f32 = 1.0;
const MAP_ROUNDING: u8 = 2;

pub(crate) fn show(app: &mut TextureCraftApp, ui: &mut Ui) {
    crate::channel_manager::ChannelManager::show(app, &ui.ctx().clone());
    let set_index = app.session.ts();
    let Some(set) = app.session.doc.project.texture_sets.get(set_index).cloned() else { return };
    let list = crate::texture_set_list::ListState::load(ui.ctx(), (app.session.project_id, app.session.mesh_id));
    let tile = list.tile.filter(|(picked_set, udim)| *picked_set == set_index && set.uv_tiles.contains(udim)).map(|(_, udim)| udim);
    egui::ScrollArea::vertical().id_salt("tss").show(ui, |ui| {
        PropertySheet::show(ui, ("texture_set_settings", set_index), |sheet| {
            general(app, sheet, set_index, &set, tile);
            channels(app, sheet, set_index, &set);
            mesh_maps(app, sheet, set_index, &set);
        });
    });
}

/// Name, description, size (of the UV tile picked in the list, else of the set), UV tiles and shader
fn general(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, ts: usize, set: &TextureSet, tile: Option<u32>) {
    sheet.section(tr("General Properties"), false);
    sheet.attribute(tr("Name"), &set.name);

    // Typing merges into one undo step until the field loses focus; typing after it comes back starts another
    let mut description = set.description.clone();
    let changed = sheet.text_field_above(tr("Description"), &mut description);
    let typing_id = sheet.ui.scope_id().with(("tss_description_typing", ts));
    let focused = sheet.ui.memory(|memory| memory.focused());
    let typing = sheet.ui.data(|data| data.get_temp::<Option<egui::Id>>(typing_id)).flatten();
    if typing.is_some() && typing != focused {
        sheet.ui.data_mut(|data| data.remove::<Option<egui::Id>>(typing_id));
    }
    if changed {
        let merge = typing.is_some() && typing == focused;
        sheet.ui.data_mut(|data| data.insert_temp(typing_id, focused));
        app.run("textureSet.settings", json!({"ts": ts, "description": description, "merge": merge}));
    }

    let sizes = resolutions();
    let size_name = |size: u32| size.to_string();
    if let Some(udim) = tile {
        let label = trf("Size of UV tile {udim}", &[("udim", &udim)]);
        if let Some(size) = sheet.dropdown_above(&label, set.tile_resolution(udim), &sizes, size_name) {
            app.run("textureSet.tile.resolution", json!({"ts": ts, "tile": udim, "resolution": size}));
        }
    } else {
        let lock_id = sheet.ui.scope_id().with(("tss_size_lock", ts));
        let mut locked = sheet.ui.data(|data| data.get_temp::<bool>(lock_id)).unwrap_or(set.resolution == set.height());
        let picked = sheet.dropdown_pair_above(tr("Size"), [set.resolution, set.height()], &sizes, size_name, &mut locked, tr("Lock Size"));
        sheet.ui.data_mut(|data| data.insert_temp(lock_id, locked));
        let resize = match picked {
            Some((0, width)) if locked => Some((width, width)),
            Some((0, width)) => Some((width, set.height())),
            Some((_, height)) => Some((set.resolution, height)),
            None => None,
        };
        if let Some((width, height)) = resize {
            app.start_resize_to(ts, width, height);
        }
    }

    let mut tiled = !set.uv_tiles.is_empty();
    if sheet.check(tr("UV tiles (UDIM)"), &mut tiled) {
        app.run("textureSet.uvTiles", json!({"ts": ts, "enabled": tiled}));
    }

    // The project has one shader: the list holds it
    sheet.dropdown(tr("Shader Instance"), 0, &[0], |_| tr(INSTANCE_NAME).to_owned(), None);
}

/// Add or remove channels, a row per channel (name, format and its list, ×), then how they mix
fn channels(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, ts: usize, set: &TextureSet) {
    sheet.section(tr("Channels"), false);
    let manage = sheet.button(ic::LIST_PLUS, tr("Add or remove channels"));
    if manage.clicked() {
        crate::channel_manager::ChannelManager::open(app, &manage, ts);
    }
    for (channel, format) in set.formats() {
        let (picked, remove) = channel_row(sheet, channel, format);
        if let Some(picked) = picked {
            app.run("textureSet.channel.format", json!({"ts": ts, "channel": channel.id(), "format": picked.label()}));
        }
        if remove {
            app.run("textureSet.channel.remove", json!({"ts": ts, "channel": channel.id()}));
        }
    }

    let mixing = NormalMixing::ALL;
    let mixing_name = |mixing| tr(if mixing == NormalMixing::Combine { "Combine" } else { "Replace" }).to_owned();
    if let (Some(picked), _) = sheet.dropdown(tr("Normal mixing"), set.normal_mixing, &mixing, mixing_name, None) {
        app.run("textureSet.settings", json!({"ts": ts, "normalMixing": picked.id()}));
    }
    let methods = HeightToNormalMethod::ALL;
    let method_name = |method| tr(if method == HeightToNormalMethod::Sharp { "Sharp" } else { "Smooth (Sobel)" }).to_owned();
    if let (Some(picked), _) = sheet.dropdown(tr("Height to normal method"), set.height_to_normal, &methods, method_name, None) {
        app.run("textureSet.settings", json!({"ts": ts, "heightToNormal": picked.id()}));
    }
    let mut strength = set.height_strength;
    if let Some(merge) = sheet.slider(tr("Height to normal strength"), &mut strength, HEIGHT_STRENGTH_RANGE, 2) {
        app.run("textureSet.settings", json!({"ts": ts, "heightStrength": strength, "merge": merge}));
    }

    // Occlusion mixes a baked or set ambient occlusion map in: greyed until the set has one
    let has_occlusion = app.session.doc.baked.get(&ts).is_some_and(|baked| baked.maps.contains_key(&BakedMap::AmbientOcclusion));
    let occlusion = AoMixing::ALL;
    let occlusion_name = |mixing| tr(if mixing == AoMixing::Multiply { "Multiply" } else { "None" }).to_owned();
    let picked = sheet.greyed(has_occlusion, Some(tr("Needs an Ambient occlusion mesh map")), |sheet| {
        sheet.dropdown(tr("Ambient occlusion mixing"), set.ao_mixing, &occlusion, occlusion_name, None).0
    });
    if let Some(picked) = picked {
        app.run("textureSet.settings", json!({"ts": ts, "aoMixing": picked.id()}));
    }

    let paddings = UvPaddingMode::ALL;
    let padding_name = |mode| tr(if mode == UvPaddingMode::Space3d { "Across Seams (3D)" } else { "From Island Edges (2D)" }).to_owned();
    if let (Some(picked), _) = sheet.dropdown(tr("UV padding"), set.uv_padding, &paddings, padding_name, None) {
        app.run("textureSet.settings", json!({"ts": ts, "uvPadding": picked.id()}));
    }

    // For the whole project, shown here beside the other normal settings
    let formats = [NormalFormat::OpenGl, NormalFormat::DirectX];
    let format_name = |format| if format == NormalFormat::DirectX { "DirectX" } else { "OpenGL" }.to_owned();
    let current = app.session.doc.project.normal_format;
    if let (Some(picked), _) = sheet.dropdown(tr("Normal map format"), current, &formats, format_name, None) {
        let id = if picked == NormalFormat::DirectX { "directX" } else { "openGl" };
        app.run("textureSet.settings", json!({"ts": ts, "normalFormat": id}));
    }
}

/// A channel: its name, its format with the gear that lists the formats, and × (Base color stays); returns the format picked and whether × was clicked
fn channel_row(sheet: &mut PropertySheet<'_>, channel: Channel, format: ChannelFormat) -> (Option<ChannelFormat>, bool) {
    sheet.embed(|ui| {
        let tokens = Tokens::get(ui.ctx());
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), CHANNEL_ROW_HEIGHT), Sense::hover());
        let name = tr(channel.label());
        let mut picked = None;
        let mut removed = false;

        let remove = Rect::from_center_size(pos2(rect.right() - GLYPH_BUTTON * 0.5, rect.center().y), vec2(GLYPH_BUTTON, GLYPH_BUTTON));
        if channel != Channel::BaseColor
            && glyph_button(ui, ("tss_remove", channel.id()), remove, ic::X, &trf("Remove {channel}", &[("channel", &name)]), &tokens).clicked()
        {
            removed = true;
        }

        let gear = remove.translate(vec2(-GLYPH_BUTTON - GLYPH_GAP, 0.0));
        let gear_label = trf("{channel} format", &[("channel", &name)]);
        let gear_response = glyph_button(ui, ("tss_format", channel.id()), gear, ic::GEAR_SIX, &gear_label, &tokens);
        Popup::menu(&gear_response).frame(menu_frame(&tokens)).width(FORMAT_LIST_WIDTH).gap(MENU_GAP).align(egui::RectAlign::BOTTOM_END).show(|ui| {
            ui.set_min_width(FORMAT_LIST_WIDTH);
            ui.add_space(MENU_PADDING);
            for option in ChannelFormat::ALL {
                if option_row(ui, option.label(), option == format).clicked() && option != format {
                    picked = Some(option);
                }
            }
            ui.add_space(MENU_PADDING);
        });

        let painter = ui.painter().clone();
        let format_at = pos2(gear.left() - FORMAT_GAP, rect.center().y);
        let format_rect = painter.text(format_at, Align2::RIGHT_CENTER, format.label(), FontId::proportional(CHANNEL_TEXT_SIZE), tokens.text_dim);
        let name_rect = painter.text(pos2(rect.left(), rect.center().y), Align2::LEFT_CENTER, name, semibold(CHANNEL_TEXT_SIZE), tokens.text);
        painter.hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, tokens.separator));

        // The drawn name and format read as labels
        for (salt, text_rect, text) in [("tss_channel_name", name_rect, name), ("tss_channel_format", format_rect, format.label())] {
            let label = ui.interact(text_rect, ui.scope_id().with((salt, channel.id())), Sense::hover());
            label.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
        }

        (picked, removed)
    })
}

/// The Bake Mesh Maps button (the bake mode's own icon), then a slot per map: a click lists the images, × clears it
fn mesh_maps(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, ts: usize, set: &TextureSet) {
    sheet.section(tr("Mesh Maps"), false);
    if sheet.button(ic::ATOM, tr("Bake Mesh Maps")).clicked() {
        crate::menus::invoke(app, &sheet.ui.ctx().clone(), "ui.mode.bake");
    }
    let empty_outline = Tokens::get(sheet.ui.ctx()).field_border;
    for map in BakedMap::ALL {
        let present = app.session.doc.baked.get(&ts).is_some_and(|baked| baked.maps.contains_key(&map));
        let color = crate::dock::mesh_map_color(map);
        let thumbnail = move |painter: &egui::Painter, rect: Rect| {
            if present {
                painter.rect_filled(rect, CornerRadius::same(MAP_ROUNDING), color);
            } else {
                painter.rect_stroke(
                    rect.shrink(EMPTY_MAP_OUTLINE),
                    CornerRadius::same(MAP_ROUNDING),
                    Stroke::new(EMPTY_MAP_OUTLINE, empty_outline),
                    StrokeKind::Inside,
                );
            }
        };
        let label = tr(baked_map_label(map));
        let source = if present { format!("{}_{}", set.name, map.id()) } else { trf("Select {map} map", &[("map", &label.to_lowercase())]) };
        let slot = sheet.slot(&thumbnail, label, &source, None, present, false);
        sheet.close_card();
        if slot.clear {
            app.run("textureSet.meshMap.clear", json!({"ts": ts, "map": map.id()}));
        }
        let popup = slot.pick.id.with("mesh_map_picker");
        if slot.pick.clicked() {
            resource_picker::toggle(&slot.pick.ctx, popup);
        }
        let tabs = || {
            vec![PickerTab {
                icon: ic::SQUARES_FOUR,
                name: tr("Resources"),
                groups: vec![PickerGroup {
                    header: Some(tr("Textures")),
                    items: ResourcePickerItem::textures(&app.session, None, |name| Some(name.to_owned())),
                }],
            }]
        };
        let picked = resource_picker::show(&slot.pick, popup, &mut app.thumbs, None, false, tabs);

        // A shelf image dropped on the slot becomes this map too
        let dropped = crate::dock::image_drop(sheet.ui, &slot.pick).map(Some);

        // The picker's Import... tile is None
        match picked.or(dropped) {
            Some(None) => crate::menus::invoke(app, &sheet.ui.ctx().clone(), "resource.import"),
            Some(Some(name)) => {
                app.run("textureSet.meshMap.set", json!({"ts": ts, "map": map.id(), "image": name}));
            }
            None => {}
        }
    }
}

/// Every size a set or tile can take
fn resolutions() -> Vec<u32> {
    std::iter::successors(Some(MIN_RESOLUTION), |size| size.checked_mul(2)).take_while(|size| *size <= MAX_RESOLUTION).collect()
}

/// A small square glyph button with its face on hover
fn glyph_button(ui: &mut Ui, id: impl egui::AsIdSalt, rect: Rect, glyph: &str, label: &str, tokens: &Tokens) -> egui::Response {
    let response = ui.interact(rect, ui.scope_id().with(id), Sense::click()).on_hover_text(label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(GLYPH_ROUNDING), tokens.row_hover);
    }
    let color = if response.hovered() { tokens.text } else { tokens.text_dim };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(GLYPH_SIZE), color);
    response
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};

    use super::*;

    fn harness(mesh: &str) -> Harness<'static, TextureCraftApp> {
        let mesh_ref = format!("builtin:{mesh}");
        let geometry = texturecraft_engine::geom::primitives::builtin(mesh).unwrap();
        let session = texturecraft_engine::Session::new_project("Texture set settings", geometry, &mesh_ref, None, 128).unwrap();
        let mut harness = Harness::builder()
            // Tall enough that the list's rows and the settings' channels show without scrolling
            .with_size(vec2(1600.0, 2400.0))
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|context| TextureCraftApp::new(context, session));
        harness.run_steps(4);
        let ctx = harness.ctx.clone();
        crate::menus::invoke(harness.state_mut(), &ctx, "ui.view.textureSetSettings");
        harness.run_steps(4);
        harness
    }

    /// Presses and lets go at `at` through the real pointer path, as a person's click lands on a control drawn over a row
    fn click_at(harness: &mut Harness<'static, TextureCraftApp>, at: egui::Pos2) {
        let modifiers = egui::Modifiers::NONE;
        harness.event(egui::Event::PointerMoved(at));
        harness.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers });
        harness.run_steps(1);
        harness.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers });
        harness.run_steps(2);
    }

    fn click(harness: &mut Harness<'static, TextureCraftApp>, role: egui::Role, label: &str) {
        let rect = harness.get_by_role_and_label(role, label).rect();

        // A row's middle is under its size dropdown in a narrow panel: aim at its name
        let at = if role == egui::Role::ListItem { egui::pos2(rect.left() + rect.width() * 0.25, rect.center().y) } else { rect.center() };
        click_at(harness, at);
    }

    /// Steps until the resize worker has handed its result back
    fn finish_resize(harness: &mut Harness<'static, TextureCraftApp>) {
        for _ in 0..200 {
            harness.run_steps(1);
            if harness.state().resize.is_none() {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        harness.run_steps(2);
    }

    #[test]
    fn typing_in_the_description_again_after_leaving_it_is_its_own_undo_step() {
        let mut harness = harness("material_ball");
        let description = |harness: &Harness<'static, TextureCraftApp>| harness.state().session.doc.project.texture_sets[0].description.clone();
        let type_into_description = |harness: &mut Harness<'static, TextureCraftApp>, text: &str| {
            click(harness, egui::Role::TextInput, "Description");
            for character in text.chars() {
                harness.event(egui::Event::Text(character.to_string()));
                harness.run_steps(1);
            }
            let ctx = harness.ctx.clone();
            ctx.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
            harness.run_steps(2);
        };

        type_into_description(&mut harness, "ab");
        let first = description(&harness);
        type_into_description(&mut harness, "c");
        assert!(description(&harness).len() > first.len(), "the second typing landed: {:?}", description(&harness));

        harness.state_mut().session.undo().unwrap();
        assert_eq!(description(&harness), first, "undo takes back the second typing alone");
    }

    #[test]
    fn a_uv_tile_picked_in_the_list_is_what_size_edits() {
        let mut harness = harness("udim_plane");
        let tiles = harness.state().session.doc.project.texture_sets[0].uv_tiles.clone();
        assert!(harness.query_by_role_and_label(egui::Role::ComboBox, &format!("Size of UV tile {}", tiles[1])).is_none());

        click(&mut harness, egui::Role::ListItem, &format!("UV tile {}", tiles[1]));
        click(&mut harness, egui::Role::ComboBox, &format!("Size of UV tile {}", tiles[1]));
        click(&mut harness, egui::Role::ListBoxOption, "256");
        harness.run_steps(3);

        let set = &harness.state().session.doc.project.texture_sets[0];
        assert!(set.tile_resolution(tiles[1]) > set.tile_resolution(tiles[0]), "only the picked tile grew");
    }

    #[test]
    fn unlocked_size_sets_the_height_alone() {
        let mut harness = harness("material_ball");
        click(&mut harness, egui::Role::CheckBox, "Lock Size");
        let height =
            harness.get_all_by_role_and_label(egui::Role::ComboBox, "Size").max_by(|a, b| a.rect().left().total_cmp(&b.rect().left())).unwrap().rect().center();
        click_at(&mut harness, height);
        click(&mut harness, egui::Role::ListBoxOption, "256");
        finish_resize(&mut harness);

        let set = &harness.state().session.doc.project.texture_sets[0];
        assert!(set.height() > set.resolution, "{}x{}", set.resolution, set.height());
    }

    #[test]
    fn a_channel_gear_lists_the_formats_and_stores_the_one_picked() {
        let mut harness = harness("material_ball");
        let format = |harness: &Harness<'static, TextureCraftApp>| {
            let formats = harness.state().session.doc.project.texture_sets[0].formats();
            formats.into_iter().find(|(channel, _)| *channel == Channel::BaseColor).map(|(_, format)| format)
        };
        let before = format(&harness);

        click(&mut harness, egui::Role::Button, "Base color format");
        click(&mut harness, egui::Role::ListBoxOption, "RGB32F");
        harness.run_steps(3);

        assert!(format(&harness) != before, "{:?}", format(&harness));
    }

    #[test]
    fn the_channel_window_marks_unread_material_channels_but_not_user_channels() {
        let mut harness = harness("material_ball");
        click(&mut harness, egui::Role::Button, "Add or remove channels");

        // The dialog lays itself out over a few frames before its checks are in the tree
        harness.run_steps(4);
        let marked = |label: &str| harness.get_by_role_and_label(egui::Role::CheckBox, label).accesskit_node().description().is_some();

        assert!(marked("Base weight"), "a channel the shader ignores is marked");
        assert!(!marked("User0"), "a user channel is not");
    }

    #[test]
    fn a_narrow_window_can_apply_its_channel_selection() {
        let mut harness = harness("material_ball");
        click(&mut harness, egui::Role::Button, "Add or remove channels");
        harness.set_size(vec2(390.0, 600.0));
        harness.run_steps(5);
        let viewport = harness.ctx.content_rect();
        let apply = harness.get_by_role_and_label(egui::Role::Button, "Apply").rect();

        assert!(viewport.contains_rect(apply), "Apply must fit on screen");
        click(&mut harness, egui::Role::CheckBox, "Base weight");
        click(&mut harness, egui::Role::Button, "Apply");

        assert!(harness.state().session.doc.project.texture_sets[0].has_channel(Channel::BaseWeight));
    }

    #[test]
    fn a_narrow_channel_window_keeps_its_legend_inside_the_footer() {
        let mut harness = harness("material_ball");
        click(&mut harness, egui::Role::Button, "Add or remove channels");
        harness.set_size(vec2(300.0, 600.0));
        harness.run_steps(5);

        let legend: Vec<_> = harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == "Not used by the shader" => Some((clipped.clip_rect, text.visual_bounding_rect())),
                _ => None,
            })
            .collect();
        assert!(!legend.is_empty(), "the footer paints its legend");
        for (clip, painted) in legend {
            assert!(clip.contains_rect(painted), "the legend at {painted:?} is cut off by {clip:?}");
        }
    }

    #[test]
    fn a_short_channel_window_keeps_apply_on_screen() {
        // A narrow window stacks the footer to three rows: the channel list must leave room for all of them
        let mut harness = harness("material_ball");
        click(&mut harness, egui::Role::Button, "Add or remove channels");
        harness.set_size(vec2(300.0, 400.0));
        harness.run_steps(5);

        let screen = harness.ctx.content_rect();
        let apply: Vec<_> = harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == "Apply" => Some(text.visual_bounding_rect()),
                _ => None,
            })
            .collect();
        assert!(!apply.is_empty(), "the footer paints Apply");
        for painted in apply {
            assert!(screen.contains_rect(painted), "Apply at {painted:?} falls outside the window {screen:?}");
        }
    }

    #[test]
    fn escape_closes_the_channel_window_with_nothing_focused_and_drops_its_ticks() {
        let mut harness = harness("material_ball");
        let had = harness.state().session.doc.project.texture_sets[0].has_channel(Channel::User0);
        click(&mut harness, egui::Role::Button, "Add or remove channels");
        harness.run_steps(4);
        click(&mut harness, egui::Role::CheckBox, "User0");

        // As after a click on the dialog's empty space: no control keeps the keyboard
        let ctx = harness.ctx.clone();
        ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
        harness.run_steps(2);

        harness.key_press(egui::Key::Escape);
        harness.run_steps(4);
        assert!(harness.query_by_role_and_label(egui::Role::CheckBox, "User0").is_none(), "the window closed");
        assert_eq!(harness.state().session.doc.project.texture_sets[0].has_channel(Channel::User0), had, "nothing was applied");
    }
}
