//! The JSON control channel: agents drive the running app with the same commands as the menus,
//! inspect session and UI state, invoke menu items, paint strokes and take screenshots.
//!
//! Methods (params as JSON objects):
//! - `command.run {id, params?}`: run an engine command (programmatic: never opens a dialog).
//! - `command.list {filter?}`: every command with params docs and enabled state.
//! - `session.inspect`: project, texture sets, layer stack, tool, camera, display, history.
//! - `ui.inspect`: the UI state (panels, windows, mode, layout, viewport rect, who draws the title bar).
//! - `ui.menu.list`: the menu catalog with ids, shortcuts and whether each is live.
//! - `ui.menu.invoke {id}`: behave like a menu click (may open a window; `ui.resetUi` asks first).
//!   `{id: "ui.layout.apply", name}` switches to the saved panel layout `name` (an error when there is none).
//! - `ui.reset`: reset the panels and every view setting at once, without the question the menu asks.
//! - `ui.set {...}`: merge fields into the UI state (e.g. `{"layout":"Only3D"}`).
//! - `viewport.screenshot {view?: "3d"|"2d"}`: PNG of a viewport, base64.
//! - `ui.screenshot`: PNG of the whole window (replies after the next frame).

use std::sync::mpsc::{Receiver, Sender, channel};

use base64::Engine as _;
use serde_json::{Value, json};

use crate::TextureCraftApp;

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<Value>,
}

impl ControlRequest {
    pub fn new(method: String, params: Value) -> (ControlRequest, Receiver<Value>) {
        let (tx, rx) = channel();
        (ControlRequest { method, params, reply: tx }, rx)
    }
}

fn ok(v: Value) -> Value {
    json!({ "ok": true, "result": v })
}

fn err(e: impl std::fmt::Display) -> Value {
    json!({ "ok": false, "error": e.to_string() })
}

pub fn poll(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(rx) = app.control.as_ref() else { return };
    let reqs: Vec<ControlRequest> = rx.try_iter().collect();
    for r in reqs {
        if r.method == "ui.screenshot" {
            // Replies when the next frame has been captured.
            let reply = r.reply;
            let repaint = ctx.clone();
            let callback = egui::ScreenshotCallback::new(move |img| {
                let answer = encode_png(img.size[0] as u32, img.size[1] as u32, img.pixels.iter().flat_map(|c| c.to_array()).collect())
                    .map(|(w, h, png)| ok(json!({ "width": w, "height": h, "png": base64::engine::general_purpose::STANDARD.encode(png) })))
                    .unwrap_or_else(|| err("could not encode the screenshot"));
                let _ = reply.send(answer);
                repaint.request_repaint();
            });
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(callback));
            ctx.request_repaint();
            continue;
        }
        let reply = handle(app, ctx, &r.method, &r.params);
        let _ = r.reply.send(reply);
        ctx.request_repaint();
    }
}

pub(crate) fn encode_png(w: u32, h: u32, rgba: Vec<u8>) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some((w, h, png))
}

pub fn handle(app: &mut TextureCraftApp, ctx: &egui::Context, method: &str, params: &Value) -> Value {
    app.ui.prefs.apply_resource_limits();

    // These direct control jobs bypass the engine command availability check
    if !app.session.project_open && matches!(method, "ui.bake.start" | "debug.paintTest") {
        return err("no project is open");
    }

    match method {
        "ping" => ok(json!("pong")),
        "ui.resize" => {
            let size = |key: &str| params.get(key).and_then(Value::as_f64).filter(|value| value.is_finite()).map(|value| value.clamp(640.0, 7680.0) as f32);
            match (size("width"), size("height")) {
                (Some(w), Some(h)) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w, h)));
                    ok(Value::Null)
                }
                _ => err("`width` and `height` (points, 640..7680) are required"),
            }
        }
        "debug.frames" => ok(app.profiler.report()),
        "debug.paintTest" => match crate::profile::start_paint_test(app, params) {
            Ok(()) => ok(Value::Null),
            Err(error) => err(error),
        },
        "command.run" => {
            let Some(id) = params.get("id").and_then(Value::as_str) else { return err("`id` is required") };
            let command_params = params.get("params").cloned().unwrap_or(Value::Null);
            let before = app.session.camera;
            match texturecraft_engine::execute(&mut app.session, id, &command_params) {
                Ok(value) => {
                    if matches!(id, "brush.preset.store" | "brush.preset.rename" | "brush.preset.delete") {
                        app.thumbs.forget_saved_files();
                    }

                    // Work from the old project must not reach its replacement
                    if matches!(id, "file.open" | "file.new" | "file.close" | "project.new" | "project.sample") {
                        app.resize = None;
                        app.pbr_import = None;
                        app.layer_panel = Default::default();
                    }

                    // The Export dialog cannot operate after Close
                    if id == "file.close" {
                        app.ui.windows.export = false;
                    }

                    // A command that moved the camera takes over from a glide in progress; a projection switch joins it
                    app.settle_camera_glide(id, before);

                    ok(value)
                }
                Err(error) => err(error),
            }
        }
        "command.list" => ok(texturecraft_engine::commands::list(&app.session, params.get("filter").and_then(Value::as_str))),
        "session.inspect" => ok(texturecraft_engine::inspect::session(&app.session)),
        "ui.inspect" => {
            let viewport = app.viewport_rect;
            ok(json!({
                "state": app.ui,
                "viewport": [viewport.min.x, viewport.min.y, viewport.width(), viewport.height()],
                "fps": app.fps,
                "uiMs": app.ui_ms,
                "titleBar": app.title_bar.name(),
                "baking": app.bake_progress(),
                "bakeStatus": app.bake_status(),
                "bakePreview": texturecraft_engine::bake_preview::preview_json(&app.session),
                "toasts": app.toasts.iter().map(|toast| json!({"text": toast.text, "error": toast.error})).collect::<Vec<_>>(),
            }))
        }
        "ui.bake.start" => {
            // Bake in the background like the Bake button: the 3D view shows the maps as they finish
            let count = app.session.doc.project.texture_sets.len();
            let sets: Vec<usize> = match params.get("ts") {
                Some(value) => match serde_json::from_value::<Vec<usize>>(if value.is_array() { value.clone() } else { json!([value]) }) {
                    Ok(list) if list.iter().all(|&index| index < count) => list,
                    _ => return err(format!("`ts` must be texture set indices below {count}")),
                },
                None => app
                    .session
                    .doc
                    .project
                    .texture_sets
                    .iter()
                    .enumerate()
                    .filter(|(_, texture_set)| texture_set.bake.common.is_enabled)
                    .map(|(index, _)| index)
                    .collect(),
            };
            if app.bake.is_some() {
                return err("a bake is already running");
            }

            if sets.is_empty() {
                return err("no texture set to bake: tick one in Bake mode or pass `ts`");
            }

            app.ui.mode = crate::state::Mode::Baking;
            app.start_bake(sets);
            ok(texturecraft_engine::bake_preview::preview_json(&app.session))
        }
        "ui.bake.cancel" => {
            app.cancel_bake();
            ok(Value::Null)
        }
        "ui.menu.list" => ok(json!(
            crate::menus::CATALOG
                .iter()
                .filter(|item| item.label != "-")
                .map(|item| json!({
                    "menu": item.menu, "submenu": item.sub, "label": item.label, "id": item.id, "shortcut": crate::keymap::shortcut_of(&app.ui.keymap, item.id),
                    "implemented": crate::menus::implemented(item.id), "enabled": crate::menus::enabled(app, item.id),
                }))
                .collect::<Vec<_>>()
        )),
        "ui.menu.invoke" => {
            let Some(id) = params.get("id").and_then(Value::as_str) else { return err("`id` is required") };
            if id == "ui.layout.apply" {
                let Some(name) = params.get("name").and_then(Value::as_str) else { return err("`name` is required") };
                return if app.ui.apply_named_layout(name) { ok(Value::Null) } else { err(format!("no saved layout named `{name}`")) };
            }

            if !crate::menus::implemented(id) {
                return err(format!("menu item `{id}` is not implemented"));
            }

            crate::menus::invoke(app, ctx, id);
            ok(Value::Null)
        }

        // What Window > Reset UI does once the question is answered with Reset
        "ui.reset" => {
            app.ui.adopt_layout(crate::UiState::default());
            ok(Value::Null)
        }
        "ui.set" => {
            let mut current_state = serde_json::to_value(&app.ui).unwrap_or(Value::Null);
            if let (Some(object), Some(patch)) = (current_state.as_object_mut(), params.as_object()) {
                for (key, value) in patch {
                    object.insert(key.clone(), value.clone());
                }
            }

            match serde_json::from_value(current_state) {
                Ok(ui) => {
                    app.ui = ui;
                    app.ui.prefs.apply_resource_limits();
                    ok(Value::Null)
                }
                Err(error) => err(format!("invalid UI state: {error}")),
            }
        }
        "viewport.screenshot" => {
            let view = if params.get("view").and_then(Value::as_str) == Some("2d") { crate::viewport::VIEW_2D } else { crate::viewport::VIEW_3D };
            match screenshot(app, view) {
                Some((w, h, png)) => ok(json!({ "width": w, "height": h, "png": base64::engine::general_purpose::STANDARD.encode(png) })),
                None => err("no rendered view to capture yet"),
            }
        }
        other => err(format!("unknown method `{other}`")),
    }
}

/// Encode a rendered view as PNG.
pub fn screenshot(app: &TextureCraftApp, view: u64) -> Option<(u32, u32, Vec<u8>)> {
    if !app.session.project_open {
        return None;
    }

    let rs = app.render_state.as_ref()?;
    let renderer = rs.renderer.read();
    let gpu = renderer.callback_resources.get::<crate::gpu::GpuState>()?;
    let (w, h, rgba) = gpu.renderer.read_view(&rs.device, &rs.queue, view)?;
    encode_png(w, h, rgba)
}
