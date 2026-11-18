//! The Display Settings panel: the view mode, then environment, camera, post effect and viewer settings on one scrolling page

use egui::{RichText, Ui};
use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::camera_cmds::DEFAULT_CAMERA;
use texturecraft_engine::core::ToneMapping;
use texturecraft_engine::core::project::{DisplaySettings, ENV_RESOURCE_PREFIX};
use texturecraft_render::ibl::ENVIRONMENTS;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::section_page::{self, PageSection};
use crate::theme::Tokens;
use crate::unavailable;
use crate::widgets::{combo, icon_button, section_with_reset, slider};

/// The page's sections, in the reference's tab order
pub(crate) const SECTIONS: [PageSection; 4] = [
    PageSection { icon: ic::MOUNTAINS, title: "Environment settings" },
    PageSection { icon: ic::VIDEO_CAMERA, title: "Camera settings" },
    PageSection { icon: ic::FUNCTION, title: "Post effects" },
    PageSection { icon: ic::CHECKERBOARD, title: "Viewer settings" },
];

/// What the reference's shadow opacity slider shows; TextureCraft's shadows have no opacity yet
const SHADOWS_OPACITY_SHOWN: f32 = 90.0;

/// Indent of the rows under a sub-heading (Environment, Shadows)
const ROW_INDENT: f32 = 12.0;

/// The Display Settings panel body (a dock tab or rail popup)
pub(crate) fn body(app: &mut TextureCraftApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(tr("View mode"));
        crate::viewport::view_mode_picker(app, ui, "display_view_mode", ui.available_width());
    });

    let mut display = app.session.doc.project.display.clone();
    let before = display.clone();
    let mut import_hdr = false;
    section_page::show(ui, "display_settings", &SECTIONS, |ui, index| match index {
        0 => import_hdr = environment(app, ui, &mut display),
        1 => camera(app, ui, &mut display),
        2 => post_effects(ui, &mut display),
        _ => viewer(app, ui, &mut display),
    });

    if display != before {
        if display.fov != before.fov {
            app.session.camera.fov = display.fov;
        }
        app.session.doc.project.display = display;
    }
    if import_hdr {
        let ctx = ui.ctx().clone();
        crate::web_files::import_environment(app, &ctx);
    }
}

/// A sub-heading inside a section, as the reference's "Environment" and "Shadows"
fn sub_heading(ui: &mut Ui, text: &str) {
    let tokens = Tokens::get(ui.ctx());
    ui.add_space(4.0);
    ui.label(RichText::new(text).color(tokens.text_strong));
}

/// Rows indented under a sub-heading
fn indented(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(ROW_INDENT);
        ui.vertical(add);
    });
}

/// Environment and Shadows; true when Import HDR was picked
fn environment(app: &TextureCraftApp, ui: &mut Ui, display: &mut DisplaySettings) -> bool {
    let mut import_hdr = false;
    sub_heading(ui, tr("Environment"));
    indented(ui, |ui| {
        import_hdr = environment_map(app, ui, display);
        unavailable::checkbox(ui, false, tr("Override environment map color space"));
        unavailable::greyed(ui, |ui| egui::ComboBox::from_id_salt("env_color_space").width(ui.available_width()).selected_text(tr("Auto")).show_ui(ui, |_| {}));
        slider(ui, tr("Environment Opacity"), &mut display.env_opacity, 0.0..=1.0, 2);
        slider(ui, tr("Environment Exposure (EV)"), &mut display.env_exposure, -5.0..=5.0, 2);
        slider(ui, tr("Environment Blur"), &mut display.env_blur, 0.0..=1.0, 2);
        slider(ui, tr("Environment Rotation"), &mut display.env_rotation, 0.0..=360.0, 0);
        unavailable::combo(ui, tr("Environment Alignment"), tr("World"));
    });

    sub_heading(ui, tr("Shadows"));
    indented(ui, |ui| {
        ui.checkbox(&mut display.shadows, tr("Shadows")).on_hover_text(tr("Key light (shadow-casting light, direct only)"));
        unavailable::combo(ui, tr("Computation mode"), tr("Lightweight"));
        unavailable::slider(ui, tr("Shadows opacity"), SHADOWS_OPACITY_SHOWN, 0.0..=100.0, 0);
    });
    import_hdr
}

/// The Environment Map button and its list: the built-in environments, the project's HDR images and Import HDR; true when Import HDR was picked
fn environment_map(app: &TextureCraftApp, ui: &mut Ui, display: &mut DisplaySettings) -> bool {
    let mut hdrs: Vec<&String> = app.session.images.iter().filter(|(_, image)| !image.srgb).map(|(name, _)| name).collect();
    hdrs.sort();
    let shown = display.environment_resource().unwrap_or(&display.environment).to_string();
    let mut import_hdr = false;
    ui.horizontal(|ui| {
        ui.label(tr("Environment Map"));
        let button = ui.add_sized([ui.available_width(), ui.spacing().interact_size.y], egui::Button::new(shown).truncate());
        egui::Popup::menu(&button).id(ui.make_persistent_id("environment_map_list")).show(|ui| {
            for environment in ENVIRONMENTS {
                ui.selectable_value(&mut display.environment, environment.to_string(), environment);
            }
            ui.separator();
            for name in hdrs {
                ui.selectable_value(&mut display.environment, format!("{ENV_RESOURCE_PREFIX}{name}"), name.as_str());
            }
            import_hdr = ui
                .selectable_label(false, format!("{} {}", ic::FILE_ARROW_UP, tr("Import HDR...")))
                .on_hover_text(tr("Use an equirectangular .hdr or .exr image as the environment"))
                .clicked();
        });
    });
    import_hdr
}

/// The camera preset and its restore, the lens, the saved cameras and depth of field
fn camera(app: &mut TextureCraftApp, ui: &mut Ui, display: &mut DisplaySettings) {
    let current = texturecraft_engine::camera_cmds::current(&app.session);
    let names: Vec<String> = app.session.doc.project.cameras.iter().map(|camera| camera.name.clone()).collect();
    let shown = current.and_then(|index| names.get(index)).map_or(tr(DEFAULT_CAMERA), String::as_str).to_string();
    ui.horizontal(|ui| {
        ui.label(tr("Preset"));
        let restore_width = ui.spacing().interact_size.x * 1.5;
        egui::ComboBox::from_id_salt("display_camera_preset").width((ui.available_width() - restore_width).max(60.0)).selected_text(shown).show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), tr(DEFAULT_CAMERA)).clicked() {
                app.run("camera.select", json!({}));
            }
            for (index, name) in names.iter().enumerate() {
                if ui.selectable_label(current == Some(index), name).clicked() {
                    app.run("camera.select", json!({"index": index}));
                }
            }
        });

        // Back to the preset's own view, after orbiting away from it
        let restore = ui.button(tr("Restore")).on_hover_text(tr("Return to the preset's view"));
        if restore.clicked() {
            let params = current.map_or_else(|| json!({}), |index| json!({"index": index}));
            app.run("camera.select", params);
        }
    });

    slider(ui, tr("Field of view (deg)"), &mut display.fov, 5.0..=120.0, 0);
    let mut ortho = app.session.camera.ortho;
    if ui.checkbox(&mut ortho, tr("Orthographic")).changed() {
        app.run("camera.set", json!({"ortho": ortho}));
    }
    let tone_mapping_name = |tone_mapping: ToneMapping| match tone_mapping {
        ToneMapping::Linear => tr("Linear"),
        ToneMapping::AcesFilmic => tr("ACES filmic"),
    };
    combo(ui, tr("Tone mapping"), tone_mapping_name(display.tone_mapping), |ui| {
        for tone_mapping in [ToneMapping::Linear, ToneMapping::AcesFilmic] {
            ui.selectable_value(&mut display.tone_mapping, tone_mapping, tone_mapping_name(tone_mapping));
        }
    });
    cameras(app, ui);

    sub_heading(ui, tr("Depth of field"));
    ui.checkbox(&mut display.dof.enabled, tr("Depth of field (viewport and Rendering mode)"));
    ui.add_enabled_ui(display.dof.enabled, |ui| {
        let far = (app.session.camera.distance * 4.0).max(display.dof.focus_distance);
        slider(ui, tr("Focus distance"), &mut display.dof.focus_distance, 0.0..=far, 2)
            .on_hover_text(tr("Depth of the sharp plane along the view (0: the orbit center)"));
        slider(ui, tr("Aperture"), &mut display.dof.aperture, 0.0..=1.0, 3)
            .on_hover_text(tr("Lens size: how quickly things blur away from the focus distance"));
        let picking = crate::viewport::focus_picking(ui.ctx());
        if ui.selectable_label(picking, format!("{}  {}", ic::CROSSHAIR_SIMPLE, tr("Pick focus in the viewport"))).clicked() {
            crate::viewport::set_focus_picking(ui.ctx(), !picking);
        }
    });
}

/// The camera list: rename (edit the name), view or delete saved and imported cameras, save the
/// current view.
fn cameras(app: &mut TextureCraftApp, ui: &mut Ui) {
    let current = texturecraft_engine::camera_cmds::current(&app.session);
    let names: Vec<String> = app.session.doc.project.cameras.iter().map(|c| c.name.clone()).collect();
    for (i, name) in names.iter().enumerate() {
        ui.horizontal(|ui| {
            if icon_button(ui, ic::VIDEO_CAMERA, tr("View through this camera"), current == Some(i), 20.0).clicked() {
                app.run("camera.select", json!({"index": i}));
            }
            // The name being typed lives in egui's memory until the field loses focus.
            let id = ui.scope_id().with(("camera_name", i));
            let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| name.clone());
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width() - 26.0));
            if r.changed() {
                ui.data_mut(|d| d.insert_temp(id, text.clone()));
            }
            if r.lost_focus() {
                ui.data_mut(|d| d.remove::<String>(id));
                if text.trim() != name && !text.trim().is_empty() {
                    app.run("camera.rename", json!({"index": i, "to": text.trim()}));
                }
            }
            if icon_button(ui, ic::TRASH, tr("Delete this camera"), false, 20.0).clicked() {
                app.run("camera.remove", json!({"index": i}));
            }
        });
    }
    if ui.button(format!("{}  {}", ic::PLUS, tr("Save view as camera"))).clicked() {
        app.run("camera.add", json!({}));
    }
}

/// The post effects switch and amounts, with a reset on their header
fn post_effects(ui: &mut Ui, display: &mut DisplaySettings) {
    // The switch gates all four amounts (the engine has one switch for the group); each amount 0 leaves that effect out
    let post_effect_fields = |d: &DisplaySettings| (d.post_effects, d.bloom, d.vignette, d.sharpen, d.grain);
    let post_effect_defaults = post_effect_fields(&DisplaySettings::default());
    if section_with_reset(ui, "sec_post_effects", tr("Post effects"), false, post_effect_fields(display) == post_effect_defaults, |ui| {
        ui.checkbox(&mut display.post_effects, tr("Post effects (viewport and Rendering mode)"));
        ui.add_enabled_ui(display.post_effects, |ui| {
            slider(ui, tr("Glow"), &mut display.bloom, 0.0..=1.0, 2).on_hover_text(tr("Bright areas bleed light into their surroundings"));
            slider(ui, tr("Vignette"), &mut display.vignette, 0.0..=1.0, 2).on_hover_text(tr("Darkens the corners of the view"));
            slider(ui, tr("Sharpen"), &mut display.sharpen, 0.0..=1.0, 2).on_hover_text(tr("Brings out fine detail"));
            slider(ui, tr("Film grain"), &mut display.grain, 0.0..=1.0, 2).on_hover_text(tr("Adds fine noise over the view"));
        });
    }) {
        (display.post_effects, display.bloom, display.vignette, display.sharpen, display.grain) = post_effect_defaults;
    }
}

/// The wireframes and temporal anti-aliasing
fn viewer(app: &mut TextureCraftApp, ui: &mut Ui, display: &mut DisplaySettings) {
    ui.checkbox(&mut app.ui.wireframe_3d, tr("Show wireframe in the 3D view"));
    ui.checkbox(&mut app.ui.uv_wireframe, tr("Show UV wireframe in the 2D view"));
    ui.checkbox(&mut display.taa, tr("Temporal anti-aliasing (refines the 3D view while it is still)"));
}
