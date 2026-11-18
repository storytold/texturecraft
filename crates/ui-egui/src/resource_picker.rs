//! A popup under a slot that picks a resource, such as a filter, from grouped shelf tiles searched by name; a fill source's picker adds a second tab with the anchor points it can read

use egui::{Id, Response, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use texturecraft_engine::core::LayerId;

use crate::i18n::tr;
use crate::resource_picker_anchor::ResourcePickerAnchor;
use crate::resource_picker_choice::ResourcePickerChoice;
use crate::resource_picker_item::ResourcePickerItem;
use crate::resource_picker_tab::ResourcePickerTab;
use crate::resource_picker_tile::ResourcePickerTile;
use crate::shelf;
use crate::shelf_preview::ShelfPreview;
use crate::theme::Tokens;
use crate::thumbs::Thumbs;

/// Four shelf tiles and the scrollbar
const PICKER_WIDTH: f32 = 320.0;
/// The grid's height, fixed so the popup keeps its place while searching
const PICKER_HEIGHT: f32 = 340.0;
const CURRENT_OUTLINE_WIDTH: f32 = 1.5;
const TILE_ROUNDING: f32 = 3.0;
/// Group headers read as the shelf's section titles, a size above the tile labels
const GROUP_FONT_SIZE: f32 = 13.0;
const GROUP_GAP: f32 = 2.0;

/// The tab strip along the popup's top edge: its height, the tabs' padding, rounding and the gap between them
const TAB_STRIP_HEIGHT: f32 = 28.0;
const TAB_PADDING: f32 = 10.0;
const TAB_ICON_GAP: f32 = 6.0;
const TAB_ROUNDING: f32 = 4.0;
const TAB_GAP: f32 = 4.0;
const TAB_INSET: f32 = 4.0;
const TAB_FONT_SIZE: f32 = 12.0;
/// How much darker the strip is than the popup body
const TAB_STRIP_DARKEN: f32 = 0.2;

/// Anchor tiles: the shelf's tile size, the picture's size and place, the anchor badge in its corner and the channel line's size
const ANCHOR_TILE_SIZE: egui::Vec2 = vec2(72.0, 86.0);
const ANCHOR_PICTURE_SIZE: f32 = 56.0;
const ANCHOR_PICTURE_TOP: f32 = 4.0;
const ANCHOR_BADGE_SIZE: f32 = 13.0;
const ANCHOR_BADGE_INSET: f32 = 2.0;
const ANCHOR_BADGE_ICON_SIZE: f32 = 10.0;
/// Space kept clear at each side of an anchor tile's name
const ANCHOR_LABEL_INSET: f32 = 3.0;
const ANCHOR_LABEL_FONT_SIZE: f32 = 9.5;
const ANCHOR_LABEL_GAP: f32 = 3.0;
const ANCHOR_LINE_HEIGHT: f32 = 11.0;

/// Opens the picker `popup_id` with an empty, focused search field, or closes it
pub(crate) fn toggle(ctx: &egui::Context, popup_id: Id) {
    if egui::Popup::is_id_open(ctx, popup_id) {
        egui::Popup::close_id(ctx, popup_id);
        return;
    }

    egui::Popup::open_id(ctx, popup_id);
    ctx.data_mut(|data| {
        data.insert_temp(search_id(popup_id), String::new());
        data.insert_temp(focus_id(popup_id), true);
    });
}

/// Whether picker `popup_id` is open: its tiles are worth gathering only then
pub(crate) fn is_open(ctx: &egui::Context, popup_id: Id) -> bool {
    egui::Popup::is_id_open(ctx, popup_id)
}

/// Draws picker `popup_id` at `slot` while open and returns the id clicked (`items` runs only then)
pub(crate) fn show(
    slot: &Response,
    popup_id: Id,
    thumbs: &mut Thumbs,
    current: Option<&str>,
    items: impl FnOnce(&mut Thumbs) -> Vec<ResourcePickerItem>,
    preview: impl Fn(&'static str) -> ShelfPreview,
) -> Option<&'static str> {
    if !is_open(&slot.ctx, popup_id) {
        return None;
    }

    let items = items(thumbs);
    let tiles = items
        .iter()
        .map(|item| {
            let id = item.id;
            let preview = &preview;
            ResourcePickerTile {
                key: id.to_string(),
                label: item.label.to_string(),
                group: item.group,
                thumbnail: item.thumbnail.clone(),
                image: None,
                preview: Box::new(move || preview(id)),
            }
        })
        .collect();
    match popup(slot, popup_id, thumbs, (current, None), tiles, None, |_, _| None) {
        Some(ResourcePickerChoice::Resource(key)) => items.iter().find(|item| item.id == key).map(|item| item.id),
        _ => None,
    }
}

/// Draws fill source picker `popup_id` at `slot` while open, with its Resources and Anchor points tabs: returns the tile clicked; `current` is the key of the source shown, `current_anchor` the anchor it reads (its tab opens first)
pub(crate) fn show_sources(
    slot: &Response,
    popup_id: Id,
    thumbs: &mut Thumbs,
    (current, current_anchor): (Option<&str>, Option<LayerId>),
    tiles: Vec<ResourcePickerTile<'_>>,
    anchors: Vec<ResourcePickerAnchor>,
    picture: impl FnMut(&Ui, LayerId) -> Option<egui::TextureId>,
) -> Option<ResourcePickerChoice> {
    if !is_open(&slot.ctx, popup_id) {
        return None;
    }

    popup(slot, popup_id, thumbs, (current, current_anchor), tiles, Some(anchors), picture)
}

/// The popup: the tabs when there are anchors to offer, the search field, then the tiles of the shown tab
fn popup(
    slot: &Response,
    popup_id: Id,
    thumbs: &mut Thumbs,
    (current, current_anchor): (Option<&str>, Option<LayerId>),
    tiles: Vec<ResourcePickerTile<'_>>,
    anchors: Option<Vec<ResourcePickerAnchor>>,
    mut picture: impl FnMut(&Ui, LayerId) -> Option<egui::TextureId>,
) -> Option<ResourcePickerChoice> {
    let ctx = slot.ctx.clone();
    let mut picked = None;
    egui::Popup::from_response(slot)
        .id(popup_id)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .align(egui::RectAlign::BOTTOM_END)
        .width(PICKER_WIDTH)
        .show(|ui| {
            ui.set_width(PICKER_WIDTH);
            let just_opened = ui.data_mut(|data| data.remove_temp::<bool>(focus_id(popup_id))).unwrap_or(false);
            let tab = match &anchors {
                Some(_) => tab_strip(ui, popup_id, just_opened && current_anchor.is_some()),
                None => ResourcePickerTab::Resources,
            };
            let query = search_field(ui, popup_id, just_opened);
            egui::ScrollArea::vertical().max_height(PICKER_HEIGHT).show(ui, |ui| {
                // A search that leaves few tiles must not shrink the popup, or egui moves it
                ui.set_min_height(PICKER_HEIGHT);
                picked = match (tab, &anchors) {
                    (ResourcePickerTab::Anchors, Some(anchors)) => {
                        anchor_grid(ui, anchors, &query, current_anchor, &mut picture).map(ResourcePickerChoice::Anchor)
                    }
                    _ => grid(ui, thumbs, &tiles, &query, current, just_opened).map(ResourcePickerChoice::Resource),
                };
            });
        });

    if picked.is_some() {
        egui::Popup::close_id(&ctx, popup_id);
    }
    picked
}

/// The tabs on the popup's top edge: a darker strip, the shown tab in the body's color so it joins the list below. Returns the shown tab
fn tab_strip(ui: &mut Ui, popup_id: Id, open_anchors: bool) -> ResourcePickerTab {
    let tokens = Tokens::get(ui.ctx());
    let tab_id = popup_id.with("tab");
    let mut shown = if open_anchors {
        ResourcePickerTab::Anchors
    } else {
        ui.data(|data| data.get_temp::<ResourcePickerTab>(tab_id)).unwrap_or(ResourcePickerTab::Resources)
    };

    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), TAB_STRIP_HEIGHT), Sense::hover());
    let body = ui.visuals().window_fill;
    ui.painter().rect_filled(strip, 0.0, body.lerp_to_gamma(egui::Color32::BLACK, TAB_STRIP_DARKEN));
    let font = egui::FontId::proportional(TAB_FONT_SIZE);
    let mut left = strip.left() + TAB_INSET;
    for (tab, icon, label) in [(ResourcePickerTab::Resources, ic::SQUARES_FOUR, tr("Resources")), (ResourcePickerTab::Anchors, ic::ANCHOR, tr("Anchor points"))]
    {
        let icon_width = ui.painter().layout_no_wrap(icon.to_string(), font.clone(), tokens.text).size().x;
        let label_width = ui.painter().layout_no_wrap(label.to_string(), font.clone(), tokens.text).size().x;
        let width = TAB_PADDING * 2.0 + icon_width + TAB_ICON_GAP + label_width;
        let rect = egui::Rect::from_min_max(pos2(left, strip.top() + TAB_INSET), pos2(left + width, strip.bottom()));
        let response = ui.interact(rect, popup_id.with(label), Sense::click());
        if response.clicked() {
            shown = tab;
        }
        let on = shown == tab;
        if on {
            ui.painter().rect_filled(rect, egui::CornerRadius { nw: TAB_ROUNDING as u8, ne: TAB_ROUNDING as u8, sw: 0, se: 0 }, body);
        }
        let color = if on {
            tokens.text_strong
        } else if response.hovered() {
            tokens.text
        } else {
            tokens.text_dim
        };
        let painter = ui.painter();
        painter.text(pos2(rect.left() + TAB_PADDING, rect.center().y), egui::Align2::LEFT_CENTER, icon, font.clone(), color);
        painter.text(pos2(rect.left() + TAB_PADDING + icon_width + TAB_ICON_GAP, rect.center().y), egui::Align2::LEFT_CENTER, label, font.clone(), color);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, label));
        left += width + TAB_GAP;
    }
    ui.data_mut(|data| data.insert_temp(tab_id, shown));
    shown
}

/// The search field, focused the frame the picker opens; returns the lowercase query
fn search_field(ui: &mut Ui, popup_id: Id, just_opened: bool) -> String {
    let mut search: String = ui.data(|data| data.get_temp(search_id(popup_id))).unwrap_or_default();
    let field = ui.add(egui::TextEdit::singleline(&mut search).hint_text(tr("Search")).desired_width(f32::INFINITY));
    if just_opened {
        field.request_focus();
    }

    let query = search.trim().to_lowercase();
    ui.data_mut(|data| data.insert_temp(search_id(popup_id), search));
    query
}

/// A group's title over its tiles, as the shelf's section titles
fn group_header(ui: &mut Ui, text: &str) {
    let tokens = Tokens::get(ui.ctx());
    ui.add_space(GROUP_GAP);
    ui.label(egui::RichText::new(text).size(GROUP_FONT_SIZE).color(tokens.text));
}

/// The tiles matching `query` under group headers, `current` outlined; returns the key of the one clicked
fn grid(ui: &mut Ui, thumbs: &mut Thumbs, tiles: &[ResourcePickerTile<'_>], query: &str, current: Option<&str>, reveal_current: bool) -> Option<String> {
    let tokens = Tokens::get(ui.ctx());
    let shown: Vec<&ResourcePickerTile<'_>> = tiles.iter().filter(|tile| shelf::matches(query, &[&tile.label, &tile.key])).collect();
    let mut picked = None;
    for group in shelf::categories(&shown, |tile| tile.group) {
        group_header(ui, group);
        shelf::tiles(ui, |ui| {
            for tile in shown.iter().filter(|tile| tile.group == group) {
                let response = shelf::tile(
                    ui,
                    thumbs,
                    |ui, thumbs| tile.image.as_ref().map(|image| thumbs.image(ui.ctx(), &tile.label, image)).or_else(|| tile.thumbnail.clone()),
                    || (tile.preview)(),
                    &tile.label,
                    None,
                );
                if current == Some(tile.key.as_str()) {
                    if reveal_current {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    ui.painter().rect_stroke(response.rect, TILE_ROUNDING, Stroke::new(CURRENT_OUTLINE_WIDTH, tokens.accent), egui::StrokeKind::Inside);
                }
                if response.clicked() {
                    picked = Some(tile.key.clone());
                }
            }
        });
    }
    picked
}

/// The anchor points matching `query` as tiles: a picture of what each reads, an anchor badge, the layer and the channel
fn anchor_grid(
    ui: &mut Ui,
    anchors: &[ResourcePickerAnchor],
    query: &str,
    current: Option<LayerId>,
    picture: &mut impl FnMut(&Ui, LayerId) -> Option<egui::TextureId>,
) -> Option<LayerId> {
    let tokens = Tokens::get(ui.ctx());
    if anchors.is_empty() {
        ui.label(egui::RichText::new(tr("No anchor point below this layer.")).color(tokens.text_dim));
        return None;
    }

    let shown: Vec<&ResourcePickerAnchor> = anchors.iter().filter(|anchor| shelf::matches(query, &[&anchor.name, tr(anchor.channel)])).collect();
    let mut picked = None;
    shelf::tiles(ui, |ui| {
        for anchor in shown {
            let response = anchor_tile(ui, anchor, picture);
            if current == Some(anchor.anchor) {
                ui.painter().rect_stroke(response.rect, TILE_ROUNDING, Stroke::new(CURRENT_OUTLINE_WIDTH, tokens.accent), egui::StrokeKind::Inside);
            }
            if response.clicked() {
                picked = Some(anchor.anchor);
            }
        }
    });
    picked
}

/// One anchor tile, the shelf tile's size: the picture, the anchor badge in its corner, the layer's name and the channel under it
fn anchor_tile(ui: &mut Ui, anchor: &ResourcePickerAnchor, picture: &mut impl FnMut(&Ui, LayerId) -> Option<egui::TextureId>) -> Response {
    let tokens = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(ANCHOR_TILE_SIZE, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &anchor.name));

    // Offscreen tiles must not queue composition of their anchor stacks
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let texture = picture(ui, anchor.anchor);
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, TILE_ROUNDING, tokens.row_hover);
    }
    let picture = egui::Rect::from_min_size(
        pos2(rect.center().x - ANCHOR_PICTURE_SIZE * 0.5, rect.top() + ANCHOR_PICTURE_TOP),
        vec2(ANCHOR_PICTURE_SIZE, ANCHOR_PICTURE_SIZE),
    );
    painter.rect_filled(picture, TILE_ROUNDING, tokens.field);
    if let Some(texture) = texture {
        painter.image(texture, picture, egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    let badge = egui::Rect::from_min_size(
        picture.right_bottom() - vec2(ANCHOR_BADGE_SIZE + ANCHOR_BADGE_INSET, ANCHOR_BADGE_SIZE + ANCHOR_BADGE_INSET),
        vec2(ANCHOR_BADGE_SIZE, ANCHOR_BADGE_SIZE),
    );
    painter.rect_filled(badge, TILE_ROUNDING, tokens.panel);
    painter.text(badge.center(), egui::Align2::CENTER_CENTER, ic::ANCHOR, egui::FontId::proportional(ANCHOR_BADGE_ICON_SIZE), tokens.text);

    let font = egui::FontId::proportional(ANCHOR_LABEL_FONT_SIZE);
    let width = rect.width() - ANCHOR_LABEL_INSET * 2.0;
    let name = crate::widgets::elided(painter, anchor.layer.clone(), ANCHOR_LABEL_FONT_SIZE, tokens.text, width);
    let top = picture.bottom() + ANCHOR_LABEL_GAP;
    painter.galley(pos2(rect.center().x - name.size().x * 0.5, top), name, tokens.text);
    painter.text(pos2(rect.center().x, top + ANCHOR_LINE_HEIGHT), egui::Align2::CENTER_TOP, tr(anchor.channel), font, tokens.text_dim);
    response.on_hover_text(&anchor.name)
}

/// Where the picker keeps its search text
fn search_id(popup_id: Id) -> Id {
    popup_id.with("search")
}

/// Set while the picker waits for its first frame open
fn focus_id(popup_id: Id) -> Id {
    popup_id.with("focus")
}
