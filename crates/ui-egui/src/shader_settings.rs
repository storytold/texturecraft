//! The Shader Settings panel: the shader and its instance name, then shader parameters and displacement on one scrolling page

use egui::{RichText, Ui, vec2};
use egui_phosphor::regular as ic;
use texturecraft_engine::core::project::{DisplaySettings, MAX_INDEX_OF_REFRACTION, MIN_INDEX_OF_REFRACTION};

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

/// Wide enough for the longer shader model name
const SHADER_MODEL_COMBO_WIDTH: f32 = 260.0;

/// Width of the scatter color swatch
const SCATTER_SWATCH_WIDTH: f32 = 44.0;

/// Index of refraction slider range: air to well past diamond (2.42)
const INDEX_OF_REFRACTION_RANGE: std::ops::RangeInclusive<f32> = MIN_INDEX_OF_REFRACTION..=MAX_INDEX_OF_REFRACTION;

/// Values the reference's panel shows for controls TextureCraft lacks
const SCATTER_ANISOTROPY_SHOWN: f32 = 0.1;
const SUBDIVISION_COUNT_SHOWN: f32 = 1.0;

/// Size of a tile in the shader choice, and the columns it has
pub(crate) const SHADER_TILE_SIZE: egui::Vec2 = vec2(64.0, 64.0);
const SHADER_TILE_COLUMNS: usize = 5;

/// Size of the shader choice's reset button
const RESET_QUERY_SIZE: f32 = 22.0;

/// The Shader Settings panel body (a dock tab or rail popup)
pub(crate) fn body(app: &mut TextureCraftApp, ui: &mut Ui) {
    header(app, ui);

    let custom = app
        .session
        .doc
        .project
        .display
        .custom_shaders
        .iter()
        .find(|shader| Some(&shader.name) == app.session.doc.project.display.custom_shader.as_ref())
        .cloned();
    if let Some(shader) = custom {
        custom_parameters(app, ui, &shader);
        return;
    }

    let mut display = app.session.doc.project.display.clone();
    let mut ior_specular = display.ior_specular;
    section_page::show(ui, "shader_settings", &SECTIONS, |ui, index| match index {
        0 => shader_parameters(ui, &mut display, &mut ior_specular),
        _ => displacement(ui, &mut display),
    });

    if display != app.session.doc.project.display {
        app.session.doc.project.display = display;
        app.session.mark_modified();
    }

    if ior_specular != app.session.doc.project.display.ior_specular {
        app.run("display.set", serde_json::json!({"iorSpecular": ior_specular}));
    }
}

/// Undo and Redo of the shader's settings, the shader button and the instance name
fn header(app: &mut TextureCraftApp, ui: &mut Ui) {
    let mut edit_action = None;
    ui.columns(2, |columns| {
        let width = columns[0].available_width();
        if columns[0].add_enabled(app.session.can_undo(), egui::Button::new(tr("Undo")).min_size(vec2(width, 24.0))).clicked() {
            edit_action = Some("edit.undo");
        }
        if columns[1].add_enabled(app.session.can_redo(), egui::Button::new(tr("Redo")).min_size(vec2(width, 24.0))).clicked() {
            edit_action = Some("edit.redo");
        }
    });

    let selected = app.session.doc.project.display.custom_shader.as_deref().unwrap_or(CURRENT_SHADER);
    let shader = ui.add_sized([ui.available_width(), ui.spacing().interact_size.y], egui::Button::new(selected));
    egui::Popup::menu(&shader).id(ui.make_persistent_id("shader_choice")).show(|ui| shader_choice(app, ui));

    ui.horizontal(|ui| {
        ui.label(tr("Instance name"));
        let draft_id = ui.make_persistent_id(("shader_instance_draft", app.session.project_id));
        let mut name = ui.data(|data| data.get_temp::<String>(draft_id)).unwrap_or_else(|| app.session.doc.project.display.shader_instance_name.clone());
        let field = ui.add(egui::TextEdit::singleline(&mut name).id(draft_id).desired_width(ui.available_width()));
        field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr("Instance name")));
        if field.changed() {
            ui.data_mut(|data| data.insert_temp(draft_id, name.clone()));
        }
        if field.lost_focus() && name != app.session.doc.project.display.shader_instance_name {
            app.run("shader.set", serde_json::json!({"instanceName":name}));
            ui.data_mut(|data| data.remove::<String>(draft_id));
        }
        if !field.has_focus() && !field.changed() {
            ui.data_mut(|data| data.remove::<String>(draft_id));
        }
    });
    if let Some(command) = edit_action {
        app.run(command, serde_json::json!({}));
    }
}

/// Search built-in entries and project-owned imports in the shader picker.
fn shader_choice(app: &mut TextureCraftApp, ui: &mut Ui) {
    let query_id = ui.make_persistent_id("shader_query");
    let mut query = ui.data(|data| data.get_temp::<String>(query_id)).unwrap_or_else(|| SHADER_QUERY.to_string());
    ui.horizontal(|ui| {
        let field_width = SHADER_TILE_SIZE.x * SHADER_TILE_COLUMNS as f32 - RESET_QUERY_SIZE;
        let field = ui.add(egui::TextEdit::singleline(&mut query).desired_width(field_width));
        field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr("Search shaders")));
        if crate::widgets::icon_button(ui, ic::ARROW_COUNTER_CLOCKWISE, tr("Reset"), false, RESET_QUERY_SIZE).clicked() {
            query = SHADER_QUERY.to_string();
        }
    });
    ui.data_mut(|data| data.insert_temp(query_id, query.clone()));

    let selected = app.session.doc.project.display.custom_shader.clone();
    let imported = app.session.doc.project.display.custom_shaders.clone();
    egui::Grid::new("shader_types").spacing(vec2(4.0, 4.0)).show(ui, |ui| {
        let mut shown = 0;
        for name in SHADER_TYPES.iter().filter(|name| matches_query(name, &query)) {
            let tile = egui::Button::selectable(*name == CURRENT_SHADER && selected.is_none(), RichText::new(format!("{}\n{name}", ic::SPHERE)))
                .min_size(SHADER_TILE_SIZE);
            if *name == CURRENT_SHADER {
                let response = crate::shader_previews::tile(app, ui, name, selected.is_none(), None);
                if response.clicked() {
                    if app.run("shader.select", serde_json::json!({"name":CURRENT_SHADER})).is_some() {
                        app.toast(crate::i18n::trf("Shader selected: {name}", &[("name", &CURRENT_SHADER)]), false);
                        app.announce_latest_toast();
                    }
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
        for shader in imported.iter().filter(|shader| matches_query(&shader.name, &query)) {
            let name = &shader.name;
            let response = crate::shader_previews::tile(app, ui, name, selected.as_ref() == Some(name), Some(shader));
            if response.on_hover_text(name).clicked() {
                if app.run("shader.select", serde_json::json!({"name":name})).is_some() {
                    app.toast(crate::i18n::trf("Shader selected: {name}", &[("name", name)]), false);
                    app.announce_latest_toast();
                }
                ui.close();
            }
            shown += 1;
            if shown % SHADER_TILE_COLUMNS == 0 {
                ui.end_row();
            }
        }
    });
    if ui.button(tr("Import shader")).clicked() {
        crate::web_files::import_shader(app, ui.ctx());
        ui.close();
    }
}

fn custom_parameters(app: &mut TextureCraftApp, ui: &mut Ui, shader: &texturecraft_engine::core::custom_shader::CustomShader) {
    ui.label(tr("Shader parameters"));
    ui.label(tr(texturecraft_engine::core::custom_shader::SHADER_SUPPORT));
    for warning in &shader.warnings {
        ui.label(tr(warning));
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        for input in &shader.textures {
            let mut resource = input.resource.clone();
            let before = resource.clone();
            let selected = resource.clone().unwrap_or_else(|| tr("Default color").to_string());
            let response = ui
                .push_id(&input.name, |ui| {
                    combo(ui, tr(&input.label), &selected, |ui| {
                        ui.selectable_value(&mut resource, None, tr("Default color"));
                        let mut names: Vec<_> = app.session.images.keys().collect();
                        names.sort();
                        for name in names {
                            ui.selectable_value(&mut resource, Some(name.clone()), name);
                        }
                    })
                })
                .inner;
            response.widget_info(|| egui::WidgetInfo {
                current_text_value: Some(selected.clone()),
                ..egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), tr(&input.label))
            });
            if resource != before {
                let mut textures = serde_json::Map::new();
                textures.insert(input.name.clone(), serde_json::json!(resource));
                app.run("shader.set", serde_json::json!({"textures":textures}));
            }
        }
        let mut last_group = String::new();
        for p in &shader.parameters {
            if p.group != last_group {
                ui.separator();
                ui.label(tr(&p.group));
                last_group = p.group.clone();
            }
            let mut value = p.value.clone();
            let changed = if p.kind == "bool" {
                let mut on = value.as_bool().unwrap_or(false);
                let changed = ui.checkbox(&mut on, tr(&p.label)).on_hover_text(tr(&p.description)).changed();
                value = serde_json::json!(on);
                changed
            } else if !p.choices.is_empty() {
                let mut selected = value.as_i64().unwrap_or(0) as i32;
                let before = selected;
                let text = p.choices.iter().find(|(_, v)| *v == selected).map_or("", |(label, _)| tr(label)).to_string();
                let response = ui
                    .push_id(&p.name, |ui| {
                        combo(ui, tr(&p.label), &text, |ui| {
                            for (label, v) in &p.choices {
                                ui.selectable_value(&mut selected, *v, tr(label));
                            }
                        })
                    })
                    .inner;
                response.widget_info(|| egui::WidgetInfo {
                    current_text_value: Some(text.clone()),
                    ..egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), tr(&p.label))
                });
                response.on_hover_text(tr(&p.description));
                value = serde_json::json!(selected);
                selected != before
            } else if p.widget == "color" && p.kind == "vec3" {
                let mut color = std::array::from_fn(|i| value.get(i).and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32);
                ui.label(tr(&p.label));
                let response = ui.color_edit_button_rgb(&mut color);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr(&p.label)));
                let changed = response.on_hover_text(tr(&p.description)).changed();
                value = serde_json::json!(color.map(|component| {
                    let mut component = f64::from(component);
                    if let Some(min) = p.min {
                        component = component.max(min);
                    }
                    if let Some(max) = p.max {
                        component = component.min(max);
                    }
                    component
                }));
                changed
            } else {
                let mut values: Vec<f64> =
                    value.as_array().map_or_else(|| vec![value.as_f64().unwrap_or(0.0)], |a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect());
                let mut changed = false;
                for (i, v) in values.iter_mut().enumerate() {
                    let label = if p.value.is_array() { format!("{} {}", tr(&p.label), i + 1) } else { tr(&p.label).to_string() };
                    if let (Some(min), Some(max)) = (p.min, p.max) {
                        let mut control = egui::Slider::new(v, min..=max).text(&label);
                        if p.kind == "int" || p.kind.starts_with("ivec") {
                            control = control.integer();
                        }
                        changed |= ui.add(control).on_hover_text(tr(&p.description)).changed();
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(&label);
                            let mut control = egui::DragValue::new(v).speed(0.01);
                            if p.kind == "int" || p.kind.starts_with("ivec") {
                                control = control.speed(1.0).max_decimals(0);
                            }
                            let response = ui.add(control);
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, &label));
                            changed |= response.changed();
                        });
                    }
                }
                value = if p.value.is_array() { serde_json::json!(values) } else { serde_json::json!(values[0]) };
                changed
            };
            if changed {
                let mut parameters = serde_json::Map::new();
                parameters.insert(p.name.clone(), value);
                app.run("shader.set", serde_json::json!({"parameters":parameters}));
            }
        }
        if ui.button(tr("Restore defaults")).clicked() {
            app.run("shader.set", serde_json::json!({"reset":true}));
        }
    });
}

/// Whether a shader's name has every word of the query; `u:` words name the resource kind, which every entry is
pub fn matches_query(name: &str, query: &str) -> bool {
    query.split_whitespace().filter(|word| !word.starts_with("u:")).all(|word| name.to_lowercase().contains(&word.to_lowercase()))
}

/// Base Surface, Specular, Transmission and Subsurface
fn shader_parameters(ui: &mut Ui, display: &mut DisplaySettings, ior_specular: &mut bool) {
    let tokens = Tokens::get(ui.ctx());
    section(ui, "shader_base_surface", tr("Base Surface"), true, |ui| {
        shader_model(ui, display);
        slider(ui, tr("AO Intensity"), &mut display.ao_intensity, 0.0..=1.0, 2);
    });
    section(ui, "shader_specular", tr("Specular"), true, |ui| {
        unavailable::checkbox(ui, false, tr("Enable anisotropy"));
        slider(ui, tr("Index of refraction"), &mut display.ior, INDEX_OF_REFRACTION_RANGE, 2).on_hover_text(tr(
            "Controls how strongly transmissive surfaces bend light. When the checkbox below is on, it also controls dielectric reflectance.",
        ));

        ui.add_enabled_ui(!display.spec_gloss, |ui| {
            let widgets = &mut ui.visuals_mut().widgets;
            widgets.inactive.bg_stroke.color = tokens.text_dim;
            widgets.hovered.bg_stroke.color = tokens.text;
            widgets.active.bg_stroke.color = tokens.text_strong;

            ui.checkbox(ior_specular, tr("Use IOR for specular reflection"))
                .on_hover_text(tr(
                    "Uses the index of refraction for dielectric reflectance at normal incidence. Specular level adjusts the surface reflection.",
                ))
                .on_disabled_hover_text(tr("Specular Glossiness uses the Specular color channel for reflectance."));
        });

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
