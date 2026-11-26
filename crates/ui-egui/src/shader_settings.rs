//! The Shader Settings panel: the shader and its instance name, then shader parameters and displacement on one scrolling page

use egui::{RichText, Ui, vec2};
use egui_phosphor::regular as ic;
use texturecraft_engine::core::project::DisplaySettings;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::section_page::{self, PageSection};
use crate::theme::Tokens;
use crate::unavailable::{self, NOT_AVAILABLE};
use crate::widgets::{color_swatch, combo, section, slider};

/// The page's sections, in the reference's tab order
pub(crate) const SECTIONS: [PageSection; 2] =
    [PageSection { icon: ic::GLOBE_HEMISPHERE_WEST, title: "Shader parameters" }, PageSection { icon: ic::DIAMOND, title: "Displacement & tessellation" }];

/// The shader TextureCraft draws with, the one its shader button names
pub const CURRENT_SHADER: &str = "open-pbr";

/// The shader choice lists these, as the reference's list shows them; the names it cuts short stay cut short
pub const SHADER_TYPES: [&str; 17] = [
    "asm-...",
    "clay",
    "dota-2",
    "non-p...",
    CURRENT_SHADER,
    "pbr-ca...",
    "pbr-co...",
    "pbr-m...",
    "pbr-m...",
    "pbr-m...",
    "pbr-m...",
    "pbr-m...",
    "pbr-m...",
    "pbr-sp...",
    "pbr-ve...",
    "vraymt...",
    "vraymt...",
];

/// The shader choice's search starts as the reference's: shaders only
pub const SHADER_QUERY: &str = "u:shader";

/// The reference's default instance name
const INSTANCE_NAME: &str = "Main shader";

/// Wide enough for the longer shader model name
const SHADER_MODEL_COMBO_WIDTH: f32 = 260.0;

/// Width of the scatter color swatch
const SCATTER_SWATCH_WIDTH: f32 = 44.0;

/// Index of refraction slider range: air to well past diamond (2.42)
const INDEX_OF_REFRACTION_RANGE: std::ops::RangeInclusive<f32> = 1.0..=3.0;

/// Values the reference's panel shows for controls TextureCraft lacks
const SCATTER_ANISOTROPY_SHOWN: f32 = 0.1;
const SUBDIVISION_COUNT_SHOWN: f32 = 1.0;

/// Size of a tile in the shader choice, and the columns it has
const SHADER_TILE_SIZE: egui::Vec2 = vec2(64.0, 64.0);
const SHADER_TILE_COLUMNS: usize = 5;

/// Size of the shader choice's reset button
const RESET_QUERY_SIZE: f32 = 22.0;

/// The Shader Settings panel body (a dock tab or rail popup)
pub(crate) fn body(app: &mut TextureCraftApp, ui: &mut Ui) {
    header(ui);

    let mut display = app.session.doc.project.display.clone();
    section_page::show(ui, "shader_settings", &SECTIONS, |ui, index| match index {
        0 => shader_parameters(ui, &mut display),
        _ => displacement(ui, &mut display),
    });

    if display != app.session.doc.project.display {
        app.session.doc.project.display = display;
        app.session.modified = true;
        app.session.revision = app.session.revision.wrapping_add(1);
    }
}

/// Undo and Redo of the shader's settings, the shader button and the instance name
fn header(ui: &mut Ui) {
    ui.columns(2, |columns| {
        let width = columns[0].available_width();
        unavailable::greyed(&mut columns[0], |ui| ui.add_sized([width, ui.spacing().interact_size.y], egui::Button::new(tr("Undo"))));
        unavailable::greyed(&mut columns[1], |ui| ui.add_sized([width, ui.spacing().interact_size.y], egui::Button::new(tr("Redo"))));
    });

    let shader = ui.add_sized([ui.available_width(), ui.spacing().interact_size.y], egui::Button::new(CURRENT_SHADER));
    egui::Popup::menu(&shader).id(ui.make_persistent_id("shader_choice")).show(shader_choice);

    ui.horizontal(|ui| {
        ui.label(tr("Instance name"));
        let mut name = tr(INSTANCE_NAME).to_string();
        unavailable::greyed(ui, |ui| ui.add(egui::TextEdit::singleline(&mut name).desired_width(ui.available_width())));
    });
}

/// The shader choice under the shader button: a search, and a tile for each shader the reference lists (only the current one works)
fn shader_choice(ui: &mut Ui) {
    let query_id = ui.make_persistent_id("shader_query");
    let mut query = ui.data(|data| data.get_temp::<String>(query_id)).unwrap_or_else(|| SHADER_QUERY.to_string());
    ui.horizontal(|ui| {
        let field_width = SHADER_TILE_SIZE.x * SHADER_TILE_COLUMNS as f32 - RESET_QUERY_SIZE;
        ui.add(egui::TextEdit::singleline(&mut query).desired_width(field_width));
        if crate::widgets::icon_button(ui, ic::ARROW_COUNTER_CLOCKWISE, tr("Reset"), false, RESET_QUERY_SIZE).clicked() {
            query = SHADER_QUERY.to_string();
        }
    });
    ui.data_mut(|data| data.insert_temp(query_id, query.clone()));

    egui::Grid::new("shader_types").spacing(vec2(4.0, 4.0)).show(ui, |ui| {
        let mut shown = 0;
        for name in SHADER_TYPES.iter().filter(|name| matches_query(name, &query)) {
            let tile = egui::Button::selectable(*name == CURRENT_SHADER, RichText::new(format!("{}\n{name}", ic::SPHERE))).min_size(SHADER_TILE_SIZE);
            if *name == CURRENT_SHADER {
                if ui.add(tile).clicked() {
                    ui.close();
                }
            } else {
                ui.add_enabled(false, tile).on_disabled_hover_text(tr(NOT_AVAILABLE));
            }

            shown += 1;
            if shown % SHADER_TILE_COLUMNS == 0 {
                ui.end_row();
            }
        }
    });
}

/// Whether a shader's name has every word of the query; `u:` words name the resource kind, which every entry is
pub fn matches_query(name: &str, query: &str) -> bool {
    query.split_whitespace().filter(|word| !word.starts_with("u:")).all(|word| name.to_lowercase().contains(&word.to_lowercase()))
}

/// Base Surface, Specular, Transmission and Subsurface
fn shader_parameters(ui: &mut Ui, display: &mut DisplaySettings) {
    let tokens = Tokens::get(ui.ctx());
    section(ui, "shader_base_surface", tr("Base Surface"), true, |ui| {
        shader_model(ui, display);
        slider(ui, tr("AO Intensity"), &mut display.ao_intensity, 0.0..=1.0, 2);
    });
    section(ui, "shader_specular", tr("Specular"), true, |ui| {
        unavailable::checkbox(ui, false, tr("Enable anisotropy"));
        slider(ui, tr("Index of refraction"), &mut display.ior, INDEX_OF_REFRACTION_RANGE, 2).on_hover_text(tr(
            "How strongly transmissive surfaces bend light (where the Transmission channel is painted): water 1.33, glass 1.5, diamond 2.42",
        ));
        unavailable::checkbox(ui, false, tr("Enable specular color"));
    });
    section(ui, "shader_transmission", tr("Transmission"), true, |ui| {
        unavailable::checkbox(ui, false, tr("Enable transmission"));
    });
    section(ui, "shader_subsurface", tr("Subsurface"), true, |ui| {
        unavailable::checkbox(ui, true, tr("Enable"));
        slider(ui, tr("Subsurface radius"), &mut display.scatter_radius, 0.0..=1.0, 3)
            .on_hover_text(tr("How far light travels under the surface, as a fraction of the mesh size (where the Scattering weight channel is painted)"));
        unavailable::slider(ui, tr("Subsurface scatter anisotropy"), SCATTER_ANISOTROPY_SHOWN, -1.0..=1.0, 1);
        unavailable::checkbox(ui, false, tr("Use thickness map"));
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr("Scatter color")).color(tokens.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                color_swatch(ui, "shader_scatter_color", &mut display.scatter_color, SCATTER_SWATCH_WIDTH)
            })
        })
        .response
        .on_hover_text(tr("Color of the light that travels under the surface (each channel goes as far as its value)"));
    });
}

/// TextureCraft's two material models, with what each reads and how it lights
fn shader_model(ui: &mut Ui, display: &mut DisplaySettings) {
    let model_name = |spec_gloss: bool| if spec_gloss { tr("TextureCraft PBR - Specular Glossiness") } else { tr("TextureCraft PBR - Metallic Roughness") };
    let channels = if display.spec_gloss {
        tr("Base color is the diffuse color, Specular the F0 color and Glossiness 1 - roughness (add these channels in the texture set settings).")
    } else {
        tr("Base color, Metallic, Roughness and Specular level.")
    };
    let model_text = tr(
        "GGX specular with height-correlated Smith visibility, Lambert diffuse, split-sum image-based lighting with spherical-harmonics irradiance and horizon fading.",
    );
    let optional = tr(
        "Optional channels add a clear coat (Coat weight, Coat roughness, Coat normal), sheen (Sheen weight, color, roughness), anisotropy (Anisotropy level, angle), subsurface scattering (Scattering weight) and transmission (Transmission).",
    );

    egui::ComboBox::from_id_salt("shader_model")
        .width(SHADER_MODEL_COMBO_WIDTH.min(ui.available_width()))
        .selected_text(model_name(display.spec_gloss))
        .show_ui(ui, |ui| {
            for spec_gloss in [false, true] {
                ui.selectable_value(&mut display.spec_gloss, spec_gloss, model_name(spec_gloss));
            }
        })
        .response
        .on_hover_text(format!("{model_text} {channels} {optional}"));
}

/// The displacement switch, parallax, the displacement source and the tessellation, with Restore defaults
fn displacement(ui: &mut Ui, display: &mut DisplaySettings) {
    ui.checkbox(&mut display.displacement, tr("Enable")).on_hover_text(tr(
        "Moves the surface by the height channel on a finer copy of the mesh, at the height to normal strength (viewport only: painting follows the base surface)",
    ));
    ui.checkbox(&mut display.parallax, tr("Parallax occlusion (height channel)"))
        .on_hover_text(tr("Offsets the textures by the height channel, at the height to normal strength"));

    section(ui, "shader_displacement", tr("Displacement"), true, |ui| {
        // The height channel is the only source TextureCraft displaces by
        combo(ui, tr("Source Channel"), tr("Height"), |ui| {
            let _ = ui.selectable_label(true, tr("Height"));
        });
        unavailable::combo(ui, tr("Scale unit"), tr("Normalized"));
        unavailable::slider(ui, tr("Scale amount"), 0.0, -1.0..=1.0, 0);
    });
    section(ui, "shader_tessellation", tr("Tessellation"), true, |ui| {
        unavailable::combo(ui, tr("Subdivision Mode"), tr("Uniform"));
        unavailable::slider(ui, tr("Subdivision Count"), SUBDIVISION_COUNT_SHOWN, 1.0..=6.0, 0);
    });

    // One row keeps the button to its height; a bare right-to-left layout fills the page
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let defaults = DisplaySettings::default();
            let at_default = display.displacement == defaults.displacement && display.parallax == defaults.parallax;
            if ui.add_enabled(!at_default, egui::Button::new(tr("Restore defaults"))).clicked() {
                display.displacement = defaults.displacement;
                display.parallax = defaults.parallax;
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_matches_every_word_but_the_kind() {
        assert!(matches_query(CURRENT_SHADER, SHADER_QUERY));
        assert!(matches_query(CURRENT_SHADER, "u:shader open"));
        assert!(matches_query("pbr-sp...", "PBR"));
        assert!(!matches_query("clay", "u:shader pbr"));
    }

    #[test]
    fn the_list_names_the_current_shader_once() {
        assert_eq!(SHADER_TYPES.iter().filter(|name| **name == CURRENT_SHADER).count(), 1);
    }
}
