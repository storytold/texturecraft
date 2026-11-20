//! The view-mode picker of a view's header and of Display Settings: the lit material, one channel, the mesh maps, or any image of the project ("Other...")

use egui::{RichText, Ui};
use serde_json::json;
use texturecraft_engine::ViewMode;
use texturecraft_engine::core::{BakedMap, Channel};

use crate::TextureCraftApp;
use crate::i18n::{n_, tr, trf};
use crate::query_field;
use crate::shelf_preview::ShelfPreview;
use crate::theme::Tokens;

/// The picker's "Other..." entry
const OTHER: &str = "Other...";

/// What the image picker's query starts with (and returns to)
const IMAGE_QUERY: &str = "u:image PRs";

/// The most the list of modes grows before it scrolls (it holds three titles and twenty entries)
const LIST_HEIGHT: f32 = 800.0;

/// Size of the image picker's popup
const IMAGE_PICKER_WIDTH: f32 = 290.0;
const IMAGE_PICKER_HEIGHT: f32 = 250.0;

/// A channel's name in this picker, which follows the reference's shader naming; other places keep the channel's own name
pub(crate) fn channel_label(channel: Channel) -> &'static str {
    match channel {
        Channel::Metallic => n_("Base metalness"),
        Channel::Roughness => n_("Specular roughness"),
        other => other.label(),
    }
}

/// A mesh map's name in this picker (the bake panels spell them with capitals)
pub(crate) fn map_label(map: BakedMap) -> &'static str {
    match map {
        BakedMap::Normal => n_("Normal"),
        BakedMap::WorldNormal => n_("World space normal"),
        BakedMap::Id => n_("ID"),
        BakedMap::AmbientOcclusion => n_("Ambient occlusion"),
        BakedMap::Curvature => n_("Curvature"),
        BakedMap::Position => n_("Position"),
        BakedMap::Thickness => n_("Thickness"),
        BakedMap::Height => n_("Height"),
        BakedMap::BentNormal => n_("Bent normals"),
        BakedMap::Opacity => n_("Opacity"),
    }
}

/// An italic title over a group of entries: it cannot be picked
fn section(ui: &mut Ui, title: &str) {
    let tokens = Tokens::get(ui.ctx());
    ui.label(RichText::new(tr(title)).italics().color(tokens.text_dim));
}

/// The picker: a combo of the view modes under their section titles. In Bake mode it offers the material and the mesh maps only.
pub(crate) fn show(app: &mut TextureCraftApp, child: &mut Ui, id_salt: &str, width: f32) {
    let project_open = app.session.project_open;

    // Bake mode lights the project mesh with the neutral material unless the painted one is asked for
    let baking = app.ui.mode == crate::state::Mode::Baking;
    let neutral = baking && app.session.view == ViewMode::Material && app.ui.bake_visualization.neutral_shown;
    let label = match app.session.view {
        _ if neutral => tr("Neutral Material").to_owned(),
        ViewMode::MeshMap(map) if baking => tr(crate::i18n::baked_map_label(map)).to_owned(),
        ViewMode::MeshMap(map) => tr(map_label(map)).to_owned(),
        ViewMode::Channel(channel) => tr(channel_label(channel)).to_owned(),
        ViewMode::Image => app.session.view_image.clone().unwrap_or_else(|| tr(OTHER).to_owned()),
        _ => tr(&app.session.view.label()).to_owned(),
    };
    let channels = app.session.doc.project.texture_set(app.session.ts()).map(|texture_set| texture_set.channel_list()).unwrap_or_default();
    let popup_id = child.make_persistent_id((id_salt, "other"));

    // The picker stays greyed without project maps
    let combo = child
        .add_enabled_ui(project_open, |child| {
            egui::ComboBox::from_id_salt(id_salt).width(width).height(LIST_HEIGHT).selected_text(label).show_ui(child, |ui| {
                // Bake mode keeps its own list: the material, then the mesh maps, without titles
                if !baking {
                    section(ui, "Lighting");
                }
                if baking && ui.selectable_label(neutral, tr("Neutral Material")).clicked() {
                    app.ui.bake_visualization.neutral_shown = true;
                    app.run("viewport.view", json!({"mode": "material"}));
                }
                if ui.selectable_label(app.session.view == ViewMode::Material && !neutral, tr("Material")).clicked() && project_open {
                    // Keep the bake material choice when switching to Material in Painting mode
                    app.ui.bake_visualization.neutral_shown &= !baking;
                    app.run("viewport.view", json!({"mode": "material"}));
                }

                // Nothing else to show without a project
                if !project_open {
                    return;
                }

                if baking {
                    ui.separator();
                    for map in BakedMap::ALL {
                        let label = trf("Mesh map: {map}", &[("map", &tr(crate::i18n::baked_map_label(map)))]);
                        if ui.selectable_label(app.session.view == ViewMode::MeshMap(map), label).clicked() {
                            app.run("viewport.view", json!({"mode": format!("map:{}", map.id())}));
                        }
                    }
                    return;
                }

                if ui.selectable_label(app.session.view == ViewMode::UvChecker, tr("UV checker")).clicked() {
                    app.run("viewport.view", json!({"mode": "uvChecker"}));
                }

                section(ui, "Single channel");
                for channel in channels {
                    if ui.selectable_label(app.session.view == ViewMode::Channel(channel), tr(channel_label(channel))).clicked() {
                        app.run("viewport.view", json!({"mode": channel.id()}));
                    }
                }
                if ui.selectable_label(app.session.view == ViewMode::NormalHeightMesh, tr(texturecraft_engine::NORMAL_HEIGHT_MESH_LABEL)).clicked() {
                    app.run("viewport.view", json!({"mode": "normalHeightMesh"}));
                }

                section(ui, "Mesh maps");
                for map in BakedMap::ALL {
                    if ui.selectable_label(app.session.view == ViewMode::MeshMap(map), tr(map_label(map))).clicked() {
                        app.run("viewport.view", json!({"mode": format!("map:{}", map.id())}));
                    }
                }
                if ui.selectable_label(app.session.view == ViewMode::Image, tr(OTHER)).clicked() {
                    query_field::reset(ui.ctx(), popup_id.with("query"));
                    ui.data_mut(|data| data.insert_temp(popup_id, true));
                }
            })
        })
        .inner;

    image_picker(app, &combo.response, popup_id);
}

/// The popup under the combo that picks a project image to show: a query field and the images as tiles
fn image_picker(app: &mut TextureCraftApp, combo: &egui::Response, popup_id: egui::Id) {
    let ctx = combo.ctx.clone();
    let mut open = ctx.data(|data| data.get_temp::<bool>(popup_id)).unwrap_or(false);
    if !open {
        return;
    }

    let mut picked = None;
    egui::Popup::from_response(combo)
        .id(popup_id)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .align(egui::RectAlign::BOTTOM_START)
        .width(IMAGE_PICKER_WIDTH)
        .show(|ui| {
            ui.set_width(IMAGE_PICKER_WIDTH);
            let words = query_field::words(&query_field::show(ui, popup_id.with("query"), IMAGE_QUERY), IMAGE_QUERY);
            let images: Vec<String> =
                app.session.doc.resources.keys().filter(|name| app.session.images.contains_key(*name) && query_field::matches(&words, name)).cloned().collect();
            egui::ScrollArea::vertical().max_height(IMAGE_PICKER_HEIGHT).show(ui, |ui| {
                ui.set_min_height(IMAGE_PICKER_HEIGHT);
                crate::shelf::tiles(ui, |ui| {
                    for name in &images {
                        let Some(image) = app.session.images.get(name).cloned() else { continue };
                        let response = crate::shelf::tile(
                            ui,
                            &mut app.thumbs,
                            |ui, thumbs| Some(thumbs.image(ui.ctx(), name, &image)),
                            || ShelfPreview::Image(name.clone(), image.clone()),
                            name,
                            None,
                        );
                        if response.clicked() {
                            picked = Some(name.clone());
                        }
                    }
                });
            });
        });

    ctx.data_mut(|data| data.insert_temp(popup_id, open && picked.is_none()));
    if let Some(name) = picked {
        app.run("viewport.view", json!({"mode": "image", "image": name}));
    }
}
