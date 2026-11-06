//! The filter slot atop a filter's properties: thumbnail and name, opening a picker that swaps it

use egui::{Align2, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::core::{Filter, LayerId};
use texturecraft_engine::filter_lib::{FILTERS, find};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::resource_picker;
use crate::resource_picker_item::ResourcePickerItem;
use crate::shelf_preview::ShelfPreview;
use crate::theme::Tokens;
use crate::thumbs::Thumbs;

const SLOT_HEIGHT: f32 = 52.0;
const SLOT_ROUNDING: f32 = 3.0;
const THUMBNAIL_SIZE: f32 = 40.0;
const THUMBNAIL_INSET: f32 = 6.0;
const CAPTION_FONT_SIZE: f32 = 11.0;
const NAME_FONT_SIZE: f32 = 13.0;
/// Gap between the caption's and the name's centers
const LINE_GAP: f32 = 15.0;
const CARET_INSET: f32 = 14.0;
const CARET_FONT_SIZE: f32 = 14.0;

/// The slot of filter effect `effect`: a pick runs `effect.set {filter}`, masks offer only mask filters
pub(crate) fn show(app: &mut TextureCraftApp, ui: &mut Ui, effect: u64, filter: &Filter) {
    let ctx = ui.ctx().clone();
    let on_mask = is_on_mask(app, effect);
    let current = find(filter.label()).map(|def| def.id);
    let thumbnail = current.and_then(|id| app.thumbs.filter(&ctx, id));
    let name = tr(filter.label());

    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), SLOT_HEIGHT), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
    paint(ui, rect, response.hovered(), thumbnail.as_ref(), name);

    let popup = ui.make_persistent_id(("filter_picker", effect));
    if response.clicked() {
        resource_picker::toggle(&ctx, popup);
    }

    let items = |thumbs: &mut Thumbs| {
        FILTERS
            .iter()
            .filter(|def| def.is_filter_effect() && (def.mask || !on_mask))
            .map(|def| ResourcePickerItem { id: def.id, label: tr(def.label), group: tr(def.group), thumbnail: thumbs.filter(&ctx, def.id) })
            .collect()
    };
    let picked = resource_picker::show(&response, popup, &mut app.thumbs, current, items, ShelfPreview::Filter);
    if let Some(id) = picked.filter(|id| Some(*id) != current) {
        app.run("effect.set", json!({"effect": effect, "filter": id}));
    }
}

/// Whether filter effect `effect` sits in a mask rather than the layer content
fn is_on_mask(app: &TextureCraftApp, effect: u64) -> bool {
    let project = &app.session.doc.project;
    let owner = project.effect(LayerId(effect)).and_then(|(owner, _)| project.layers.get(&owner));
    owner.and_then(|layer| layer.mask.as_ref()).is_some_and(|mask| mask.effects.iter().any(|masked| masked.id.0 == effect))
}

/// The slot's box: thumbnail, "Filter" over the name, caret
fn paint(ui: &Ui, rect: Rect, hovered: bool, thumbnail: Option<&egui::TextureHandle>, name: &str) {
    let tk = Tokens::get(ui.ctx());
    let painter = ui.painter();
    let (fill, border) = if hovered { (tk.row_hover, tk.button_border) } else { (tk.field, tk.field_border) };
    painter.rect(rect, SLOT_ROUNDING, fill, Stroke::new(1.0, border), egui::StrokeKind::Inside);

    if let Some(texture) = thumbnail {
        let image = Rect::from_min_size(pos2(rect.left() + THUMBNAIL_INSET, rect.center().y - THUMBNAIL_SIZE * 0.5), vec2(THUMBNAIL_SIZE, THUMBNAIL_SIZE));
        painter.image(texture.id(), image, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
    }

    let center = rect.center();
    painter.text(center - vec2(0.0, LINE_GAP * 0.5), Align2::CENTER_CENTER, tr("Filter"), FontId::proportional(CAPTION_FONT_SIZE), tk.text_dim);
    painter.text(center + vec2(0.0, LINE_GAP * 0.5), Align2::CENTER_CENTER, name, FontId::proportional(NAME_FONT_SIZE), tk.text_strong);

    let caret_color = if hovered { tk.text } else { tk.text_dim };
    painter.text(pos2(rect.right() - CARET_INSET, center.y), Align2::CENTER_CENTER, ic::CARET_DOWN, FontId::proportional(CARET_FONT_SIZE), caret_color);
}
