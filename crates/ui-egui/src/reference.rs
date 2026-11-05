//! The Reference panel (images to paint from: zoom, pan, flip, grayscale, colour picking into the
//! brush) and the reference pinned over the 3D view. The overlay is drawn by egui over the 3D
//! view's rectangle only: the renderer, the path tracer and the exports never see it.
//!
//! Everything that changes the project goes through the `reference.*` engine commands.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use egui::{Color32, Id, Rect, RichText, Sense, Stroke, TextureHandle, Ui, Vec2, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::Session;
use texturecraft_engine::core::{Reference, ReferenceSource};
use texturecraft_engine::reference_cmds::{DISPLAY_SIDE, RefView, hex};

use crate::TextureCraftApp;
use crate::i18n::{n_, tr, trf};
use crate::state::Mode;
use crate::theme::Tokens;
use crate::widgets::{chip, icon_button, slider_edit};

/// Averaging radius (source pixels) of the sample choices.
const SAMPLES: [(&str, f32); 4] = [(n_("Point sample"), 0.0), (n_("3 x 3 average"), 1.0), (n_("5 x 5 average"), 2.0), (n_("9 x 9 average"), 4.0)];

/// What the panel remembers while the app runs (not saved with the project).
#[derive(Clone)]
struct PanelState {
    selected: Option<u64>,
    zoom: f32,
    pan: Vec2,
    /// New images are linked to their file instead of copied into the project.
    link: bool,
    sample: usize,
}

impl Default for PanelState {
    fn default() -> Self {
        PanelState { selected: None, zoom: 1.0, pan: Vec2::ZERO, link: false, sample: 0 }
    }
}

fn state_id() -> Id {
    Id::unique("texturecraft-reference-panel")
}

fn rect_id() -> Id {
    Id::unique("texturecraft-reference-panel-rect")
}

/// Display textures by (reference, grayscale), with the decoded image they were made from.
#[derive(Clone, Default)]
struct Textures(Arc<Mutex<TextureMap>>);

type TextureMap = HashMap<(u64, bool), (usize, TextureHandle)>;

/// The egui texture of reference `id` (grayscale applied), made again when the image changed.
fn texture(ctx: &egui::Context, session: &Session, id: u64, gray: bool) -> Option<TextureHandle> {
    let image = session.reference_image(id)?;
    let key = Arc::as_ptr(&image) as usize;
    let cache = ctx.data_mut(|d| d.get_temp_mut_or_default::<Textures>(Id::unique("texturecraft-reference-textures")).clone());
    let mut map = cache.0.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((k, tex)) = map.get(&(id, gray))
        && *k == key
    {
        return Some(tex.clone());
    }
    let (w, h, rgba) = image.display_copy(DISPLAY_SIDE, gray);
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    let tex = ctx.load_texture(format!("reference-{id}-{gray}"), color, egui::TextureOptions::LINEAR);
    map.retain(|(i, _), _| session.doc.project.references.iter().any(|r| r.id == *i));
    map.insert((id, gray), (key, tex.clone()));
    Some(tex)
}

/// UV rectangle showing the image with the reference's flips.
fn uv_of(r: &Reference) -> Rect {
    let (u0, u1) = if r.flip_h { (1.0, 0.0) } else { (0.0, 1.0) };
    let (v0, v1) = if r.flip_v { (1.0, 0.0) } else { (0.0, 1.0) };
    Rect::from_min_max(pos2(u0, v0), pos2(u1, v1))
}

// ---------------------------------------------------------------------------------------------
// The overlay

/// Draw the pinned reference over the 3D view `rect` (painting mode only).
pub fn overlay(app: &TextureCraftApp, ui: &Ui, rect: Rect) {
    if app.ui.mode != Mode::Painting {
        return;
    }
    let o = &app.session.doc.project.reference_overlay;
    let Some(r) = o.reference.and_then(|id| app.session.doc.project.references.iter().find(|r| r.id == id)) else { return };
    let Some(image) = app.session.reference_image(r.id) else { return };
    let Some(tex) = texture(ui.ctx(), &app.session, r.id, r.grayscale) else { return };
    let aspect = image.width as f32 / image.height.max(1) as f32;
    let h = (o.scale * rect.height()).max(1.0);
    let center = pos2(rect.left() + o.center[0] * rect.width(), rect.top() + o.center[1] * rect.height());
    let img = Rect::from_center_size(center, vec2(h * aspect, h));
    ui.painter().with_clip_rect(rect).image(tex.id(), img, uv_of(r), Color32::WHITE.gamma_multiply(o.opacity.clamp(0.0, 1.0)));
}

// ---------------------------------------------------------------------------------------------
// Adding images

/// Add the image files at `paths` as references; failures become toasts.
#[cfg(not(target_arch = "wasm32"))]
fn add_files(app: &mut TextureCraftApp, paths: &[std::path::PathBuf], link: bool) {
    for p in paths {
        let path = p.display().to_string();
        match app.session.add_reference_file(&path, link) {
            Ok(id) => select(id),
            Err(e) => app.toast(trf("Couldn't add {name}: {error}", &[("name", &path), ("error", &e)]), true),
        }
    }
}

/// The panel shows the new image on the next frame.
pub(crate) fn select(id: u64) {
    PENDING.lock().unwrap_or_else(PoisonError::into_inner).replace(id);
}

/// The reference to select on the next frame.
static PENDING: Mutex<Option<u64>> = Mutex::new(None);

#[cfg(not(target_arch = "wasm32"))]
fn choose_files(app: &mut TextureCraftApp, link: bool) {
    let Some(paths) = rfd::FileDialog::new().add_filter(tr("Images"), crate::web_files::IMAGE_EXTS).pick_files() else { return };
    add_files(app, &paths, link);
}

/// Paste an image from the clipboard (or an image file's path copied as text).
#[cfg(not(target_arch = "wasm32"))]
fn paste(app: &mut TextureCraftApp) {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => return app.toast(trf("The clipboard isn't available: {error}", &[("error", &e)]), true),
    };
    if let Ok(img) = clipboard.get_image() {
        let Some(rgba) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) else {
            return app.toast(tr("The clipboard has no image."), true);
        };
        let mut png = Vec::new();
        if rgba.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            return app.toast(tr("The clipboard has no image."), true);
        }
        match app.session.add_reference_bytes("Pasted image.png", png) {
            Ok(id) => select(id),
            Err(e) => app.toast(trf("Couldn't add {name}: {error}", &[("name", &"Pasted image.png"), ("error", &e)]), true),
        }
        return;
    }
    let path = clipboard.get_text().ok().map(|t| std::path::PathBuf::from(t.trim().trim_start_matches("file://")));
    match path.filter(|p| p.is_file()) {
        Some(p) => add_files(app, &[p], false),
        None => app.toast(tr("The clipboard has no image."), true),
    }
}

/// Where the Reference panel was drawn last frame.
pub fn panel_rect(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp::<Rect>(rect_id()))
}

/// Is the pointer over the Reference panel (as of the last frame)?
fn over_panel(ctx: &egui::Context) -> bool {
    panel_rect(ctx).zip(ctx.input(|i| i.pointer.hover_pos())).is_some_and(|(r, p)| r.contains(p))
}

/// A file dropped on the window while the pointer is over the Reference panel belongs to the
/// panel: true (and the file is added) when so.
pub fn take_drop(app: &mut TextureCraftApp, ctx: &egui::Context, file: &egui::DroppedFileHandle) -> bool {
    if !over_panel(ctx) {
        return false;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = ctx;
        let link = ctx.data(|d| d.get_temp::<PanelState>(state_id())).unwrap_or_default().link;
        add_files(app, &[file.path().to_path_buf()], link);
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = app;
        crate::web_files::drop_reference(ctx, file);
    }
    true
}

// ---------------------------------------------------------------------------------------------
// The panel

pub fn panel(app: &mut TextureCraftApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let tk = Tokens::get(&ctx);
    let mut st = ctx.data(|d| d.get_temp::<PanelState>(state_id())).unwrap_or_default();
    ctx.data_mut(|d| d.insert_temp(rect_id(), ui.max_rect()));
    if let Some(id) = PENDING.lock().unwrap_or_else(PoisonError::into_inner).take() {
        st.selected = Some(id);
        st.zoom = 1.0;
        st.pan = Vec2::ZERO;
    }
    // Files dropped on a floating panel's own window arrive in its viewport.
    #[cfg(not(target_arch = "wasm32"))]
    if !ctx.input(|i| i.raw.dropped_files.is_empty()) && over_panel(&ctx) && ctx.viewport_id() != egui::ViewportId::ROOT {
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            take_drop(app, &ctx, &f);
        }
    }

    let refs = app.session.doc.project.references.clone();
    if st.selected.is_none_or(|id| !refs.iter().any(|r| r.id == id)) {
        st.selected = refs.first().map(|r| r.id);
    }

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if icon_button(ui, ic::PLUS, tr("Add reference images"), false, 24.0).clicked() {
            #[cfg(not(target_arch = "wasm32"))]
            choose_files(app, st.link);
            #[cfg(target_arch = "wasm32")]
            crate::web_files::pick_reference(&ctx);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if icon_button(ui, ic::CLIPBOARD, tr("Paste an image from the clipboard"), false, 24.0).clicked() {
                paste(app);
            }
            if chip(ui, tr("Link files"), st.link)
                .on_hover_text(tr("Keep a link to the file (a path next to the project) instead of a copy inside the project."))
                .clicked()
            {
                st.link = !st.link;
            }
        }
    });

    if refs.is_empty() {
        ui.add_space(12.0);
        ui.label(RichText::new(tr("Drop images here, paste, or press + to keep references beside your model.")).color(tk.text_dim));
        ctx.data_mut(|d| d.insert_temp(state_id(), st));
        return;
    }

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for r in &refs {
            let bad = app.session.reference_error(r.id).is_some();
            let label = if bad { format!("{} {}", ic::WARNING, r.name) } else { r.name.clone() };
            let mut resp = chip(ui, &label, st.selected == Some(r.id));
            if let Some(e) = app.session.reference_error(r.id) {
                resp = resp.on_hover_text(e.message());
            }
            if resp.clicked() {
                st.selected = Some(r.id);
                st.zoom = 1.0;
                st.pan = Vec2::ZERO;
            }
        }
    });

    let Some(sel) = st.selected.and_then(|id| refs.iter().find(|r| r.id == id)).cloned() else { return };
    let pinned = app.session.doc.project.reference_overlay.reference == Some(sel.id);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if icon_button(ui, ic::FLIP_HORIZONTAL, tr("Flip horizontally"), sel.flip_h, 24.0).clicked() {
            app.run("reference.set", json!({"id": sel.id, "flipH": !sel.flip_h}));
        }
        if icon_button(ui, ic::FLIP_VERTICAL, tr("Flip vertically"), sel.flip_v, 24.0).clicked() {
            app.run("reference.set", json!({"id": sel.id, "flipV": !sel.flip_v}));
        }
        if icon_button(ui, ic::CIRCLE_HALF, tr("Grayscale"), sel.grayscale, 24.0).clicked() {
            app.run("reference.set", json!({"id": sel.id, "grayscale": !sel.grayscale}));
        }
        if icon_button(ui, ic::PUSH_PIN, tr("Pin over the 3D view"), pinned, 24.0).clicked() {
            app.run("reference.overlay", json!({"reference": if pinned { None } else { Some(sel.id) }}));
        }
        if icon_button(ui, ic::MAGNIFYING_GLASS, tr("Fit in the panel"), false, 24.0).clicked() {
            st.zoom = 1.0;
            st.pan = Vec2::ZERO;
        }
        if app.session.reference_error(sel.id).is_some() && icon_button(ui, ic::ARROWS_CLOCKWISE, tr("Read the file again"), false, 24.0).clicked() {
            app.run("reference.reload", json!({"id": sel.id}));
        }
        if icon_button(ui, ic::TRASH, tr("Remove this reference"), false, 24.0).clicked() {
            app.run("reference.remove", json!({"id": sel.id}));
        }
    });
    if let Some(ReferenceSource::Linked { path }) = Some(&sel.source) {
        ui.label(RichText::new(path).size(11.0).color(tk.text_dim));
    }
    if pinned {
        let o = app.session.doc.project.reference_overlay.clone();
        for (label, key, mut v, range) in [
            (tr("Opacity"), "opacity", o.opacity, 0.0..=1.0),
            (tr("Position X"), "x", o.center[0], -0.5..=1.5),
            (tr("Position Y"), "y", o.center[1], -0.5..=1.5),
            (tr("Size"), "scale", o.scale, 0.05..=3.0),
        ] {
            if let Some(merge) = slider_edit(ui, label, &mut v, range, 2) {
                app.run("reference.overlay", json!({ key: v, "merge": merge }));
            }
        }
    }
    egui::ComboBox::from_id_salt("reference-sample").selected_text(tr(SAMPLES.get(st.sample).map_or("Point sample", |s| s.0))).show_ui(ui, |ui| {
        for (i, (name, _)) in SAMPLES.iter().enumerate() {
            ui.selectable_value(&mut st.sample, i, tr(name));
        }
    });

    // The image.
    let avail = ui.available_size().max(vec2(40.0, 40.0));
    let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, tk.viewport);
    match (app.session.reference_image(sel.id), app.session.reference_error(sel.id)) {
        (Some(image), _) => {
            let (iw, ih) = (image.width as f32, image.height as f32);
            let fit = (rect.width() / iw).min(rect.height() / ih).max(1e-4);
            if resp.hovered() {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y + (i.zoom_delta() - 1.0) * 200.0);
                if let (true, Some(p)) = (scroll != 0.0, resp.hover_pos()) {
                    let f = (scroll * 0.004).exp();
                    let zoom = (st.zoom * f).clamp(0.05, 64.0);
                    let f = zoom / st.zoom;
                    let from_center = p - rect.center();
                    st.pan = from_center - (from_center - st.pan) * f;
                    st.zoom = zoom;
                }
            }
            if resp.dragged() {
                st.pan += ctx.input(|i| i.pointer.delta());
            }
            if resp.double_clicked() {
                st.zoom = 1.0;
                st.pan = Vec2::ZERO;
            }
            let scale = fit * st.zoom;
            let img_rect = Rect::from_center_size(rect.center() + st.pan, vec2(iw * scale, ih * scale));
            if let Some(tex) = texture(&ctx, &app.session, sel.id, sel.grayscale) {
                painter.image(tex.id(), img_rect, uv_of(&sel), Color32::WHITE);
            }
            if let Some(p) = resp.hover_pos().filter(|p| img_rect.contains(*p)) {
                ui.output_mut(|o| o.cursor_icon = egui::CursorIcon::Crosshair);
                let (u, v) = ((p.x - img_rect.left()) / img_rect.width(), (p.y - img_rect.top()) / img_rect.height());
                let radius = SAMPLES.get(st.sample).map_or(0.0, |s| s.1);
                if let Some(linear) = image.pick(u, v, radius, RefView::of(&sel)) {
                    let c = hex(linear);
                    let srgb = texturecraft_engine::texel::linear_to_srgb;
                    let swatch =
                        Color32::from_rgb((srgb(linear[0]) * 255.0 + 0.5) as u8, (srgb(linear[1]) * 255.0 + 0.5) as u8, (srgb(linear[2]) * 255.0 + 0.5) as u8);
                    let tag = Rect::from_min_size(rect.left_bottom() + vec2(6.0, -26.0), vec2(86.0, 20.0));
                    painter.rect(tag, 4.0, tk.panel, Stroke::new(1.0, tk.button_border), egui::StrokeKind::Inside);
                    painter.rect_filled(Rect::from_min_size(tag.min + vec2(3.0, 3.0), vec2(14.0, 14.0)), 3.0, swatch);
                    painter.text(tag.left_center() + vec2(22.0, 0.0), egui::Align2::LEFT_CENTER, c, egui::FontId::monospace(11.0), tk.text_strong);
                }
                if resp.clicked() {
                    app.run("reference.pick", json!({"id": sel.id, "u": u, "v": v, "radius": radius}));
                }
                resp.on_hover_text(tr("Click to use this colour for the brush"));
            }
        }
        (None, error) => {
            let why = error.map(|e| e.message().to_string()).unwrap_or_default();
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                trf("Can't show this image: {error}", &[("error", &why)]),
                egui::FontId::proportional(12.0),
                tk.text_dim,
            );
        }
    }
    ctx.data_mut(|d| d.insert_temp(state_id(), st));
}
