//! Colour swatches next to the colour editors: recent colours, the project's swatches and the
//! user's global ones. Click applies a swatch (to the brush, or to the fill being edited), right-click
//! renames, replaces or deletes, drag reorders (or copies between the project and global lists).
//! Every change is an engine command (`swatch.*`); the global swatches and the recent colours
//! live in [`crate::state::Prefs`] and are kept in step by [`sync`].

use egui::{Color32, Sense, Stroke, StrokeKind, Ui, vec2};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::core::swatch::{self, MAX_SWATCHES, Swatch};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use crate::widgets::{dnd_release, icon_button};

/// Where a clicked swatch goes.
#[derive(Clone, Copy)]
pub enum Target<'a> {
    /// The brush's base colour (the Properties panel of a painting tool).
    Brush,
    /// The base colour of the fill being edited: `base` holds its `layer` or `effect` id.
    Fill(&'a Value),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scope {
    Project,
    Global,
}

impl Scope {
    fn id(self) -> &'static str {
        match self {
            Scope::Project => "project",
            Scope::Global => "global",
        }
    }
}

/// What is being dragged: a swatch of a list.
#[derive(Clone, Copy)]
struct Drag {
    scope: Scope,
    index: usize,
}

/// What a row did this frame, applied after drawing.
enum Act {
    Apply(Scope, usize),
    ApplyRecent(usize),
    AddCurrent(Scope),
    Rename(Scope, usize, String),
    Replace(Scope, usize),
    Delete(Scope, usize),
    Move(Scope, usize, usize),
    Copy(Scope, usize, Scope),
    /// Desktop only: the browser has no palette import yet.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Import(Scope),
    Export(Scope),
}

/// Start a new app with the user's swatches from the preferences.
pub fn load(app: &mut TextureCraftApp) {
    app.session.swatches.global = app.ui.prefs.swatches.iter().take(MAX_SWATCHES).map(|s| Swatch::new(&s.name, s.rgb)).collect();
    app.session.swatches.recent = app.ui.prefs.recent_colors.iter().take(swatch::MAX_RECENT).map(|c| swatch::sanitize(*c)).collect();
}

/// Keep the preferences (saved with the UI state) equal to the session's global swatches and
/// recent colours. Call every frame; it only copies on a change.
pub fn sync(app: &mut TextureCraftApp) {
    if app.ui.prefs.swatches != app.session.swatches.global {
        app.ui.prefs.swatches = app.session.swatches.global.clone();
    }
    if app.ui.prefs.recent_colors != app.session.swatches.recent {
        app.ui.prefs.recent_colors = app.session.swatches.recent.clone();
    }
}

fn display(linear: [f32; 3]) -> Color32 {
    let b = swatch::to_srgb8(linear);
    Color32::from_rgb(b[0], b[1], b[2])
}

/// The swatch strip: `current` is the colour the editor above shows (marked, and what "add"
/// and "replace" take).
pub fn strip(app: &mut TextureCraftApp, ui: &mut Ui, current: [f32; 3], target: Target<'_>, salt: &str) {
    let tk = Tokens::get(ui.ctx());
    let project = app.session.doc.project.swatches.clone();
    let global = app.session.swatches.global.clone();
    let recent = app.session.swatches.recent.clone();
    let mut acts = Vec::new();

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tr("Swatches")).size(11.0).color(tk.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(egui::RichText::new(ic::DOTS_THREE).size(13.0), |ui| {
                for (scope, add, import, export) in [
                    (Scope::Project, "Add the color to the project swatches", "Import palette to the project...", "Export the project palette..."),
                    (Scope::Global, "Add the color to the global swatches", "Import palette to the global swatches...", "Export the global palette..."),
                ] {
                    if ui.button(tr(add)).clicked() {
                        acts.push(Act::AddCurrent(scope));
                        ui.close();
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button(tr(import)).clicked() {
                        acts.push(Act::Import(scope));
                        ui.close();
                    }
                    #[cfg(target_arch = "wasm32")]
                    let _ = import;
                    if ui.button(tr(export)).clicked() {
                        acts.push(Act::Export(scope));
                        ui.close();
                    }
                    if scope == Scope::Project {
                        ui.separator();
                    }
                }
            });
            if icon_button(ui, ic::PLUS, tr("Add the color to the project swatches"), false, 18.0).clicked() {
                acts.push(Act::AddCurrent(Scope::Project));
            }
        });
    });

    if !recent.is_empty() {
        row_label(ui, tr("Recent"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
            for (i, c) in recent.iter().enumerate() {
                let hex = swatch::to_hex(*c);
                let r = square(ui, *c, swatch::same_colour(*c, current), &format!("recent {hex}"));
                if r.on_hover_text(hex).clicked() {
                    acts.push(Act::ApplyRecent(i));
                }
            }
        });
    }
    for (scope, list, label) in [(Scope::Project, &project, tr("Project")), (Scope::Global, &global, tr("Global"))] {
        row_label(ui, label);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
            if list.is_empty() {
                ui.label(egui::RichText::new(tr("Empty: add the current color with +")).size(10.5).color(tk.text_dim));
            }
            for (i, sw) in list.iter().enumerate() {
                swatch_cell(ui, scope, i, sw, current, salt, &mut acts);
            }
        });
    }

    for act in acts {
        run(app, act, current, target);
    }
}

fn row_label(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(10.0).color(Tokens::get(ui.ctx()).text_dim));
}

/// A colour square named `label` (for screen readers and tests); `marked` outlines it.
fn square(ui: &mut Ui, linear: [f32; 3], marked: bool, label: &str) -> egui::Response {
    let tk = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    ui.painter().rect_filled(rect, 3.0, display(linear));
    let stroke = if marked {
        Stroke::new(2.0, tk.accent)
    } else if resp.hovered() {
        Stroke::new(1.5, tk.text)
    } else {
        Stroke::new(1.0, tk.field_border)
    };
    ui.painter().rect_stroke(rect, 3.0, stroke, StrokeKind::Inside);
    resp
}

fn swatch_cell(ui: &mut Ui, scope: Scope, i: usize, sw: &Swatch, current: [f32; 3], salt: &str, acts: &mut Vec<Act>) {
    let tip = if sw.name.is_empty() { sw.hex() } else { format!("{}  {}", sw.name, sw.hex()) };
    let resp = square(ui, sw.rgb, swatch::same_colour(sw.rgb, current), &format!("{} {tip}", scope.id()));
    let resp = resp.on_hover_text(tip);
    if resp.drag_started() {
        resp.dnd_set_drag_payload(Drag { scope, index: i });
    }
    if let Some(d) = resp.dnd_hover_payload::<Drag>() {
        // Where the dragged swatch would land.
        let x = if d.scope == scope && d.index < i { resp.rect.right() + 1.5 } else { resp.rect.left() - 1.5 };
        let a = Tokens::get(ui.ctx()).accent;
        ui.painter().line_segment([egui::pos2(x, resp.rect.top()), egui::pos2(x, resp.rect.bottom())], Stroke::new(2.0, a));
    }
    if let Some(d) = dnd_release::<Drag>(&resp) {
        acts.push(if d.scope == scope { Act::Move(scope, d.index, i) } else { Act::Copy(d.scope, d.index, scope) });
    } else if resp.clicked() {
        acts.push(Act::Apply(scope, i));
    }
    resp.context_menu(|ui| {
        let id = ui.scope_id().with((salt, scope.id(), i, "rename"));
        let mut name = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| sw.name.clone());
        let edit = ui.add(egui::TextEdit::singleline(&mut name).hint_text(tr("Name")).desired_width(130.0).char_limit(swatch::MAX_NAME_CHARS));
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter {
            acts.push(Act::Rename(scope, i, name.clone()));
            ui.data_mut(|d| d.remove_temp::<String>(id));
            ui.close();
        } else {
            ui.data_mut(|d| d.insert_temp(id, name));
        }
        if ui.button(tr("Replace with the current color")).clicked() {
            acts.push(Act::Replace(scope, i));
            ui.close();
        }
        if ui.button(tr("Delete swatch")).clicked() {
            acts.push(Act::Delete(scope, i));
            ui.close();
        }
    });
}

fn run(app: &mut TextureCraftApp, act: Act, current: [f32; 3], target: Target<'_>) {
    let sc = |s: Scope| json!({ "scope": s.id() });
    match act {
        Act::Apply(s, i) => apply(app, json!({ "scope": s.id(), "index": i }), target),
        Act::ApplyRecent(i) => apply(app, json!({ "recent": i }), target),
        Act::AddCurrent(s) => {
            let mut p = sc(s);
            p["color"] = json!(current);
            app.run("swatch.add", p);
        }
        Act::Rename(s, i, name) => {
            let mut p = sc(s);
            p["index"] = json!(i);
            p["newName"] = json!(name);
            app.run("swatch.rename", p);
        }
        Act::Replace(s, i) => {
            let mut p = sc(s);
            p["index"] = json!(i);
            p["color"] = json!(current);
            app.run("swatch.replace", p);
        }
        Act::Delete(s, i) => {
            let mut p = sc(s);
            p["index"] = json!(i);
            app.run("swatch.remove", p);
        }
        Act::Move(s, from, to) => {
            let mut p = sc(s);
            p["from"] = json!(from);
            p["to"] = json!(to);
            app.run("swatch.move", p);
        }
        Act::Copy(from_scope, i, to_scope) => {
            let list = match from_scope {
                Scope::Project => &app.session.doc.project.swatches,
                Scope::Global => &app.session.swatches.global,
            };
            if let Some(sw) = list.get(i).cloned() {
                let mut p = sc(to_scope);
                p["color"] = json!(sw.rgb);
                p["name"] = json!(sw.name);
                app.run("swatch.add", p);
            }
        }
        Act::Import(s) => import(app, s),
        Act::Export(s) => export(app, s),
    }
}

/// `swatch.apply` to the brush, or to the fill being edited.
fn apply(app: &mut TextureCraftApp, mut p: Value, target: Target<'_>) {
    if let (Target::Fill(base), Some(o)) = (target, p.as_object_mut()) {
        o.insert("target".into(), json!("layer"));
        if let Some(b) = base.as_object() {
            o.extend(b.iter().filter(|(k, _)| matches!(k.as_str(), "layer" | "effect")).map(|(k, v)| (k.clone(), v.clone())));
        }
    }
    app.run("swatch.apply", p);
}

#[cfg(not(target_arch = "wasm32"))]
fn import(app: &mut TextureCraftApp, scope: Scope) {
    let Some(path) = rfd::FileDialog::new().add_filter(tr("Color palette"), &["gpl", "txt", "hex"]).pick_file() else { return };
    if let Some(r) = app.run("swatch.import", json!({ "path": path.display().to_string(), "scope": scope.id() })) {
        let n = r["added"].as_u64().unwrap_or(0);
        let skipped = r["skipped"].as_u64().unwrap_or(0);
        app.toast(crate::i18n::trf("Imported {n} colors ({skipped} lines skipped)", &[("n", &n), ("skipped", &skipped)]), false);
    }
}

#[cfg(target_arch = "wasm32")]
fn import(_: &mut TextureCraftApp, _: Scope) {}

#[cfg(not(target_arch = "wasm32"))]
fn export(app: &mut TextureCraftApp, scope: Scope) {
    let dialog = rfd::FileDialog::new().add_filter(tr("GIMP palette"), &["gpl"]).add_filter(tr("Hex list"), &["txt"]).set_file_name("palette.gpl");
    let Some(path) = dialog.save_file() else { return };
    if let Some(r) = app.run("swatch.export", json!({ "path": path.display().to_string(), "scope": scope.id() })) {
        let n = r["count"].as_u64().unwrap_or(0);
        app.toast(crate::i18n::trf("Exported {n} colors", &[("n", &n)]), false);
    }
}

#[cfg(target_arch = "wasm32")]
fn export(app: &mut TextureCraftApp, scope: Scope) {
    if let Some(r) = app.run("swatch.export", json!({ "scope": scope.id() }))
        && let Some(text) = r["text"].as_str()
        && let Err(e) = crate::web_files::download("palette.gpl", text.as_bytes())
    {
        app.toast(e, true);
    }
}
