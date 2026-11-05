//! Scene image export: the **Export image** window (the camera button on the viewport top bar)
//! and the renders behind it and behind `viewport.screenshot {width?, height?, preset?, scale?}`.
//!
//! The settings (size, preset, render multiplier, transparent background) live in
//! [`crate::state::Prefs::image_export`], so they persist like the other preferences, and size
//! both the real-time export here and Rendering mode's path-traced render (and its Save render).
//! Sizes resolve through `texturecraft_render::output`, the same rules as the headless `render`.

use egui::RichText;
use texturecraft_render::output::{self, PRESETS, SCALES, SizeRequest};

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::state::{ImageExport, Mode};

/// The renderer view an export renders into (dropped afterwards; never shown).
pub const VIEW_EXPORT: u64 = 0xE0;
/// Largest exported edge in the app, before the GPU's own limit.
pub const MAX_SIDE: u32 = 4096;

/// The size request of the saved settings.
pub fn request(e: &ImageExport) -> SizeRequest {
    SizeRequest {
        width: (e.width > 0).then_some(e.width),
        height: (e.height > 0).then_some(e.height),
        preset: (!e.preset.is_empty()).then(|| e.preset.clone()),
        scale: Some(e.scale),
    }
}

/// The output size for a view of `base` pixels, within `max_side` (named `limit` in errors).
pub fn resolve(e: &ImageExport, base: [u32; 2], max_side: u32, limit: &'static str) -> Result<[u32; 2], String> {
    output::resolve([base[0] as f32, base[1] as f32], &request(e), max_side, limit).map_err(|err| err.to_string())
}

/// The ceiling for real-time exports: [`MAX_SIDE`] or the GPU's texture limit, whichever is lower.
fn gpu_limit(app: &TextureCraftApp) -> (u32, &'static str) {
    let gpu = app.render_state.as_ref().map_or(MAX_SIDE, |rs| rs.device.limits().max_texture_dimension_2d);
    if gpu < MAX_SIDE { (gpu, "GPU texture limit") } else { (MAX_SIDE, "export limit") }
}

/// Render the view `view` (3D or 2D) again at the size `req` asks for: the same camera, shading
/// and content as on screen, one frame (with the view's anti-aliasing setting: 4× MSAA or one
/// sample per pixel; no temporal accumulation). RGBA8, opaque.
pub fn render_view(app: &TextureCraftApp, view: u64, req: &SizeRequest) -> Result<(u32, u32, Vec<u8>), String> {
    let rs = app.render_state.as_ref().ok_or_else(|| tr("no GPU").to_string())?;
    let (max, limit) = gpu_limit(app);
    let mut renderer = rs.renderer.write();
    let gpu = renderer.callback_resources.get_mut::<crate::gpu::GpuState>().ok_or_else(|| tr("no GPU").to_string())?;
    let mut frame = gpu.renderer.last_frame(view).cloned().ok_or_else(|| tr("no rendered view to capture yet").to_string())?;
    let base = frame.size.map(|v| v as f32);
    frame.size = output::resolve(base, req, max, limit).map_err(|e| e.to_string())?;
    frame.view_id = VIEW_EXPORT;
    let mut enc = rs.device.create_command_encoder(&texturecraft_render::wgpu::CommandEncoderDescriptor { label: Some("tc_image_export") });
    gpu.renderer.prepare(&rs.device, &rs.queue, &mut enc, &frame);
    rs.queue.submit([enc.finish()]);
    let out = gpu.renderer.read_view(&rs.device, &rs.queue, VIEW_EXPORT);
    gpu.renderer.drop_view(VIEW_EXPORT);
    out.ok_or_else(|| tr("Couldn't read the render back from the GPU").to_string())
}

/// The camera button and `ui.imageExport`: open (or close) the Export image window.
pub fn toggle(app: &mut TextureCraftApp) {
    app.ui.windows.image_export = !app.ui.windows.image_export;
}

/// Save PNG bytes through a file dialog (desktop) or as a download (browser).
pub(crate) fn save_png(app: &mut TextureCraftApp, name: String, w: u32, h: u32, rgba: Vec<u8>) {
    let Some(png) = crate::control::encode_png(w, h, rgba).map(|(_, _, png)| png) else {
        app.toast(tr("Couldn't encode the render"), true);
        return;
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(path) = rfd::FileDialog::new().add_filter(tr("PNG image"), &["png"]).set_file_name(name).save_file() else { return };
        match std::fs::write(&path, png) {
            Ok(()) => app.toast(trf("Saved {path}", &[("path", &path.display())]), false),
            Err(e) => app.toast(trf("Couldn't save the render: {error}", &[("error", &e)]), true),
        }
    }
    #[cfg(target_arch = "wasm32")]
    match crate::web_files::download(&name, &png) {
        Ok(()) => app.toast(trf("Downloaded {name}", &[("name", &name)]), false),
        Err(e) => app.toast(trf("Couldn't save the render: {error}", &[("error", &e)]), true),
    }
}

fn file_stem(app: &TextureCraftApp) -> String {
    texturecraft_engine::formats::export::sanitize_file_stem(&app.session.doc.project.name)
}

/// The Export image window.
pub fn window(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if !app.ui.windows.image_export {
        return;
    }
    let mut open = true;
    let rendering = app.ui.mode == Mode::Rendering;
    let base = app.render_state.as_ref().and_then(|rs| {
        let r = rs.renderer.read();
        r.callback_resources.get::<crate::gpu::GpuState>().and_then(|g| g.renderer.last_frame(crate::viewport::VIEW_3D)).map(|f| f.size)
    });
    let mut save = false;
    crate::windows::window("image_export", tr("Export image"), &mut open).default_width(280.0).show(ctx, |ui| {
        let e = &mut app.ui.prefs.image_export;
        egui::Grid::new("image_export_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label(tr("Preset"));
            let shown = if e.preset.is_empty() { tr("Viewport size").to_string() } else { preset_label(&e.preset) };
            egui::ComboBox::from_id_salt("image_export_preset").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut e.preset, String::new(), tr("Viewport size"));
                for p in PRESETS {
                    ui.selectable_value(&mut e.preset, p.id.to_string(), preset_label(p.id));
                }
            });
            ui.end_row();
            ui.label(tr("Width"));
            ui.add(egui::DragValue::new(&mut e.width).range(0..=MAX_SIDE).custom_formatter(|v, _| if v < 1.0 { tr("Auto").into() } else { format!("{v:.0}") }))
                .on_hover_text(tr("Pixels; Auto follows the preset or the viewport and keeps its aspect ratio"));
            ui.end_row();
            ui.label(tr("Height"));
            ui.add(
                egui::DragValue::new(&mut e.height).range(0..=MAX_SIDE).custom_formatter(|v, _| if v < 1.0 { tr("Auto").into() } else { format!("{v:.0}") }),
            )
            .on_hover_text(tr("Pixels; Auto follows the preset or the viewport and keeps its aspect ratio"));
            ui.end_row();
            ui.label(tr("Render scale"));
            ui.horizontal(|ui| {
                for s in SCALES {
                    ui.selectable_value(&mut e.scale, s, format!("{s}\u{d7}"));
                }
            });
            ui.end_row();
        });
        ui.checkbox(&mut e.transparent, tr("Transparent background"))
            .on_hover_text(tr("Path-traced renders only (Rendering mode, on the CPU); the real-time view always draws its backdrop"));
        ui.add_space(4.0);
        let (max, limit) = gpu_limit(app);
        let e = &app.ui.prefs.image_export;
        let size = base.map(|b| resolve(e, b, max, limit));
        match &size {
            Some(Ok([w, h])) => {
                ui.label(RichText::new(trf("Output: {w} × {h} px", &[("w", w), ("h", h)])).strong());
            }
            Some(Err(err)) => {
                ui.colored_label(ui.visuals().error_fg_color, err);
            }
            None => {
                ui.label(tr("no rendered view to capture yet"));
            }
        }
        if rendering {
            ui.label(RichText::new(tr("Rendering mode renders at this size; use Save render... in its panel.")).weak());
        } else {
            if e.transparent {
                ui.label(RichText::new(tr("A transparent background needs Rendering mode (F10).")).weak());
            }
            let ok = matches!(size, Some(Ok(_))) && !e.transparent;
            if ui.add_enabled(ok, egui::Button::new(tr("Save image..."))).clicked() {
                save = true;
            }
        }
    });
    if !open {
        app.ui.windows.image_export = false;
    }
    if save {
        let req = request(&app.ui.prefs.image_export);
        match render_view(app, crate::viewport::VIEW_3D, &req) {
            Ok((w, h, rgba)) => {
                let name = format!("{}_{w}x{h}.png", file_stem(app));
                save_png(app, name, w, h, rgba);
            }
            Err(e) => app.toast(trf("Couldn't render the image: {error}", &[("error", &e)]), true),
        }
    }
}

fn preset_label(id: &str) -> String {
    match output::preset(id) {
        Some(p) => format!("{} ({}\u{d7}{})", p.id.to_uppercase(), p.width, p.height),
        None => id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_map_to_size_requests() {
        let mut e = ImageExport::default();
        assert_eq!(resolve(&e, [1000, 500], 4096, "x"), Ok([1000, 500]), "the default is the viewport at 1x");
        e.scale = 2;
        assert_eq!(resolve(&e, [1000, 500], 4096, "x"), Ok([2000, 1000]));
        e.width = 400;
        assert_eq!(resolve(&e, [1000, 500], 4096, "x"), Ok([800, 400]), "one edge keeps the view's aspect");
        e = ImageExport { preset: "hd".into(), scale: 4, ..ImageExport::default() };
        let err = resolve(&e, [1000, 500], 4096, "GPU texture limit").unwrap_err();
        assert!(err.contains("7680x4320") && err.contains("GPU texture limit"), "{err}");
        e.scale = 3;
        assert!(resolve(&e, [1000, 500], 4096, "x").is_err(), "a hand-edited scale is refused, not used");
    }
}
