//! The Layers panel's top row: the displayed channel, the Add effect and Add mask menus, the add-layer buttons and remove

use egui::{Align2, Color32, CornerRadius, FontId, Frame, Id, Painter, Pos2, Rect, Response, Sense, Shadow, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::Session;
use texturecraft_engine::core::{Layer, LayerId};
use texturecraft_engine::paint::alphas;

use crate::TextureCraftApp;
use crate::i18n::{n_, tr};
use crate::picker_group::PickerGroup;
use crate::picker_tab::PickerTab;
use crate::resource_picker;
use crate::resource_picker_item::ResourcePickerItem;
use crate::theme::{HARD_SHADOW, Tokens};
use crate::tile_picture::TilePicture;

mod add_item;
mod bitmap_choice;
mod item_icon;
mod menu_icon;
mod pick;

use add_item::AddItem;
use bitmap_choice::BitmapChoice;
use item_icon::ItemIcon;
use menu_icon::MenuIcon;
use pick::Pick;

/// The row's height and its controls' height
const BAR_HEIGHT: f32 = 40.0;
const CONTROL_HEIGHT: f32 = 26.0;
const CONTROL_ROUNDING: u8 = 5;

/// Room between the row's ends and its first and last controls
const SIDE_PADDING: f32 = 8.0;

const CHANNEL_WIDTH: f32 = 88.0;
/// A menu button: its icon, then its caret
const MENU_BUTTON_WIDTH: f32 = 32.0;
const MENU_ICON_X: f32 = 11.0;
const MENU_CARET_INSET: f32 = 8.0;
const MENU_ICON_SIZE: f32 = 14.0;
const MASK_ICON_SIZE: f32 = 12.0;
const CARET_SIZE: f32 = 11.0;
const BUTTON_WIDTH: f32 = 24.0;
const ICON_SIZE: f32 = 14.0;
/// Air after the channel list, between the two menu buttons, before the add-layer buttons and between them
const CHANNEL_GAP: f32 = 6.0;
const MENU_GAP: f32 = 3.0;
const GROUP_GAP: f32 = 8.0;
const BUTTON_GAP: f32 = 2.0;
const CHANNEL_TEXT_X: f32 = 8.0;
const CHANNEL_CARET_INSET: f32 = 10.0;
const CHANNEL_CARET_SIZE: f32 = 12.0;
const TEXT_SIZE: f32 = 12.0;

/// The row's width besides the channel list's: its padding, the two menu buttons and their gaps
const MENUS_ROW_WIDTH: f32 = 2.0 * SIDE_PADDING + CHANNEL_GAP + 2.0 * MENU_BUTTON_WIDTH + MENU_GAP;
/// The three add-layer buttons and Remove
const LAYER_BUTTONS_WIDTH: f32 = 4.0 * BUTTON_WIDTH + 3.0 * BUTTON_GAP;
/// The width every control takes in one row
const ONE_ROW_WIDTH: f32 = MENUS_ROW_WIDTH + CHANNEL_WIDTH + GROUP_GAP + LAYER_BUTTONS_WIDTH;

/// A hanging menu: its caption, rows, icon and text columns and its smallest width
const MENU_PADDING: f32 = 4.0;
const MENU_CAPTION_HEIGHT: f32 = 22.0;
const MENU_CAPTION_X: f32 = 12.0;
const MENU_CAPTION_SIZE: f32 = 10.0;
const MENU_ROW: f32 = 24.0;
const MENU_ROW_INSET: f32 = 4.0;
const MENU_ROW_ROUNDING: u8 = 4;
const MENU_ITEM_ICON_X: f32 = 18.0;
const MENU_ITEM_ICON_SIZE: f32 = 13.0;
const MENU_ITEM_SWATCH: f32 = 12.0;
const MENU_ITEM_SWATCH_ROUNDING: u8 = 2;
const MENU_TEXT_X: f32 = 34.0;
const MENU_RIGHT: f32 = 22.0;
const MENU_MIN_WIDTH: f32 = 120.0;
const MENU_ROUNDING: u8 = 6;

/// The menu rises over its button's bottom edge by this much (a negative gap), so the two read as one tab with no seam
const MENU_TAB_OVERLAP: f32 = -1.0;

const SHADOW_OFFSET: [i8; 2] = [3, 4];
/// The two swatches of Add mask's White and Black
const WHITE_SWATCH: u8 = 235;
const BLACK_SWATCH: u8 = 10;

/// The filter Add filter adds: an empty one, which changes nothing until its slot's picker swaps a filter into it
pub(crate) const NEW_FILTER: &str = texturecraft_engine::mask_cmds::EMPTY_FILTER;

/// The Layers panel's top row and what its menus open
pub(crate) fn show(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());

    // A panel narrower than one row puts the add-layer buttons and Remove on a second row, where they stay inside it
    let width = ui.available_width();
    let two_rows = width < ONE_ROW_WIDTH;
    let (area, _) = ui.allocate_exact_size(vec2(width, if two_rows { 2.0 * BAR_HEIGHT } else { BAR_HEIGHT }), Sense::hover());
    let anchor = ui.interact(area, ui.scope_id().with("layer_toolbar"), Sense::hover());
    let bar = Rect::from_min_size(area.min, vec2(width, BAR_HEIGHT));
    let top = bar.center().y - CONTROL_HEIGHT * 0.5;
    let mut x = bar.left() + SIDE_PADDING;
    let control = |x: f32, top: f32, width: f32| Rect::from_min_size(pos2(x, top), vec2(width, CONTROL_HEIGHT));

    // Narrower still, the channel list gives up width before a menu button leaves the panel
    let channel_width = CHANNEL_WIDTH.min(width - MENUS_ROW_WIDTH).max(0.0);
    channel_list(app, ui, control(x, top, channel_width), &tokens);
    x += channel_width + CHANNEL_GAP;

    let selected = app.session.selected_layer().filter(|id| app.session.doc.project.layers.contains_key(id));
    let effects = || effect_items(app, layer_of(app, selected), false);
    let mut pick = menu_button(ui, control(x, top, MENU_BUTTON_WIDTH), tr("Add effect"), MenuIcon::Glyph(ic::MAGIC_WAND), tr("Add effect"), effects, &tokens);
    x += MENU_BUTTON_WIDTH + MENU_GAP;
    let masks = || mask_items(layer_of(app, selected));
    pick = pick.or(menu_button(ui, control(x, top, MENU_BUTTON_WIDTH), tr("Add mask"), MenuIcon::Mask, tr("Add mask"), masks, &tokens));
    x += MENU_BUTTON_WIDTH + GROUP_GAP;

    let (top, mut x) = if two_rows { (top + BAR_HEIGHT, bar.left() + SIDE_PADDING) } else { (top, x) };
    for (glyph, label, command) in [
        (ic::PAINT_BRUSH, tr("Add paint layer"), "layer.add.paint"),
        (ic::PAINT_BUCKET, tr("Add fill layer"), "layer.add.fill"),
        (ic::FOLDER_SIMPLE, tr("Add folder"), "layer.add.group"),
    ] {
        if plain_button(ui, control(x, top, BUTTON_WIDTH), glyph, label, &tokens).clicked() {
            app.run(command, Value::Null);
        }
        x += BUTTON_WIDTH + BUTTON_GAP;
    }
    let remove = control((bar.right() - SIDE_PADDING - BUTTON_WIDTH).max(x), top, BUTTON_WIDTH);
    if plain_button(ui, remove, ic::TRASH, &crate::keymap::tip(&app.ui.keymap, tr("Remove layer"), "layer.remove"), &tokens).clicked() {
        app.run("layer.remove", Value::Null);
    }

    // Key the picker to its opening layer so a selection change discards its result
    let popup = |layer: LayerId| ui.scope_id().with(("bitmap_mask_picker", layer.0));
    match (pick, selected) {
        (Some(Pick::Run { command, params }), Some(layer)) => run_on(app, ui, layer, command, params),
        (Some(Pick::Bitmap), Some(layer)) => resource_picker::toggle(ui.ctx(), popup(layer)),
        _ => {}
    }
    if let Some(layer) = selected {
        bitmap_picker(app, &anchor, popup(layer), layer);
    }
}

/// Layer `id` of the project, when there is one
fn layer_of(app: &TextureCraftApp, id: Option<LayerId>) -> Option<&Layer> {
    id.and_then(|id| app.session.doc.project.layers.get(&id))
}

/// How far a hovered dropdown lightens towards the text colour
const HOVER_TINT: f32 = 0.06;

/// Air between the channel's name and its caret, and between the button and its list
const CHANNEL_TEXT_AIR: f32 = 2.0;
const CHANNEL_LIST_GAP: f32 = 1.0;

/// The displayed channel, or Material for the whole material: a list hanging from its button
fn channel_list(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect, tokens: &Tokens) {
    let response = ui.interact(rect, ui.scope_id().with("layer_channel"), Sense::click());
    let current = if app.ui.layer_material_view { tr("Material") } else { tr(app.ui.layer_channel.label()) };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, current));
    let painter = ui.painter();
    let fill = if response.hovered() { tokens.button.lerp_to_gamma(tokens.text, HOVER_TINT) } else { tokens.button };
    painter.rect(rect, CornerRadius::same(CONTROL_ROUNDING), fill, Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
    let caret_x = rect.right() - CHANNEL_CARET_INSET;
    let text = crate::widgets::elided(
        painter,
        current.to_owned(),
        TEXT_SIZE,
        tokens.text,
        caret_x - CHANNEL_CARET_SIZE * 0.5 - rect.left() - CHANNEL_TEXT_X - CHANNEL_TEXT_AIR,
    );
    painter.galley(pos2(rect.left() + CHANNEL_TEXT_X, rect.center().y - text.size().y * 0.5), text, tokens.text);
    painter.text(pos2(caret_x, rect.center().y), Align2::CENTER_CENTER, ic::CARET_DOWN, FontId::proportional(CHANNEL_CARET_SIZE), tokens.text);

    let channels = app.session.doc.project.texture_set(app.session.ts()).map(|set| set.channel_list()).unwrap_or_default();
    egui::Popup::menu(&response).id(ui.scope_id().with("layer_channel_list")).width(rect.width()).gap(CHANNEL_LIST_GAP).show(|ui| {
        ui.set_min_width(rect.width());
        if ui.selectable_label(app.ui.layer_material_view, tr("Material")).clicked() {
            app.ui.layer_material_view = true;
        }
        for channel in channels {
            let picked = !app.ui.layer_material_view && app.ui.layer_channel == channel;
            if ui.selectable_label(picked, tr(channel.label())).clicked() {
                app.ui.layer_channel = channel;
                app.ui.layer_material_view = false;
            }
        }
    });
}

/// How far the open dropdown's tab reaches into its list, and how much of the seam under it the tab covers
const TAB_OVERLAP: f32 = 1.5;
const TAB_SEAM_COVER: f32 = 2.5;

/// The tab's border width: the seam cover stays inside it and its left line runs on down over the seam
const TAB_BORDER_WIDTH: f32 = 1.0;

/// A button with its icon and a caret whose menu hangs from it (caption, then the `items` built while it is open); the item picked
fn menu_button(ui: &mut Ui, rect: Rect, label: &str, icon: MenuIcon, caption: &str, items: impl FnOnce() -> Vec<AddItem>, tokens: &Tokens) -> Option<Pick> {
    let response = tip_above(ui.interact(rect, ui.scope_id().with(("layer_toolbar_menu", label)), Sense::click()), label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let popup = ui.scope_id().with(("layer_toolbar_menu_popup", label));
    let open = egui::Popup::is_id_open(ui.ctx(), popup);
    let painter = ui.painter();
    if response.hovered() && !open {
        painter.rect_filled(rect, CornerRadius::same(CONTROL_ROUNDING), tokens.button);
    }
    button_face(painter, rect, icon, tokens);

    // A click opens the menu this frame
    let items = if open || response.clicked() { items() } else { Vec::new() };
    let width = menu_width(ui, &items);
    let frame = Frame::NONE
        .fill(tokens.chrome)
        .stroke(Stroke::new(1.0, tokens.field_border))
        .corner_radius(CornerRadius { nw: 0, ..CornerRadius::same(MENU_ROUNDING) })
        .shadow(Shadow { offset: SHADOW_OFFSET, blur: 0, spread: 0, color: HARD_SHADOW });
    let mut picked = None;
    let shown = egui::Popup::menu(&response).id(popup).align(egui::RectAlign::BOTTOM_START).gap(MENU_TAB_OVERLAP).frame(frame).width(width).show(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
        ui.add_space(MENU_PADDING);
        let (head, _) = ui.allocate_exact_size(vec2(width, MENU_CAPTION_HEIGHT - MENU_PADDING), Sense::hover());
        let caption = egui::RichText::new(caption.to_uppercase()).size(MENU_CAPTION_SIZE).strong().color(tokens.text_dim);
        let galley = egui::WidgetText::from(caption).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
        ui.painter().galley(pos2(head.left() + MENU_CAPTION_X, head.center().y - galley.size().y * 0.5), galley, tokens.text_dim);
        for item in &items {
            if menu_row(ui, width, item, tokens).clicked() {
                picked = Some(if item.command == BITMAP { Pick::Bitmap } else { Pick::Run { command: item.command, params: item.params.clone() } });
                ui.close();
            }
        }
        ui.add_space(MENU_PADDING);
    });

    // The open button becomes the menu's tab: same fill and border, no line between them
    if let Some(shown) = shown {
        let painter = ui.ctx().layer_painter(shown.response.layer_id);
        let tab = Rect::from_min_max(rect.min, pos2(rect.right(), shown.response.rect.top() + TAB_OVERLAP));
        let top = CornerRadius { nw: CONTROL_ROUNDING, ne: CONTROL_ROUNDING, sw: 0, se: 0 };
        painter.rect(tab, top, tokens.chrome, Stroke::new(TAB_BORDER_WIDTH, tokens.field_border), StrokeKind::Inside);
        painter.rect_filled(
            Rect::from_min_max(pos2(tab.left() + TAB_BORDER_WIDTH, tab.bottom() - TAB_SEAM_COVER), pos2(tab.right() - TAB_BORDER_WIDTH, tab.bottom())),
            CornerRadius::ZERO,
            tokens.chrome,
        );
        let border_x = tab.left() + TAB_BORDER_WIDTH * 0.5;
        painter.vline(border_x, tab.bottom() - TAB_SEAM_COVER..=tab.bottom(), Stroke::new(TAB_BORDER_WIDTH, tokens.field_border));
        button_face(&painter, rect, icon, tokens);
    }
    picked
}

/// A menu button's icon and caret
fn button_face(painter: &Painter, rect: Rect, icon: MenuIcon, tokens: &Tokens) {
    let icon_at = pos2(rect.left() + MENU_ICON_X, rect.center().y);
    match icon {
        MenuIcon::Glyph(glyph) => {
            painter.text(icon_at, Align2::CENTER_CENTER, glyph, FontId::proportional(MENU_ICON_SIZE), tokens.text);
        }
        MenuIcon::Mask => mask_icon(painter, icon_at, MASK_ICON_SIZE, tokens.text),
    }
    painter.text(pos2(rect.right() - MENU_CARET_INSET, rect.center().y), Align2::CENTER_CENTER, ic::CARET_DOWN, FontId::proportional(CARET_SIZE), tokens.text);
}

/// The menu's width: its longest item, with room for the icons and the right edge
fn menu_width(ui: &Ui, items: &[AddItem]) -> f32 {
    let widest = items
        .iter()
        .map(|item| ui.fonts_mut(|fonts| fonts.layout_no_wrap(tr(item.short).to_owned(), FontId::proportional(TEXT_SIZE), Color32::WHITE).size().x))
        .fold(0.0, f32::max);
    (widest + MENU_TEXT_X + MENU_RIGHT).max(MENU_MIN_WIDTH)
}

/// One menu row: hover face, icon, name; dim and unclickable when it can't go where the selection is
fn menu_row(ui: &mut Ui, width: f32, item: &AddItem, tokens: &Tokens) -> Response {
    let sense = if item.enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, MENU_ROW), sense);
    let label = tr(item.short);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, item.enabled, label));
    let painter = ui.painter();
    if item.enabled && response.hovered() {
        painter.rect_filled(rect.shrink2(vec2(MENU_ROW_INSET, 0.0)), CornerRadius::same(MENU_ROW_ROUNDING), tokens.button);
    }
    let color = if item.enabled { tokens.text } else { tokens.text_dim };
    let icon_at = pos2(rect.left() + MENU_ITEM_ICON_X, rect.center().y);
    match item.icon {
        ItemIcon::Glyph(glyph) => {
            painter.text(icon_at, Align2::CENTER_CENTER, glyph, FontId::proportional(MENU_ITEM_ICON_SIZE), color);
        }
        ItemIcon::Swatch(gray) => {
            let swatch = Rect::from_center_size(icon_at, vec2(MENU_ITEM_SWATCH, MENU_ITEM_SWATCH));
            painter.rect(
                swatch,
                CornerRadius::same(MENU_ITEM_SWATCH_ROUNDING),
                Color32::from_gray(gray),
                Stroke::new(1.0, tokens.field_border),
                StrokeKind::Inside,
            );
        }
    }
    painter.text(pos2(rect.left() + MENU_TEXT_X, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(TEXT_SIZE), color);
    response
}

/// The tooltip of toolbar button `response`, above it so it covers none of the layer rows under the toolbar
fn tip_above(response: Response, text: &str) -> Response {
    let mut tooltip = egui::Tooltip::for_enabled(&response);
    tooltip.popup = tooltip.popup.anchor(response.rect).align(egui::RectAlign::TOP).align_alternatives(&[egui::RectAlign::LEFT, egui::RectAlign::RIGHT]);
    tooltip.show(|ui| ui.label(text));
    response
}

/// A plain icon button of the row: a button face while hovered
fn plain_button(ui: &mut Ui, rect: Rect, glyph: &str, label: &str, tokens: &Tokens) -> Response {
    let response = tip_above(ui.interact(rect, ui.scope_id().with(("layer_toolbar_button", glyph)), Sense::click()), label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(CONTROL_ROUNDING), tokens.button);
    }
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(ICON_SIZE), tokens.text);
    response
}

/// The outline width and corner rounding of a drawn toolbar icon
const ICON_LINE_WIDTH: f32 = 1.5;
const ICON_ROUNDING: u8 = 2;

/// Add mask's icon: a square, half of it filled corner to corner
pub(crate) fn mask_icon(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let half = size * 0.5;
    let filled = vec![pos2(center.x + half, center.y - half), pos2(center.x + half, center.y + half), pos2(center.x - half, center.y + half)];
    painter.add(egui::Shape::convex_polygon(filled, color, Stroke::NONE));
    painter.rect_stroke(
        Rect::from_center_size(center, vec2(size, size)),
        CornerRadius::same(ICON_ROUNDING),
        Stroke::new(ICON_LINE_WIDTH, color),
        StrokeKind::Inside,
    );
}

/// Selects `layer` when it isn't, runs `command` on it, and shows a filter Add filter added in Properties with its picker open
fn run_on(app: &mut TextureCraftApp, ui: &Ui, layer: LayerId, command: &str, params: Value) {
    if app.session.selected_layer() != Some(layer) {
        app.run("layer.select", json!({"layer": layer.0}));
    }
    let mut params = params;
    if let Some(object) = params.as_object_mut() {
        object.insert("layer".into(), json!(layer.0));
    }
    let added = app.run(command, params);
    if command == "effect.add.filter"
        && let Some(effect) = added.as_ref().and_then(|result| result["effect"].as_u64())
    {
        app.ui.dock.focus(crate::docking::Panel::Properties);
        crate::filter_slot::open_picker_when_shown(ui.ctx(), effect);
    }
}

/// The Add effect items for `layer`: on its content when the content is the selected target (or `content_thumbnail`: the menu of the layer's content thumbnail, whatever is selected), else in its mask; an item that can't go there is greyed, not sent elsewhere (all of them without a layer)
fn effect_items(app: &TextureCraftApp, layer: Option<&Layer>, content_thumbnail: bool) -> Vec<AddItem> {
    let content = layer
        .is_some_and(|layer| content_thumbnail || (!layer.is_group() && app.session.selected_layer() == Some(layer.id) && crate::dock::targets_content(app)));
    let target = if content { "content" } else { "mask" };

    // Generators, compare masks and colour selections go in masks only
    let anywhere = layer.is_some();
    let mask_only = anywhere && !content;
    let item = |icon, short, long, command, params: Value, enabled| AddItem { icon: ItemIcon::Glyph(icon), short, long, command, params, enabled };
    vec![
        item(ic::SPARKLE, n_("Generator"), n_("Add generator"), "effect.add.generator", json!({}), mask_only),
        item(ic::PAINT_BRUSH, n_("Paint"), n_("Add paint"), "effect.add.paint", json!({"target": target}), anywhere),
        item(ic::PAINT_BUCKET, n_("Fill"), n_("Add fill"), "effect.add.fill", json!({"target": target}), anywhere),
        item(ic::CHART_BAR, n_("Levels"), n_("Add levels"), "effect.add.levels", json!({"target": target}), anywhere),
        item(ic::SCALES, n_("Compare mask"), n_("Add compare mask"), "effect.add.compare", json!({}), mask_only),
        item(ic::FUNNEL, n_("Filter"), n_("Add filter"), "effect.add.filter", json!({"target": target, "filter": NEW_FILTER}), anywhere),
        item(ic::SELECTION, n_("Color selection"), n_("Add color selection"), "effect.add.colorSelection", json!({}), mask_only),
        item(ic::ANCHOR, n_("Anchor point"), n_("Add anchor point"), "effect.add.anchor", json!({"target": target}), anywhere),
    ]
}

/// The command of Add mask's Bitmap: the texture picker opens first and adds the mask with the picked texture
const BITMAP: &str = "mask.add.bitmap";

/// The Add mask items: a layer that has a mask of its own adds to it through Add effect instead
fn mask_items(layer: Option<&Layer>) -> Vec<AddItem> {
    let enabled = layer.is_some_and(|layer| layer.mask.is_none());
    let item = |icon, short, long, command| AddItem { icon, short, long, command, params: json!({}), enabled };
    vec![
        item(ItemIcon::Swatch(WHITE_SWATCH), n_("White"), n_("Add white mask"), "mask.add.white"),
        item(ItemIcon::Swatch(BLACK_SWATCH), n_("Black"), n_("Add black mask"), "mask.add.black"),
        item(ItemIcon::Glyph(ic::IMAGE), n_("Bitmap"), n_("Add bitmap mask"), BITMAP),
        item(ItemIcon::Glyph(ic::SELECTION), n_("Color selection"), n_("Add mask with color selection"), "mask.add.colorSelection"),
        item(ItemIcon::Glyph(ic::SCALES), n_("Height combination"), n_("Add mask with height combination"), "mask.add.heightCombination"),
    ]
}

/// The layer context menu's Add effect items (its own buttons, the long names); `content_thumbnail`: the menu of the layer's content thumbnail, which adds to the content
pub(crate) fn effect_menu_items(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer, content_thumbnail: bool) {
    for item in effect_items(app, Some(layer), content_thumbnail) {
        let text = format!("{}  {}", icon_of(item.icon), tr(item.long));
        if ui.add_enabled(item.enabled, egui::Button::new(text)).clicked() {
            run_on(app, ui, layer.id, item.command, item.params);
            ui.close();
        }
    }
}

/// The Add mask items of a layer row's empty mask slot (the long names); Bitmap opens the picker under the layer row
pub(crate) fn mask_slot_menu(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer, picker: Id) {
    for item in mask_items(Some(layer)) {
        if ui.add_enabled(item.enabled, egui::Button::new(tr(item.long))).clicked() {
            if item.command == BITMAP {
                resource_picker::toggle(ui.ctx(), picker);
            } else {
                run_on(app, ui, layer.id, item.command, item.params);
            }
            ui.close();
        }
    }
}

fn icon_of(icon: ItemIcon) -> &'static str {
    match icon {
        ItemIcon::Glyph(glyph) => glyph,
        ItemIcon::Swatch(_) => ic::SQUARE,
    }
}

/// The texture picker of Add bitmap mask, hanging under `anchor` at its width while open: a pick adds the mask to `layer`, Escape or a click elsewhere adds nothing
pub(crate) fn bitmap_picker(app: &mut TextureCraftApp, anchor: &Response, popup: Id, layer: LayerId) {
    let ctx = anchor.ctx.clone();
    let tabs = || bitmap_tabs(&app.session);
    match resource_picker::show(anchor, popup, &mut app.thumbs, None, false, tabs) {
        Some(BitmapChoice::Import) => crate::menus::invoke(app, &ctx, "resource.import"),
        Some(BitmapChoice::Image(name)) => {
            app.run("layer.select", json!({"layer": layer.0}));
            app.run(BITMAP, json!({"layer": layer.0, "image": name}));
        }
        Some(BitmapChoice::Alpha(alpha)) => {
            app.run("layer.select", json!({"layer": layer.0}));
            app.run(BITMAP, json!({"layer": layer.0, "alpha": alpha}));
        }
        None => {}
    }
}

/// The bitmap picker's one tab: Textures (Import... first) and the Alphas in their shelf groups
fn bitmap_tabs(session: &Session) -> Vec<PickerTab<BitmapChoice>> {
    let tile = |id, label: &str, caption, picture| ResourcePickerItem { id, label: label.to_owned(), caption, picture };
    let textures = ResourcePickerItem::textures(session, BitmapChoice::Import, |name| BitmapChoice::Image(name.to_owned()));
    let alphas = alphas::library()
        .iter()
        .map(|alpha| tile(BitmapChoice::Alpha(alpha.id.as_str()), tr(&alpha.label), Some(tr(alpha.category)), TilePicture::Alpha(alpha)))
        .collect();
    let groups = vec![PickerGroup { header: Some(tr("Textures")), items: textures }, PickerGroup { header: Some(tr("Alphas")), items: alphas }];
    vec![PickerTab { icon: ic::SQUARES_FOUR, name: tr("Resources"), groups }]
}
