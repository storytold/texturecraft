use egui::collapsing_header::CollapsingState;
use egui::{Rect, Ui, vec2};
use egui_phosphor::regular as icon;
use texturecraft_render::BakeOverlay;

use super::BakeVisualizationToggles;
use super::section_opening::SectionOpening;
use crate::TextureCraftApp;
use crate::i18n::{n_, tr, trf};
use crate::screen_color_pick::eyedropper;
use crate::theme::Tokens;
use crate::widgets::{color_swatch, combo, icon_button, section_chevron, slider};

/// The panel's width and its gap from the 3D view's top-left corner
const PANEL_WIDTH: f32 = 280.0;
const PANEL_MARGIN: f32 = 12.0;

/// Opacity of the panel's backing
const PANEL_OPACITY: f32 = 0.92;

/// Size of the panel's eye button, and of the smaller buttons on its rows
const EYE_BUTTON_SIZE: f32 = 22.0;
const ROW_BUTTON_SIZE: f32 = 18.0;

/// Width of an overlay's color swatch
const SWATCH_WIDTH: f32 = 44.0;

/// The neutral material's quality steps: samples per pixel and their names
const QUALITIES: [(u32, &str); 6] = [
    (4, n_("Very Low (4 spp)")),
    (16, n_("Low (16 spp)")),
    (32, n_("Medium (32 spp)")),
    (64, n_("High (64 spp)")),
    (128, n_("Very High (128 spp)")),
    (256, n_("Ultra (256 spp)")),
];

/// The Baking Visualization panel in the top-left corner of bake mode's 3D view `view`
pub(crate) fn bake_visualization_panel(app: &mut TextureCraftApp, ui: &Ui, view: Rect) {
    let toggles = &mut app.ui.bake_visualization;
    let tokens = Tokens::get(ui.ctx());
    egui::Area::new(ui.scope_id().with("bake_visualization_panel")).fixed_pos(view.left_top() + vec2(PANEL_MARGIN, PANEL_MARGIN)).constrain_to(view).show(
        ui.ctx(),
        |ui| {
            // Its own backing, so the panel reads in every theme over whatever the 3D view shows
            egui::Frame::popup(ui.style()).fill(tokens.panel.gamma_multiply(PANEL_OPACITY)).show(ui, |ui| {
                ui.set_width(PANEL_WIDTH);
                // Ids renamed with the reference defaults, so open/closed states saved by earlier builds don't carry over
                let mut collapsing_state = CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id("baking_visualization_open"), true);
                if header(ui, toggles, collapsing_state.is_open()).clicked() {
                    collapsing_state.toggle(ui);
                }

                collapsing_state.show_body_unindented(ui, |ui| sections(ui, toggles));
            });
        },
    );
}

/// The panel's sections below its header
fn sections(ui: &mut Ui, toggles: &mut BakeVisualizationToggles) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut toggles.selected_set_only, tr("Show for Selected Texture Set Only"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(icon::INFO).color(Tokens::get(ui.ctx()).text_dim))
                .on_hover_text(tr("Only the cage and the high poly meshes of the selected texture set show."));
        });
    });

    // The eye greys what it hides, and holds the project mesh shown
    let meshes_editable = !toggles.hide_baking_meshes;
    if collapsible(ui, "high_poly", tr("High Definition Mesh (HP)"), SectionOpening::Open, true, meshes_editable, |ui| {
        overlay_row(ui, "bake_high_poly_color", tr("Mesh"), &mut toggles.high_poly_mesh);
        overlay_row(ui, "bake_matching_error_color", tr("Matching Error"), &mut toggles.matching_error);
    }) {
        toggles.reset_high_poly();
    }

    if collapsible(ui, "cage", tr("Cage"), SectionOpening::Open, true, meshes_editable, |ui| {
        overlay_row(ui, "bake_cage_surface_color", tr("Cage Surface"), &mut toggles.cage_surface);
        ui.add_enabled_ui(toggles.cage_surface.shown, |ui| slider(ui, tr("Cage Surface Opacity"), &mut toggles.cage_surface.opacity, 0.0..=1.0, 2));
        overlay_row(ui, "bake_cage_wire_color", tr("Cage Wireframe"), &mut toggles.cage_wire);
        ui.add_enabled_ui(toggles.cage_wire.shown, |ui| slider(ui, tr("Cage Wireframe Opacity"), &mut toggles.cage_wire.opacity, 0.0..=1.0, 2));
    }) {
        toggles.reset_cage();
    }

    if collapsible(ui, "seams", tr("UV Seams"), SectionOpening::Closed, true, true, |ui| {
        overlay_row(ui, "bake_missing_seam_color", tr("Missing Seams on Hard Edges"), &mut toggles.missing_seams);
    }) {
        toggles.reset_seams();
    }

    if collapsible(ui, "project_mesh", tr("Project Mesh (LP)"), SectionOpening::Closed, true, true, |ui| {
        ui.add_enabled_ui(meshes_editable, |ui| overlay_row(ui, "bake_project_mesh_color", tr("Project Mesh"), &mut toggles.project_mesh));
        ui.add_enabled_ui(toggles.project_mesh.shown, |ui| {
            collapsible(ui, "neutral_material", tr("Neutral Material"), SectionOpening::Open, false, true, |ui| neutral_material(ui, toggles))
        });
    }) {
        toggles.reset_project_mesh();
    }
}

/// The project mesh's neutral material (picked in the 3D view's view menu): its quality, roughness, metallic, occlusion strength and bent normal
fn neutral_material(ui: &mut Ui, toggles: &mut BakeVisualizationToggles) {
    ui.add_enabled_ui(toggles.neutral_shown, |ui| {
        let neutral = &mut toggles.neutral;
        let selected = QUALITIES.iter().find(|(samples, _)| *samples == neutral.samples_per_pixel).map_or("", |(_, name)| tr(name));
        combo(ui, tr("Quality"), selected, |ui| {
            for (samples, name) in QUALITIES {
                ui.selectable_value(&mut neutral.samples_per_pixel, samples, tr(name));
            }
        });
        slider(ui, tr("Roughness"), &mut neutral.roughness, 0.0..=1.0, 2);
        slider(ui, tr("Metallic"), &mut neutral.metallic, 0.0..=1.0, 2);
        slider(ui, tr("AO intensity"), &mut neutral.ao_intensity, 0.0..=1.0, 2);
        ui.checkbox(&mut neutral.bent_normal, tr("Bent Normal"));
        ui.add_enabled_ui(neutral.bent_normal, |ui| {
            slider(ui, tr("Bent Normal Diffuse Amount"), &mut neutral.bent_normal_diffuse, 0.0..=1.0, 2);
            slider(ui, tr("Bent Normal Specular Amount"), &mut neutral.bent_normal_specular, 0.0..=1.0, 2);
        });
    });
}

/// A section `title`, open or closed at first as `opening` says, with a reset button on its header when `resettable` and its contents greyed unless `editable`; returns whether the reset was clicked
fn collapsible(ui: &mut Ui, id: &str, title: &str, opening: SectionOpening, resettable: bool, editable: bool, body: impl FnOnce(&mut Ui)) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let mut collapsing_state =
        CollapsingState::load_with_default_open(ui.ctx(), ui.make_persistent_id(("baking_visualization_section", id)), opening == SectionOpening::Open);
    let (header, reset) = ui
        .horizontal(|ui| {
            ui.set_min_height(EYE_BUTTON_SIZE);
            let text = egui::RichText::new(format!("{}  {title}", section_chevron(ui, collapsing_state.is_open()).text())).strong().color(tokens.text_strong);
            let header = ui.add(egui::Label::new(text).selectable(false).sense(egui::Sense::click()));
            // Dimmed while the section is closed, as the reference draws it
            let reset = resettable.then(|| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(collapsing_state.is_open() && editable, |ui| {
                        icon_button(ui, icon::CLOCK_COUNTER_CLOCKWISE, &trf("Reset {section}", &[("section", &title)]), false, ROW_BUTTON_SIZE)
                    })
                    .inner
                })
                .inner
            });
            (header, reset)
        })
        .inner;
    if header.clicked() {
        collapsing_state.toggle(ui);
    }

    collapsing_state.show_body_indented(&header, ui, |ui| {
        ui.add_enabled_ui(editable, body);
    });
    reset.is_some_and(|reset| reset.clicked())
}

/// A checkbox for whether `overlay` shows, with its color swatch and an eyedropper for it on the right
fn overlay_row(ui: &mut Ui, id: &str, label: &str, overlay: &mut BakeOverlay) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut overlay.shown, label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(overlay.shown, |ui| {
                eyedropper(
                    ui,
                    ui.make_persistent_id(id),
                    &mut overlay.color,
                    &trf("Pick the {overlay} Color from the Screen", &[("overlay", &label)]),
                    ROW_BUTTON_SIZE,
                );
                color_swatch(ui, id, &mut overlay.color, SWATCH_WIDTH);
            });
        });
    });
}

/// The panel's title with a chevron showing whether the panel is `open`, and the eye that hides the baking meshes; returns the title's response
fn header(ui: &mut Ui, toggles: &mut BakeVisualizationToggles, open: bool) -> egui::Response {
    let tokens = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.set_min_height(EYE_BUTTON_SIZE);
        let text = egui::RichText::new(format!("{}  {}", section_chevron(ui, open).text(), tr("Baking Visualization"))).color(tokens.text_strong);
        let response = ui.add(egui::Label::new(text).selectable(false).sense(egui::Sense::click()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, icon::EYE_SLASH, tr("Hide Baking Meshes"), toggles.hide_baking_meshes, EYE_BUTTON_SIZE).clicked() {
                toggles.toggle_hiding();
            }
        });
        response
    })
    .inner
}
