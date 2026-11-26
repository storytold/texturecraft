//! Display Settings' Environment Map: a drop slot that shows the environment's name and opens a grid of environment thumbnails with a search

use egui::{Sense, Stroke, Ui};
use egui_phosphor::regular as ic;
use texturecraft_engine::core::project::{DisplaySettings, ENV_RESOURCE_PREFIX};
use texturecraft_render::ibl::ENVIRONMENTS;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::query_field;
use crate::shelf_preview::ShelfPreview;
use crate::theme::Tokens;

/// What the picker's query starts with (and returns to)
const ENVIRONMENT_QUERY: &str = "environment ";

/// Size of the picker's popup
const PICKER_WIDTH: f32 = 290.0;
const PICKER_HEIGHT: f32 = 270.0;

/// Outline of the environment in use
const CURRENT_OUTLINE_WIDTH: f32 = 1.5;
const TILE_ROUNDING: f32 = 3.0;

/// The label, the slot and the picker; true when Import HDR was picked
pub(crate) fn show(app: &mut TextureCraftApp, ui: &mut Ui, display: &mut DisplaySettings) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let shown = display.environment_resource().unwrap_or(&display.environment).to_owned();
    let mut import_hdr = false;
    ui.horizontal(|ui| {
        ui.label(tr("Environment Map"));
        let size = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        let (rect, slot) = ui.allocate_exact_size(size, Sense::click());
        slot.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &shown));
        let fill = if slot.hovered() { tokens.row_hover } else { tokens.field };
        ui.painter().rect(rect, TILE_ROUNDING, fill, Stroke::new(1.0, tokens.field_border), egui::StrokeKind::Inside);
        let galley = crate::widgets::elided(ui.painter(), shown.clone(), 13.0, tokens.text, rect.width() - 12.0);
        ui.painter().galley(egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y * 0.5), galley, tokens.text);
        let slot = slot.on_hover_text(&shown);

        // An image dragged from the Assets panel becomes the environment
        if let Some(name) = crate::dock::image_drop(ui, &slot)
            && app.session.images.contains_key(&name)
        {
            display.environment = format!("{ENV_RESOURCE_PREFIX}{name}");
        }

        let popup_id = ui.make_persistent_id("environment_picker");
        if slot.clicked() {
            if !egui::Popup::is_id_open(ui.ctx(), popup_id) {
                query_field::reset(ui.ctx(), popup_id.with("query"));
            }
            egui::Popup::toggle_id(ui.ctx(), popup_id);
        }
        egui::Popup::from_response(&slot)
            .id(popup_id)
            .open_memory(None)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .align(egui::RectAlign::BOTTOM_START)
            .width(PICKER_WIDTH)
            .show(|ui| {
                ui.set_width(PICKER_WIDTH);
                import_hdr = grid(app, ui, display, popup_id);
            });
    });
    import_hdr
}

/// The query field and the environments matching it as thumbnails; Import HDR under them
fn grid(app: &mut TextureCraftApp, ui: &mut Ui, display: &mut DisplaySettings, popup_id: egui::Id) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let words = query_field::words(&query_field::show(ui, popup_id.with("query"), ENVIRONMENT_QUERY), ENVIRONMENT_QUERY);
    let mut hdrs: Vec<String> = app.session.images.iter().filter(|(_, image)| !image.srgb).map(|(name, _)| name.clone()).collect();
    hdrs.sort();
    let mut picked = None;
    egui::ScrollArea::vertical().max_height(PICKER_HEIGHT).show(ui, |ui| {
        ui.set_min_height(PICKER_HEIGHT);
        crate::shelf::tiles(ui, |ui| {
            for name in ENVIRONMENTS.into_iter().filter(|name| query_field::matches(&words, name)) {
                let response =
                    crate::shelf::tile(ui, &mut app.thumbs, |ui, thumbs| thumbs.environment(ui.ctx(), name), || ShelfPreview::Environment(name), name, None);
                if display.environment == name {
                    ui.painter().rect_stroke(response.rect, TILE_ROUNDING, Stroke::new(CURRENT_OUTLINE_WIDTH, tokens.accent), egui::StrokeKind::Inside);
                }
                if response.clicked() {
                    picked = Some(name.to_owned());
                }
            }
            for name in hdrs.iter().filter(|name| query_field::matches(&words, name)) {
                let Some(image) = app.session.images.get(name).cloned() else { continue };
                let response = crate::shelf::tile(
                    ui,
                    &mut app.thumbs,
                    |ui, thumbs| Some(thumbs.image(ui.ctx(), name, &image)),
                    || ShelfPreview::Image(name.clone(), image.clone()),
                    name,
                    None,
                );
                let key = format!("{ENV_RESOURCE_PREFIX}{name}");
                if display.environment == key {
                    ui.painter().rect_stroke(response.rect, TILE_ROUNDING, Stroke::new(CURRENT_OUTLINE_WIDTH, tokens.accent), egui::StrokeKind::Inside);
                }
                if response.clicked() {
                    picked = Some(key);
                }
            }
        });
    });
    if let Some(environment) = picked {
        display.environment = environment;
        egui::Popup::close_id(ui.ctx(), popup_id);
    }

    let import = ui
        .selectable_label(false, format!("{} {}", ic::FILE_ARROW_UP, tr("Import HDR...")))
        .on_hover_text(tr("Use an equirectangular .hdr or .exr image as the environment"))
        .clicked();
    if import {
        egui::Popup::close_id(ui.ctx(), popup_id);
    }
    import
}
