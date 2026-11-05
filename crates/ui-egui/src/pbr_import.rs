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

/// The open import dialog.
pub struct PbrDialog {
    /// `{path}` or `{files}`: where the maps come from.
    source: Value,
    /// What the scan found.
    scan: Value,
    name: String,
    /// The file picked per map where several were found.
    choose: BTreeMap<String, String>,
    /// 0: automatic (from the file name), 1: OpenGL, 2: DirectX.
    normal: usize,
    triplanar: bool,
    library: bool,
}

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
    app.pbr_import = Some(PbrDialog { source, scan, name, choose: BTreeMap::new(), normal: 0, triplanar: false, library: false });
}

/// File › Import PBR texture folder: choose the folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn pick(app: &mut TextureCraftApp) {
    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
        begin(app, json!({ "path": dir.display().to_string() }));
    }
}

/// Dropped paths: a folder, a zip, or two or more image files that name PBR maps open the
/// dialog. Returns true when the drop was taken.
#[cfg(not(target_arch = "wasm32"))]
pub fn dropped(app: &mut TextureCraftApp, paths: &[std::path::PathBuf]) -> bool {
    use texturecraft_engine::formats::pbr_maps;
    if let Some(dir) = paths.iter().find(|p| p.is_dir()) {
        begin(app, json!({ "path": dir.display().to_string() }));
        return true;
    }
    if let Some(zip) = paths.iter().find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"))) {
        begin(app, json!({ "path": zip.display().to_string() }));
        return true;
    }
    let names: Vec<String> = paths.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
    if names.len() >= 2 && pbr_maps::detect(names.iter().map(|n| (n.as_str(), 0))).chosen.len() >= 2 {
        begin(app, json!({ "files": paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>() }));
        return true;
    }
    false
}

fn label_of(id: &str) -> &'static str {
    MAPS.iter().find(|(i, _)| *i == id).map_or("", |(_, l)| *l)
}

/// The dialog, while open.
pub fn window(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(mut d) = app.pbr_import.take() else { return };
    let mut open = true;
    let (mut go, mut cancel) = (false, false);
    crate::windows::dialog("pbrImport", tr("Import PBR textures"), &mut open).show(ctx, |ui| {
        ui.set_min_width(380.0);
        ui.horizontal(|ui| {
            ui.label(tr("Layer name"));
            ui.text_edit_singleline(&mut d.name);
        });
        ui.add_space(4.0);
        ui.label(RichText::new(tr("Detected maps")).strong());
        let maps = d.scan.get("maps").and_then(Value::as_object).cloned().unwrap_or_default();
        let candidates = d.scan.get("candidates").and_then(Value::as_object).cloned().unwrap_or_default();
        for (id, _) in MAPS {
            let Some(file) = maps.get(id).and_then(Value::as_str) else { continue };
            let shown = d.choose.get(id).map(String::as_str).unwrap_or(file).to_string();
            match candidates.get(id).and_then(Value::as_array) {
                Some(all) => {
                    combo(ui, tr(label_of(id)), shown, |ui| {
                        for c in all.iter().filter_map(Value::as_str) {
                            let mut cur = d.choose.get(id).cloned().unwrap_or_else(|| file.to_string());
                            if ui.selectable_value(&mut cur, c.to_string(), c).clicked() {
                                d.choose.insert(id.to_string(), cur);
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
        if maps.is_empty() {
            ui.label(tr("Nothing recognised in this folder"));
        }
        let ignored = d.scan.get("ignored").and_then(Value::as_array).map_or(0, Vec::len);
        if ignored > 0 {
            ui.label(RichText::new(trf("{n} files ignored", &[("n", &ignored)])).weak());
        }
        ui.add_space(6.0);
        if maps.contains_key("normal") {
            let found = d.scan.get("normalFormat").and_then(Value::as_str).unwrap_or("openGl") == "directX";
            let auto = if found { format!("{} ({})", tr("Automatic"), tr("DirectX")) } else { format!("{} ({})", tr("Automatic"), tr("OpenGL")) };
            let shown = [auto.clone(), tr("OpenGL").to_string(), tr("DirectX").to_string()][d.normal.min(2)].clone();
            combo(ui, tr("Normal map format"), shown, |ui| {
                ui.selectable_value(&mut d.normal, 0, auto);
                ui.selectable_value(&mut d.normal, 1, tr("OpenGL"));
                ui.selectable_value(&mut d.normal, 2, tr("DirectX"));
            });
        }
        combo(ui, tr("Projection"), if d.triplanar { tr("Triplanar") } else { tr("UV") }, |ui| {
            ui.selectable_value(&mut d.triplanar, false, tr("UV"));
            ui.selectable_value(&mut d.triplanar, true, tr("Triplanar"));
        });
        #[cfg(not(target_arch = "wasm32"))]
        ui.checkbox(&mut d.library, tr("Add to the material library"));
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(!maps.is_empty(), |ui| go = ui.add(primary_button(ui, tr("Import"))).clicked());
            cancel = ui.button(tr("Cancel")).clicked();
        });
    });
    if go {
        run(app, &d);
    }
    if open && !go && !cancel {
        app.pbr_import = Some(d);
    }
}

/// Import with the dialog's choices.
fn run(app: &mut TextureCraftApp, d: &PbrDialog) {
    let mut p = d.source.clone();
    p["name"] = json!(d.name.trim());
    p["normalFormat"] = json!(["auto", "openGl", "directX"][d.normal.min(2)]);
    if d.triplanar {
        p["projection"] = json!("triplanar");
    }
    if !d.choose.is_empty() {
        p["choose"] = json!(d.choose);
    }
    let Some(r) = app.run("file.importPbrFolder", p) else { return };
    let name = r.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    for w in r.get("warnings").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        app.session.log.push(format!("PBR {name}: {w}"));
    }
    app.toast(trf("Imported {name}", &[("name", &name)]), false);
    app.ui.dock.focus(crate::docking::Panel::Layers);
    #[cfg(not(target_arch = "wasm32"))]
    if d.library
        && let Some(layer) = r.get("layer").and_then(Value::as_u64)
    {
        save_to_library(app, layer, &name);
    }
}

/// Keep the imported layer as a smart material in the user library (shelf › Materials).
#[cfg(not(target_arch = "wasm32"))]
fn save_to_library(app: &mut TextureCraftApp, layer: u64, name: &str) {
    let stem = texturecraft_engine::formats::export::sanitize_file_stem(name);
    let dir = crate::library_dir();
    let mut path = dir.join(format!("{stem}.{}", texturecraft_engine::material_lib::SMART_MATERIAL_EXT));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} {n}.{}", texturecraft_engine::material_lib::SMART_MATERIAL_EXT));
        n += 1;
    }
    if app.run("material.save", json!({ "layer": layer, "path": path.display().to_string(), "name": name })).is_some() {
        app.library = None;
        app.toast(trf("Added {name} to the library", &[("name", &name)]), false);
    }
}
