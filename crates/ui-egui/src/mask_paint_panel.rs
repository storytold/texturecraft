//! The paint tools' Grayscale tab while a mask is painted: the GRAYSCALE section with the slot of the gray a stroke lays down

use egui::Ui;
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::Session;
use texturecraft_engine::core::{NormalFormat, Source};

use crate::TextureCraftApp;
use crate::fill_material_panel::{shown_color, source_line};
use crate::i18n::{n_, tr};
use crate::mask_source_choice::MaskSourceChoice;
use crate::picker_group::PickerGroup;
use crate::picker_tab::PickerTab;
use crate::property_sheet::PropertySheet;
use crate::resource_picker;
use crate::resource_picker_item::ResourcePickerItem;
use crate::slot_thumbnail::{paint_thumbnail, thumbnail};
use crate::theme::Tokens;
use crate::tile_picture::TilePicture;

/// The GRAYSCALE section: a slot card for one gray (with its ramp and invert) or an image whose gray is painted
pub(crate) fn grayscale_sheet(app: &mut TextureCraftApp, ui: &mut Ui) {
    PropertySheet::show(ui, "mask_paint_sheet", |sheet| {
        sheet.section(tr("Grayscale"), false);
        let gray = app.session.tool.mask_value;
        let source = match &app.session.tool.mask_image {
            Some(name) => Source::Image { name: name.clone(), normal_format: NormalFormat::default() },
            None => Source::Uniform([gray; 3]),
        };
        let tokens = Tokens::get(sheet.ui.ctx());
        let picture = thumbnail(app, &sheet.ui.ctx().clone(), &source);
        let line = source_line(app, &source);
        let image = app.session.tool.mask_image.is_some();
        let slot = sheet.slot(&|painter, rect| paint_thumbnail(painter, rect, &picture, &tokens), tr("Grayscale"), &line, None, image, false);

        let popup = sheet.ui.make_persistent_id("mask_paint_source");
        if slot.pick.clicked() {
            resource_picker::toggle(sheet.ui.ctx(), popup);
        }
        let current = match &app.session.tool.mask_image {
            Some(name) => MaskSourceChoice::Image(name.clone()),
            None => MaskSourceChoice::Uniform,
        };
        let ctx = sheet.ui.ctx().clone();
        let tabs = || source_tabs(&app.session, shown_color([gray; 3]));
        match resource_picker::show(&slot.pick, popup, &mut app.thumbs, Some(&current), image, tabs) {
            Some(MaskSourceChoice::Import) => crate::menus::invoke(app, &ctx, "resource.import"),
            Some(MaskSourceChoice::Image(name)) if app.session.tool.mask_image.as_deref() != Some(name.as_str()) => {
                app.run("tool.material", json!({"maskImage": name}));
            }
            Some(MaskSourceChoice::Uniform) if image => {
                app.run("tool.material", json!({"maskImage": null}));
            }
            _ => {}
        }
        if slot.clear {
            app.run("tool.material", json!({"maskImage": null}));
        }
        if let Some(name) = crate::dock::image_drop(sheet.ui, &slot.pick) {
            app.run("tool.material", json!({"maskImage": name}));
        }

        if !image {
            let mut value = gray;
            let edit = sheet.ramp(tr("Grayscale"), &mut value, 0.0..=1.0);
            if edit.invert {
                app.run("tool.material", json!({"maskValue": 1.0 - gray}));
            } else if edit.changed.is_some() {
                app.run("tool.material", json!({"maskValue": value}));
            }
        }
        sheet.close_card();
    });
}

/// The picker's one Resources tab: Textures (Import... first, then the project's images) and Uniform
fn source_tabs(session: &Session, uniform: egui::Color32) -> Vec<PickerTab<MaskSourceChoice>> {
    let textures = ResourcePickerItem::textures(session, MaskSourceChoice::Import, |name| MaskSourceChoice::Image(name.to_owned()));
    let uniform =
        vec![ResourcePickerItem { id: MaskSourceChoice::Uniform, label: tr("Uniform value").to_owned(), caption: None, picture: TilePicture::Swatch(uniform) }];
    let groups =
        [(n_("Textures"), textures), (n_("Uniform"), uniform)].into_iter().map(|(header, items)| PickerGroup { header: Some(tr(header)), items }).collect();
    vec![PickerTab { icon: ic::SQUARES_FOUR, name: tr("Resources"), groups }]
}
