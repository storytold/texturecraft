//! File › Import PBR texture folder: pick a folder (or drop a folder, a zip or several image
//! files), check which maps were found, then import them as one fill layer.
//!
//! The dialog is a thin front end over `file.scanPbrFolder` and `file.importPbrFolder`: it
//! holds the choices (layer name, a file where several name one map, the normal-map convention,
//! the projection, whether to keep the material in the library) and passes them as params.

use std::collections::BTreeMap;

use egui::RichText;
use serde_json::{Value, json};

use crate::TextureCraftApp;
use crate::i18n::{n_, tr, trf};
use crate::widgets::{combo, primary_button};

mod pbr_dialog;

pub use pbr_dialog::PbrDialog;

/// Map ids in the order they are listed, with their English label.
const MAPS: [(&str, &str); 10] = [
    ("baseColor", "Base color"),
    ("roughness", "Roughness"),
    ("glossiness", "Glossiness"),
    ("metallic", "Metallic"),
    ("normal", "Normal"),
    ("height", "Height"),
    ("ambientOcclusion", "Ambient occlusion"),
    ("emissive", "Emissive"),
    ("opacity", "Opacity"),
    ("orm", n_("Packed (occlusion, roughness, metallic)")),
];

/// Scan `source` and open the dialog on the result (an error toast when nothing is found).
pub fn begin(app: &mut TextureCraftApp, source: Value) {
    let Some(scan) = app.run("file.scanPbrFolder", source.clone()) else { return };
    let name = scan.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    app.pbr_import = Some(PbrDialog {
        source,
        scan,
        name,
        choose: BTreeMap::new(),
        normal: 0,
        triplanar: false,
        #[cfg(not(target_arch = "wasm32"))]
        library: false,
    });
}

/// File › Import PBR texture folder: choose the folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn pick(app: &mut TextureCraftApp) {
    use crate::file_dialogs::{Bound, FileDialog};
    app.ask_file(FileDialog::folder(), Bound::Project, |app, paths| {
        if let Some(dir) = paths.first() {
            begin(app, json!({ "path": dir.display().to_string() }));
        }
    });
}

/// Dropped paths: a folder, a zip, or two or more image files that name PBR maps open the
/// dialog. Returns the indices consumed by the dialog; other files keep their normal drop path.
#[cfg(not(target_arch = "wasm32"))]
pub fn dropped(app: &mut TextureCraftApp, paths: &[std::path::PathBuf]) -> Vec<usize> {
    use texturecraft_engine::formats::pbr_maps;
    if let Some((index, dir)) = paths.iter().enumerate().find(|(_, p)| p.is_dir()) {
        begin(app, json!({ "path": dir.display().to_string() }));
        return vec![index];
    }

    if let Some((index, zip)) = paths.iter().enumerate().find(|(_, p)| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))) {
        begin(app, json!({ "path": zip.display().to_string() }));
        return vec![index];
    }

    let recognized: Vec<usize> = paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let name = path.file_name()?.to_string_lossy();
            pbr_maps::classify(&name).ok().filter(|detected| detected.kind != pbr_maps::MapKind::Unused).map(|_| index)
        })
        .collect();
    if recognized.len() >= 2 {
        begin(app, json!({ "files": recognized.iter().filter_map(|&index| paths.get(index)).map(|p| p.display().to_string()).collect::<Vec<_>>() }));
        return recognized;
    }

    Vec::new()
}

fn label_of(id: &str) -> &'static str {
    MAPS.iter().find(|(i, _)| *i == id).map_or("", |(_, l)| *l)
}

/// The dialog, while open.
pub fn window(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(mut dialog_state) = app.pbr_import.take() else { return };

    let mut open = true;
    let (mut import_clicked, mut cancel_clicked) = (false, false);
    crate::windows::dialog("pbrImport", tr("Import PBR textures"), &mut open).show(ctx, |ui| {
        ui.set_min_width(380.0);
        ui.horizontal(|ui| {
            ui.label(tr("Layer name"));
            ui.text_edit_singleline(&mut dialog_state.name);
        });
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Detected maps")).strong());
        let maps = dialog_state.scan.get("maps").and_then(Value::as_object);
        let candidates = dialog_state.scan.get("candidates").and_then(Value::as_object);

        for (id, _) in MAPS {
            let Some(file) = maps.and_then(|maps| maps.get(id)).and_then(Value::as_str) else { continue };
            let shown = dialog_state.choose.get(id).map(String::as_str).unwrap_or(file).to_string();
            match candidates.and_then(|candidates| candidates.get(id)).and_then(Value::as_array) {
                Some(all) => {
                    combo(ui, tr(label_of(id)), shown, |ui| {
                        for candidate in all.iter().filter_map(Value::as_str) {
                            let selected = dialog_state.choose.get(id).map(String::as_str).unwrap_or(file);
                            if ui.selectable_label(selected == candidate, candidate).clicked() {
                                dialog_state.choose.insert(id.to_string(), candidate.to_string());
                            }
                        }
                    });
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.label(tr(label_of(id)));
                        ui.label(RichText::new(shown).weak());
                    });
                }
            }
        }
        if maps.is_none_or(|maps| maps.is_empty()) {
            ui.label(tr("Nothing recognised in this folder"));
        }

        let ignored = dialog_state.scan.get("ignored").and_then(Value::as_array).map_or(0, Vec::len);
        if ignored > 0 {
            ui.label(RichText::new(trf("{n} files ignored", &[("n", &ignored)])).weak());
        }

        ui.add_space(6.0);
        if maps.is_some_and(|maps| maps.contains_key("normal")) {
            let chosen = dialog_state.choose.get("normal").map(String::as_str).or_else(|| maps.and_then(|maps| maps.get("normal")).and_then(Value::as_str));
            let found = chosen
                .and_then(|name| texturecraft_engine::formats::pbr_maps::classify(name).ok())
                .is_some_and(|detected| detected.normal == Some(texturecraft_engine::core::NormalFormat::DirectX));
            let auto = if found { format!("{} ({})", tr("Automatic"), tr("DirectX")) } else { format!("{} ({})", tr("Automatic"), tr("OpenGL")) };
            let shown = [auto.clone(), tr("OpenGL").to_string(), tr("DirectX").to_string()][dialog_state.normal.min(2)].clone();
            combo(ui, tr("Normal map format"), shown, |ui| {
                ui.selectable_value(&mut dialog_state.normal, 0, auto);
                ui.selectable_value(&mut dialog_state.normal, 1, tr("OpenGL"));
                ui.selectable_value(&mut dialog_state.normal, 2, tr("DirectX"));
            });
        }
        combo(ui, tr("Projection"), if dialog_state.triplanar { tr("Triplanar") } else { tr("UV") }, |ui| {
            ui.selectable_value(&mut dialog_state.triplanar, false, tr("UV"));
            ui.selectable_value(&mut dialog_state.triplanar, true, tr("Triplanar"));
        });
        #[cfg(not(target_arch = "wasm32"))]
        ui.checkbox(&mut dialog_state.library, tr("Add to the material library"));
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(maps.is_some_and(|maps| !maps.is_empty()), |ui| import_clicked = ui.add(primary_button(ui, tr("Import"))).clicked());
            cancel_clicked = ui.button(tr("Cancel")).clicked();
        });
    });

    let imported = import_clicked && run(app, &dialog_state);
    if open && !imported && !cancel_clicked {
        app.pbr_import = Some(dialog_state);
    }
}

/// Import with the dialog's choices.
fn run(app: &mut TextureCraftApp, dialog_state: &PbrDialog) -> bool {
    let mut params = dialog_state.source.clone();
    params["name"] = json!(dialog_state.name.trim());
    params["normalFormat"] = json!(["auto", "openGl", "directX"][dialog_state.normal.min(2)]);
    if dialog_state.triplanar {
        params["projection"] = json!("triplanar");
    }

    // Pin the displayed candidates so a later folder scan cannot silently replace them
    let mut choose: BTreeMap<String, String> = dialog_state
        .scan
        .get("maps")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|maps| maps.iter())
        .filter_map(|(id, file)| file.as_str().map(|file| (id.clone(), file.to_string())))
        .collect();
    choose.extend(dialog_state.choose.clone());
    if !choose.is_empty() {
        params["choose"] = json!(choose);
    }

    let Some(reply) = app.run("file.importPbrFolder", params) else { return false };

    let name = reply.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    for warning in reply.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        app.session.log.warn(texturecraft_engine::log::LIBRARY_SOURCE, format!("PBR {name}: {warning}"));
    }

    app.toast(trf("Imported {name}", &[("name", &name)]), false);
    app.ui.dock.focus(crate::docking::Panel::Layers);
    #[cfg(not(target_arch = "wasm32"))]
    if dialog_state.library
        && let Some(layer) = reply.get("layer").and_then(Value::as_u64)
    {
        save_to_library(app, layer, &name);
    }

    true
}

/// Keep the imported layer as a smart material in the user library (shelf › Materials).
#[cfg(not(target_arch = "wasm32"))]
fn save_to_library(app: &mut TextureCraftApp, layer: u64, name: &str) {
    let stem = texturecraft_engine::formats::export::sanitize_file_stem(name);
    let dir = crate::library_dir();
    let mut path = dir.join(format!("{stem}.{}", texturecraft_engine::material_lib::SMART_MATERIAL_EXT));
    let mut suffix_index = 2;

    while path.exists() {
        path = dir.join(format!("{stem} {suffix_index}.{}", texturecraft_engine::material_lib::SMART_MATERIAL_EXT));
        suffix_index += 1;
    }

    if app.run("material.save", json!({ "layer": layer, "path": path.display().to_string(), "name": name })).is_some() {
        app.library = None;
        app.toast(trf("Added {name} to the library", &[("name", &name)]), false);
    }
}
