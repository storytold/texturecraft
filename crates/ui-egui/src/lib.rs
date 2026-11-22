//! # texturecraft-ui-egui
//!
//! The TextureCraft desktop and web shell: a dense dark professional layout (menu bar,
//! tool-settings bar, tool shelf, asset shelf, 3D + 2D viewports, texture set list, layer stack,
//! properties, display/shader/history windows, bake mode). It is a thin layer: every action goes
//! through `texturecraft-engine` commands, so the CLI, control channel and MCP can do the same.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod angle_dial;
mod ask;
mod bake_job;
pub mod bake_mode;
mod bake_preset;
pub mod bake_presets;
mod bake_report;
mod bake_scene;
mod color_management;
mod color_selection_editor;
pub mod control;
mod decal_drop;
mod display_settings;
pub mod dock;
pub mod docking;
mod environment_picker;
mod export_panel;
pub mod export_templates;
mod export_window;
#[cfg(not(target_arch = "wasm32"))]
pub mod file_dialogs;
mod fill_source_slot;
mod fill_source_target;
mod filter_slot;
pub mod fonts;
mod generator_panel;
pub mod gpu;
mod gray_slider;
mod grid_tile;
pub mod i18n;
pub mod id_color_pick;
#[cfg(not(target_arch = "wasm32"))]
mod import_resources;
pub mod import_ui;
mod inactive_set_hint;
pub mod keymap;
pub mod layer_panel;
mod layout_dialog;
mod levels_editor;
mod levels_handle;
mod levels_properties;
mod line_snap;
mod list_action;
pub mod log_panel;
mod mask_preview_image;
mod material_preview_image;
pub mod menus;
mod mesh_export_options;
mod name_error;
mod named_layout;
pub mod named_layouts;
mod nav_button;
pub mod nav_buttons;
mod nav_view;
pub mod new_project_advanced;
mod orb_shape;
pub mod parity;
mod path_hit;
mod path_surface_cache;
mod pattern_panel;
mod pattern_target;
pub mod pbr_import;
mod preview_image;
mod preview_request;
mod preview_worker;
pub mod profile;
#[cfg(not(target_arch = "wasm32"))]
mod project_config;
pub mod project_form;
pub mod properties_popup;
mod query_field;
mod rail_popup;
pub mod render_mode;
mod resource_panel;
mod resource_picker;
mod resource_picker_anchor;
mod resource_picker_choice;
mod resource_picker_item;
mod resource_picker_tab;
mod resource_picker_tile;
mod running_bake;
pub mod saved_search;
mod screen_color_pick;
mod section_page;
pub mod settings_window;
pub mod shader_settings;
pub mod shelf;
mod shelf_entry;
mod shelf_folders;
pub mod shelf_panes;
pub mod shelf_preview;
mod shelf_tabs;
mod shelf_toolbar;
pub mod shelf_view;
#[cfg(not(target_arch = "wasm32"))]
mod smart_mask_menu;
mod soft_box;
pub mod state;
mod surface_paint_point;
pub mod symmetry;
pub mod theme;
pub mod thumbs;
mod tile_edits;
pub mod titlebar;
pub mod toolbar;
pub mod ui_state_store;
pub mod unavailable;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) mod updates_ui;
pub mod uv_paint_point;
mod uv_tile_grid;
mod uv_view_map;
mod view_mode_picker;
pub mod viewport;
pub mod web_files;
pub mod widgets;
pub mod windows;
mod worker_preview;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::Value;
use texturecraft_engine::Session;
use texturecraft_engine::bake::Progress;

pub use bake_job::BakeJob;
pub use control::ControlRequest;
pub use running_bake::RunningBake;
pub use state::UiState;

/// The app as the updater knows it: its releases, and the icon and color of its update prompt.
#[cfg(not(target_arch = "wasm32"))]
pub const UPDATE_APP: craft_update::App = craft_update::App {
    icon_png: Some(include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.texturecraft.png")),
    accent: Some([0xc2, 0x67, 0x2b]),
    ..craft_update::App::new("texturecraft", "TextureCraft", "3D texture painting", "storytold/texturecraft")
};

/// A texture set resize running on a worker thread (`Session::resize_task`).
pub struct ResizeJob {
    pub progress: Arc<texturecraft_engine::ResizeProgress>,
    pub rx: std::sync::mpsc::Receiver<texturecraft_engine::Result<texturecraft_engine::ResizeOutput>>,
    project: u64,
}

impl Drop for ResizeJob {
    fn drop(&mut self) {
        self.progress.cancel();
    }
}

mod link_bake_input;
#[cfg(not(target_arch = "wasm32"))]
mod link_import;

/// A MaterialCraft material re-cooking on a worker thread (one per layer at a time).
pub struct LinkCook {
    pub layer: texturecraft_engine::core::LayerId,
    project: u64,
    mesh: u64,
    size: u32,
    baked_inputs: Vec<link_bake_input::LinkBakeInput>,
    canceled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The parameter values it cooks: a result whose values are no longer the layer's is redone.
    pub values: serde_json::Map<String, Value>,
    pub rx: std::sync::mpsc::Receiver<texturecraft_engine::Result<texturecraft_engine::link_cmds::CookedMaterial>>,
}

impl Drop for LinkCook {
    fn drop(&mut self) {
        self.canceled.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The engine's refusal to paint while a bake shows (translated where it is toasted).
pub const PAINTING_PAUSED: &str = i18n::n_("Painting is paused while a bake is shown. Cancel the bake or wait for it to finish.");

/// The browser build's notice when WebGPU is unavailable and wgpu fell back to WebGL2.
pub const WEBGL_NOTICE: &str = i18n::n_(
    "WebGPU isn't available in this browser, so TextureCraft runs on WebGL2: Rendering mode path traces on the CPU (slower) and large texture sets show at reduced resolution. A current Chrome, Edge or Safari has WebGPU.",
);

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: f64,
    pub show_log: bool,
}

pub struct TextureCraftApp {
    pub session: Session,
    /// A smooth move to a saved camera in progress.
    camera_glide: Option<CameraGlide>,
    /// Yaw and pitch an Alt+Shift drag has reached before snapping to 90° steps (see `viewport`).
    orbit_snap: Option<(f32, f32)>,
    /// The 2D view's turn an Alt+Shift drag has reached before snapping to steps (see `viewport`).
    uv_rotation_snap: Option<f32>,
    /// Frame the 2D view at its next draw (F): pan, zoom and rotation back, on tile 1001 in a UV tile set
    uv_view_reset: bool,
    pub ui: UiState,
    /// Where `ui` is saved between runs
    ui_store: ui_state_store::UiStateStore,
    pub gpu: Option<gpu::GpuLink>,
    pub control: Option<std::sync::mpsc::Receiver<ControlRequest>>,
    pub toasts: Vec<Toast>,
    /// Texture sets the GPU could not hold, and the text of the one sticky toast naming them.
    pub(crate) gpu_memory: (Vec<String>, Option<String>),
    pub bake: Option<BakeJob>,
    /// What the bake running now has done so far, for the summary when it ends
    pub bake_report: bake_report::BakeReport,
    /// The Import PBR textures dialog while it is open.
    pub pbr_import: Option<pbr_import::PbrDialog>,
    pub resize: Option<ResizeJob>,
    /// Latest resize requested while a cancelled worker releases its resources.
    queued_resize: Option<(usize, u32)>,
    /// The project import's windows and the conversion running behind them.
    pub import: Option<import_ui::ImportUi>,
    pub(crate) import_presentation: Option<import_ui::Presentation>,
    pub link_cooks: Vec<LinkCook>,
    #[cfg(not(target_arch = "wasm32"))]
    link_imports: Vec<link_import::LinkImport>,
    pub thumbs: thumbs::Thumbs,
    /// The Layers panel's thumbnails and folded groups.
    pub layer_panel: layer_panel::LayerPanel,
    /// Last frame's viewport rect (logical points), for scripted input.
    pub viewport_rect: egui::Rect,
    pub fps: f32,
    /// User smart materials found in the library folder (refreshed on demand).
    pub library: Option<Vec<std::path::PathBuf>>,
    /// Rendering mode's path tracer.
    pub render: render_mode::RenderView,
    /// Smoothed CPU time spent building the UI per frame (ms).
    pub ui_ms: f32,
    /// Frame timing and the scripted paint test (`debug.*` control methods).
    pub profiler: profile::Profiler,
    /// Where the last 3D stroke ended on the model, scoped to its mesh and texture set, for Shift+click straight lines.
    pub last_paint_point: Option<uv_paint_point::SurfacePaintPoint>,
    /// The last finished 2D stroke's endpoint, scoped to its mesh and texture set for Shift+click lines
    pub last_paint_uv: Option<uv_paint_point::UvPaintPoint>,
    /// The decal layer the next click in the 3D view places.
    pub placing_decal: Option<texturecraft_engine::core::LayerId>,
    /// The Properties popup a right click in a view opens.
    pub properties_popup: Option<properties_popup::PropertiesPopup>,
    /// The color selection the next click in the 3D or 2D view picks an ID color for
    pub picking_id_color: Option<id_color_pick::IdColorPick>,
    /// Whether the next click on the model places the symmetry planes (Pick point in the symmetry settings)
    pub picking_symmetry_plane: bool,
    /// The wgpu state (screenshots read the renderer's targets).
    pub render_state: Option<eframe::egui_wgpu::RenderState>,
    #[cfg(not(target_arch = "wasm32"))]
    graph_accelerator: Option<Arc<texturecraft_render::gpu_graph::GraphAccelerator>>,
    /// CJK fallback fonts for the UI language.
    pub fonts: fonts::UiFonts,
    /// A notice across the top of the window until dismissed (English key, shown translated).
    pub banner: Option<&'static str>,
    /// Who draws the window's title bar (decided at startup, before the window opens).
    pub title_bar: titlebar::TitleBar,
    last_time: f64,
    /// When the next autosave is due (seconds of app time; desktop only).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    next_autosave: f64,
    /// The brightness and accent the style was last built for.
    styled: Option<(theme::Brightness, theme::Accent)>,
    /// Whether the window had focus last frame.
    focused: bool,
    /// The update check and prompt (desktop only).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) updates_rt: updates_ui::Runtime,
    /// Native file dialogs and their continuations (desktop only).
    #[cfg(not(target_arch = "wasm32"))]
    pub file_dialogs: file_dialogs::FileDialogs,
    /// The unsaved-changes dialog's Save is waiting on the Save dialog's answer.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) saving_for_prompt: bool,
    /// That Save dialog saved the project; the prompt goes on at its next frame.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) prompt_saved: bool,
    /// The resolved keymap and its key dispatcher.
    pub(crate) bindings: keymap::Bindings,
    /// The shortcuts Settings > Shortcuts edits, resolved to show shared shortcuts.
    pub(crate) settings_bindings: keymap::Bindings,
}

/// The UI state to start with: the saved one, whose tool settings go to `session`, else the defaults in the OS language
pub(crate) fn start_ui(saved: Option<UiState>, session: &mut Session) -> UiState {
    match saved {
        Some(ui) => {
            ui.tool_settings.apply(&mut session.tool);
            ui
        }
        None => {
            let mut ui = UiState::default();
            ui.prefs.language = i18n::SYSTEM_LANGUAGE.into();
            ui
        }
    }
}

impl TextureCraftApp {
    fn finish_ui_frame(&mut self, started_ms: f64) {
        let ended_ms = now_ms();
        let elapsed_ms = (ended_ms - started_ms) as f32;
        self.ui_ms = if self.ui_ms == 0.0 { elapsed_ms } else { self.ui_ms * 0.9 + elapsed_ms * 0.1 };
        self.profiler.end_frame(ended_ms, elapsed_ms);
    }

    /// The app around `session`, with the saved UI state (or defaults in the OS language) and the GPU viewport.
    pub fn new(cc: &eframe::CreationContext<'_>, mut session: Session) -> TextureCraftApp {
        theme::install(&cc.egui_ctx);

        // Bakes cast their rays on a GPU device of their own (created on the first bake)
        #[cfg(not(target_arch = "wasm32"))]
        texturecraft_engine::bake::install_gpu_tracer(Arc::new(texturecraft_render::gpu_bake::GpuBaker::new()));
        let gpu = cc.wgpu_render_state.as_ref().map(|render_state| gpu::init(render_state, &cc.egui_ctx));
        #[cfg(not(target_arch = "wasm32"))]
        let graph_accelerator = cc
            .wgpu_render_state
            .as_ref()
            .filter(|_| gpu.as_ref().is_some_and(gpu::GpuLink::supports_gpu_layers))
            .map(|render_state| Arc::new(texturecraft_render::gpu_graph::GraphAccelerator::new(&render_state.device, &render_state.queue)));

        // The first run speaks the OS (or browser) language
        let mut ui_store = ui_state_store::UiStateStore::new(ui_state_store::FILE.get().cloned());
        let mut ui = start_ui(ui_store.load(cc.storage), &mut session);

        // No window opens by itself with no project (a saved layout may have left Export open)
        if !session.project_open {
            ui.windows.export = false;
        }

        let next_autosave = autosave_period_seconds(ui.prefs.autosave_minutes);
        ui.prefs.apply_resource_limits();
        ui.prefs.apply_graph_budget();

        let mut app = TextureCraftApp {
            camera_glide: None,
            orbit_snap: None,
            uv_rotation_snap: None,
            uv_view_reset: false,
            session,
            ui,
            ui_store,
            gpu,
            control: None,
            toasts: Vec::new(),
            gpu_memory: (Vec::new(), None),
            bake: None,
            bake_report: Default::default(),
            pbr_import: None,
            resize: None,
            queued_resize: None,
            import: None,
            import_presentation: None,
            link_cooks: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            link_imports: Vec::new(),
            thumbs: thumbs::Thumbs::default(),
            layer_panel: layer_panel::LayerPanel::default(),
            viewport_rect: egui::Rect::NOTHING,
            fps: 0.0,
            ui_ms: 0.0,
            profiler: profile::Profiler::default(),
            library: None,
            render: render_mode::RenderView::default(),
            last_paint_point: None,
            last_paint_uv: None,
            placing_decal: None,
            properties_popup: None,
            picking_id_color: None,
            picking_symmetry_plane: false,
            render_state: cc.wgpu_render_state.clone(),
            #[cfg(not(target_arch = "wasm32"))]
            graph_accelerator,
            fonts: fonts::UiFonts::default(),
            banner: None,
            title_bar: titlebar::TitleBar::default(),
            last_time: 0.0,
            next_autosave,
            styled: None,
            focused: true,
            #[cfg(not(target_arch = "wasm32"))]
            updates_rt: updates_ui::Runtime::default(),
            #[cfg(not(target_arch = "wasm32"))]
            file_dialogs: file_dialogs::FileDialogs::new(cc.egui_ctx.clone()),
            #[cfg(not(target_arch = "wasm32"))]
            saving_for_prompt: false,
            #[cfg(not(target_arch = "wasm32"))]
            prompt_saved: false,
            bindings: keymap::Bindings::default(),
            settings_bindings: keymap::Bindings::default(),
        };

        // The test harness has no window: its file dialogs answer Cancelled, never on the desktop
        #[cfg(not(target_arch = "wasm32"))]
        if cc.winit_window().is_none() {
            app.file_dialogs.without_window();
        }
        app.ui.rename = None;
        app.ui.windows.new_project = false;
        i18n::set_language(&app.ui.prefs.language);
        app.report_ui_storage_warning();
        export_templates::load_library(&mut app, cc.storage);

        #[cfg(not(target_arch = "wasm32"))]
        {
            // The first window of the process fixes it; a new folder applies on restart
            let _ = DATA_FOLDER.set(data_folder(&app.ui.prefs.data_folder));

            // An export folder still at the default follows the data folder
            if app.ui.export.dir == data_folder("").join("export").display().to_string() {
                app.ui.export.dir = user_dir("export").display().to_string();
            }

            let skipped = app.session.load_templates(user_dir("templates"));
            app.session
                .log
                .warnings(texturecraft_engine::log::LIBRARY_SOURCE, skipped.into_iter().map(|warning| format!("Project template skipped: {warning}")));

            // Load presets before control commands can access the session
            for warning in app.session.load_brush_presets(user_dir("brushes")) {
                app.toast(warning, true);
            }
        }

        app
    }

    /// Run an engine command, reporting errors as toasts. Returns the result on success.
    pub fn run(&mut self, id: &str, params: Value) -> Option<Value> {
        self.ui.prefs.apply_resource_limits();
        #[cfg(not(target_arch = "wasm32"))]
        if id == "material.load"
            && self.session.project_open
            && let Some(path) = params.get("path").and_then(Value::as_str)
            && texturecraft_engine::link_cmds::is_link_file(std::path::Path::new(path))
        {
            return self.start_link_import(path, &params);
        }

        // Going to a saved camera or a fixed view glides there; the control channel and CLI jump straight away
        let glides = ["camera.select", "camera.bookmark", "camera.view", "camera.next", "camera.previous", "camera.snapRotate"]
            .iter()
            .any(|prefix| id.starts_with(prefix));
        let before = self.session.camera;
        let glide_from = glides.then_some(before);
        let relative_camera_command = matches!(id, "camera.next" | "camera.previous") || id.starts_with("camera.snapRotate");
        let pending_camera = self.camera_glide.as_ref().filter(|_| relative_camera_command).map(|glide| glide.to);
        if let Some(destination) = pending_camera {
            self.session.camera = destination;
        }

        match texturecraft_engine::execute(&mut self.session, id, &params) {
            Ok(value) => {
                if matches!(id, "brush.preset.store" | "brush.preset.rename" | "brush.preset.delete") {
                    self.thumbs.forget_saved_files();
                }

                // Pending work and thumbnail snapshots belong to the old project
                if matches!(id, "file.open" | "file.new" | "file.close" | "file.importProject" | "project.new" | "project.sample") {
                    self.clear_project_jobs();
                }

                // Schedule the reset only when Painting or Baking draws a 2D view
                if id == "viewport.frame" && self.ui.layout != state::ViewLayout::Only3D && self.ui.mode != state::Mode::Rendering {
                    self.uv_view_reset = true;
                }

                // The empty state has no baking or rendering to show
                if id == "file.close" {
                    self.ui.mode = state::Mode::Painting;
                    self.ui.windows.export = false;
                }

                match glide_from {
                    Some(from) if from != self.session.camera => {
                        self.camera_glide = Some(CameraGlide { from, to: self.session.camera, start: self.last_time });
                        self.session.camera = from;
                    }

                    // Already at the destination: drop a glide an earlier command in the same frame started
                    Some(_) => self.camera_glide = None,
                    None => self.settle_camera_glide(id, before),
                }

                Some(value)
            }
            Err(error) => {
                if pending_camera.is_some() {
                    self.session.camera = before;
                }

                self.toast(error.to_string(), true);
                None
            }
        }
    }

    /// Clear work that belongs to the previous project after replacing or closing it.
    pub(crate) fn clear_project_jobs(&mut self) {
        self.pbr_import = None;
        self.resize = None;
        self.queued_resize = None;
        #[cfg(not(target_arch = "wasm32"))]
        self.link_imports.clear();
        self.link_cooks.clear();
        self.layer_panel = Default::default();
        self.camera_glide = None;
    }

    /// After command `id` ran without starting a glide (the camera was `before`): a projection switch
    /// carries over into a running glide, which would otherwise put back the projection it recorded;
    /// any other command that moved the camera, or named where it should be, ends the glide, which
    /// would put the camera back or carry it on to the glide's old destination.
    pub(crate) fn settle_camera_glide(&mut self, id: &str, before: texturecraft_engine::geom::OrbitCamera) {
        if matches!(id, "camera.perspective" | "camera.orthographic") {
            if let Some(glide) = &mut self.camera_glide {
                glide.from.ortho = self.session.camera.ortho;
                glide.to.ortho = self.session.camera.ortho;
            }
        } else if self.session.camera != before || sets_camera(id) {
            self.camera_glide = None;
        }
    }

    fn report_ui_storage_warning(&mut self) {
        if let Some(warning) = self.ui_store.take_warning() {
            self.session.log.warn("UI", warning.clone());
            self.toast_sticky(warning);
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast { text: text.into(), error, until: self.last_time + if error { 6.0 } else { 3.0 }, show_log: false });
    }

    /// An error toast that stays until the user dismisses it (for a problem that persists and
    /// is reported once, like a texture set the GPU could not hold).
    pub fn toast_sticky(&mut self, text: impl Into<String>) {
        self.toasts.push(Toast { text: text.into(), error: true, until: f64::INFINITY, show_log: false });
    }

    /// Start baking texture sets in the background, one after another. Invalid set indices
    /// show an error without starting a preview or queuing any work.
    pub fn start_bake(&mut self, sets: Vec<usize>) {
        if self.bake.is_some() || sets.is_empty() {
            return;
        }

        // Restore the picker's view before the preview starts following baked maps
        id_color_pick::IdColorPick::end(self);

        if let Err(error) = self.session.bake_preview_begin_for(&sets) {
            self.toast(error.to_string(), true);
            return;
        }

        let total = sets.len();
        self.bake_report = bake_report::BakeReport::started(self.last_time);
        self.bake = Some(BakeJob { queue: sets, current: None, total, mesh_id: self.session.mesh_id });
    }

    /// Re-cook MaterialCraft material layer `layer` in the background. While one cook of the layer
    /// runs no other starts: when it ends with stale values, it is redone with the current ones.
    pub fn cook_link(&mut self, layer: texturecraft_engine::core::LayerId) {
        if self.link_cooks.iter().any(|c| c.layer == layer) {
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        if !self.link_imports.is_empty() || !self.link_cooks.is_empty() {
            self.toast("Another MaterialCraft cook is still running", true);
            return;
        }

        self.ui.prefs.apply_resource_limits();
        match self.session.link_job(layer, &serde_json::Map::new()) {
            Ok(job) => {
                let baked = self.session.doc.project.locate(layer).and_then(|(set, _, _)| self.session.baked(set));
                let baked_inputs = job
                    .input_identifiers
                    .iter()
                    .filter_map(|identifier| texturecraft_engine::core::BakedMap::parse(identifier))
                    .map(|map| link_bake_input::LinkBakeInput::capture(map, baked))
                    .collect();
                let values = job.values.clone();
                let size = job.size;
                let canceled = std::sync::Arc::clone(&job.canceled);
                let rx = spawn_worker("texturecraft-link", move || job.run());
                self.link_cooks.push(LinkCook {
                    layer,
                    project: self.session.project_id,
                    mesh: self.session.mesh_id,
                    size,
                    baked_inputs,
                    canceled,
                    values,
                    rx,
                });
            }
            Err(e) => self.toast(e.to_string(), true),
        }
    }

    fn poll_link_cooks(&mut self, ctx: &egui::Context) {
        if self.link_cooks.is_empty() {
            return;
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        if self.session.stroke_active() {
            return;
        }

        self.link_cooks.retain(|cook| {
            cook.project == self.session.project_id && cook.mesh == self.session.mesh_id && self.session.doc.project.locate(cook.layer).is_some()
        });
        let mut done = Vec::new();
        self.link_cooks.retain(|c| match c.rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => true,
            Ok(result) => {
                let Some((set, _, _)) = self.session.doc.project.locate(c.layer) else {
                    return false;
                };

                let resized = self.session.doc.project.texture_set(set).is_ok_and(|set| set.resolution != c.size);
                let baked = self.session.baked(set);
                let stale = resized || c.baked_inputs.iter().any(|input| !input.is_current(baked));
                done.push((c.layer, c.values.clone(), stale, result));
                false
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                done.push((c.layer, c.values.clone(), false, Err(texturecraft_engine::EngineError::Other("the MaterialCraft cook stopped".into()))));
                false
            }
        });
        for (layer, values, stale, result) in done {
            let current = self.session.link_values(layer);
            match (result, current) {
                // The layer can disappear while a cook is pending
                (_, Err(_)) => {}
                (result, Ok(now)) if now != values || stale => {
                    drop(result);
                    self.cook_link(layer);
                }
                (Ok(material), Ok(_)) => {
                    if let Err(e) = self.session.apply_link_cook(layer, &material, true) {
                        self.toast(e.to_string(), true);
                    }
                }
                (Err(e), Ok(_)) => self.toast(e.to_string(), true),
            }
        }
    }

    /// Show at most one streamed map per frame, then apply the set as one undo step after
    /// its map channel closes. Closing the preview alone does not stop the background bake.
    fn poll_bake(&mut self, ctx: &egui::Context) {
        // Leaving Bake mode puts the view mode back; the bake goes on
        if self.session.bake_preview_active() && self.ui.mode != state::Mode::Baking {
            self.session.bake_preview_end();
        }

        let Some(job) = self.bake.as_mut() else {
            // A preview whose bake is gone must not keep the view
            self.session.bake_preview_end();
            return;
        };

        if job.mesh_id != self.session.mesh_id {
            // Discard results for the replaced mesh
            if let Some(running) = &job.current {
                running.progress.cancel.store(true, Ordering::Relaxed);
            }
            self.bake = None;
            return;
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        if let Some(running) = &job.current {
            let set_index = running.set_index;

            // Wait for the sender to close so the last map also gets a preview frame
            match running.maps.try_recv() {
                Ok(map) => {
                    self.session.bake_preview_map(set_index, map);
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
            }

            match running.receiver.try_recv() {
                Ok(Ok(output)) => {
                    let maps_baked = running.maps_done.load(Ordering::Relaxed);
                    job.current = None;

                    match self.session.apply_streamed_bake(set_index, output) {
                        Ok(()) => self.bake_report.map_count += maps_baked,
                        Err(error) => self.bake_failed(set_index, &error.to_string()),
                    }
                }
                Ok(Err(error)) => {
                    // The worker's error ends the whole bake; the maps of the sets before it stay
                    self.bake_failed(set_index, &error.to_string());
                    self.cancel_bake();
                    self.finish_bake_report();
                    return;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.bake_failed(set_index, i18n::tr("The bake worker stopped"));
                    self.cancel_bake();
                    self.finish_bake_report();
                    return;
                }
            }
        }

        let Some(job) = self.bake.as_mut() else { return };

        if job.queue.is_empty() {
            self.bake = None;
            self.session.bake_preview_end();
            self.finish_bake_report();
            return;
        }

        let set_index = job.queue.remove(0);
        self.session.bake_preview_set_current(set_index);
        match self.session.bake_task(set_index) {
            Ok(task) => {
                let progress = Arc::new(Progress::new());
                let worker_progress = progress.clone();
                let maps_total = task.maps().len();
                let maps_done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let worker_done = maps_done.clone();
                let (sender, maps) = std::sync::mpsc::channel();
                let receiver = spawn_worker("texturecraft-bake", move || {
                    task.run_streaming(&worker_progress, &mut |map| {
                        worker_done.fetch_add(1, Ordering::Relaxed);
                        let _ = sender.send(map);
                    })
                });

                if let Some(job) = self.bake.as_mut() {
                    job.current = Some(RunningBake { set_index, progress, receiver, maps, maps_done, maps_total });
                }
            }
            Err(error) => self.bake_failed(set_index, &error.to_string()),
        }
    }

    /// Note a texture set whose bake failed: one Log line, counted in the summary
    fn bake_failed(&mut self, set_index: usize, error: &str) {
        self.session.log.error("Baker", format!("Bake of texture set {set_index} failed: {error}"));
        self.bake_report.failed_set_count += 1;
    }

    /// The toast that ends a bake (maps baked, time, failures) and its line in the Log
    fn finish_bake_report(&mut self) {
        let (text, line, failed) = self.bake_report.summary(self.last_time);
        if failed {
            self.session.log.warn("Baker", line);
        } else {
            self.session.log.info("Baker", line);
        }

        self.toast(text, failed);
        if let Some(toast) = self.toasts.last_mut() {
            toast.show_log = failed;
        }
    }

    /// Cancel the active worker, discard queued sets and restore the view. The current set's
    /// preview maps are discarded; previously applied sets keep their results and undo steps.
    pub fn cancel_bake(&mut self) {
        let Some(job) = self.bake.take() else { return };

        if let Some(running) = job.current {
            running.progress.cancel.store(true, Ordering::Relaxed);
        }

        // Only the current set is uncommitted; completed sets remain applied
        self.session.bake_preview_end();
    }

    /// Resize texture set `ts` to `resolution` on a worker thread (the document changes when
    /// it is done, as one undo step).
    pub fn start_resize(&mut self, ts: usize, resolution: u32) {
        if let Some(job) = &self.resize {
            if job.progress.is_cancelled() {
                self.queued_resize = Some((ts, resolution));
            }
            return;
        }

        match self.session.resize_task(ts, resolution) {
            Ok(task) => {
                let progress = Arc::new(texturecraft_engine::ResizeProgress::default());
                let p = progress.clone();
                #[cfg(not(target_arch = "wasm32"))]
                let gpu = self
                    .render_state
                    .as_ref()
                    .filter(|_| self.ui.prefs.gpu_compositing && self.gpu.as_ref().is_some_and(gpu::GpuLink::supports_gpu_layers))
                    .map(|state| (state.device.clone(), state.queue.clone()));
                let rx = spawn_worker("texturecraft-resize", move || {
                    #[cfg(not(target_arch = "wasm32"))]
                    return resize_for_view(task, &p, gpu);
                    #[cfg(target_arch = "wasm32")]
                    return task.run_for_browser(&p);
                });
                self.resize = Some(ResizeJob { progress, rx, project: self.session.project_id });
            }
            Err(e) => self.toast(e.to_string(), true),
        }
    }

    fn cancel_resize(&mut self) {
        self.queued_resize = None;
        if let Some(job) = &self.resize {
            job.progress.cancel();
            // Its result cannot commit, even if a GPU dispatch must drain before it returns.
            self.toast(i18n::tr("Texture set resize cancelled"), false);
        }
    }

    /// Apply a finished resize (after any stroke in progress ends).
    fn poll_resize(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.resize else { return };

        // Nothing to resize without a project (a result that arrives after Close is dropped)
        if !self.session.project_open || self.session.project_id != job.project {
            self.resize = None;
            self.queued_resize = None;
            return;
        }

        ctx.request_repaint_after(std::time::Duration::from_millis(30));
        if self.session.stroke_active() {
            return;
        }

        let result = match job.rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(texturecraft_engine::EngineError::Other(i18n::tr("The resize worker stopped").into())),
        };
        let cancelled = job.progress.is_cancelled();
        self.resize = None;
        if cancelled {
            if let Some((ts, resolution)) = self.queued_resize.take() {
                self.start_resize(ts, resolution);
            }
            return;
        }
        if let Ok(output) = &result
            && !output.matches_session(&self.session)
        {
            let (ts, resolution) = output.target();
            self.start_resize(ts, resolution);
            return;
        }
        match result.and_then(|out| {
            let target = out.target().0;
            self.session.apply_resize(out)?;
            Ok(target)
        }) {
            Ok(target) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.replace_resized_view(self.session.project_id, target);
                }
            }
            Err(error) => self.toast(error.to_string(), true),
        }
    }

    /// The map being baked and its position among the queued maps and texture sets.
    pub fn bake_status(&self) -> Option<String> {
        let job = self.bake.as_ref()?;
        let stage = self.session.bake_preview.as_ref()?.stage()?;
        let map = i18n::tr(i18n::baked_map_label(stage.map));
        let (n, total) = (stage.position, stage.total);
        if job.total > 1 {
            let set = job.total - job.queue.len();
            return Some(i18n::trf(
                "Baking {map} ({n}/{total}), texture set {set} of {sets}",
                &[("map", &map), ("n", &n), ("total", &total), ("set", &set.max(1)), ("sets", &job.total)],
            ));
        }

        Some(i18n::trf("Baking {map} ({n}/{total})", &[("map", &map), ("n", &n), ("total", &total)]))
    }

    /// Bake progress 0..1 across all queued sets.
    pub fn bake_progress(&self) -> Option<f32> {
        let job = self.bake.as_ref()?;
        let done = job.total - job.queue.len() - usize::from(job.current.is_some());

        // Each map is a bake of its own: its progress runs from 0 to 1 within the set's share
        let current = job
            .current
            .as_ref()
            .map(|running| (running.maps_done.load(Ordering::Relaxed) as f32 + running.progress.fraction()) / running.maps_total.max(1) as f32)
            .unwrap_or(0.0);
        Some(((done as f32 + current) / job.total.max(1) as f32).clamp(0.0, 1.0))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn resize_for_view(
    task: texturecraft_engine::ResizeTask,
    progress: &texturecraft_engine::ResizeProgress,
    gpu: Option<(texturecraft_render::wgpu::Device, texturecraft_render::wgpu::Queue)>,
) -> texturecraft_engine::Result<texturecraft_engine::ResizeOutput> {
    use texturecraft_render::gpu_resize::PaintResampler;
    let mut resampler = None;
    task.run_for_view(
        progress,
        |image, size, empty| {
            let (device, queue) = gpu.as_ref()?;
            Some(match resampler.get_or_insert_with(|| PaintResampler::new(device, queue)) {
                Ok(resampler) => resampler.resize(image, size, empty, || progress.is_cancelled()),
                Err(error) => Err(error.clone()),
            })
        },
        |inputs| gpu.is_some() && texturecraft_render::gpu_composite::CompositeProgram::try_compile_inputs(inputs, None).is_ok(),
    )
}

/// Run `f` on a worker thread named `name`; its result arrives on the returned channel.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_worker<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> std::sync::mpsc::Receiver<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        let _ = tx.send(f());
    });
    if let Err(e) = spawned {
        // The receiver reports a stopped worker
        log::warn!("could not start {name}: {e}");
    }
    rx
}

#[cfg(target_arch = "wasm32")]
fn spawn_worker<T: Send + 'static>(_name: &str, f: impl FnOnce() -> T + Send + 'static) -> std::sync::mpsc::Receiver<T> {
    // Web builds run the work synchronously because threads are unavailable
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = tx.send(f());
    rx
}

fn autosave_period_seconds(minutes: u32) -> f64 {
    f64::from(minutes) * 60.0
}

impl TextureCraftApp {
    pub(crate) fn material_graph_accelerator(&self) -> Option<Arc<dyn texturecraft_engine::compose::graphs::GraphAccelerator>> {
        #[cfg(not(target_arch = "wasm32"))]
        if self.ui.prefs.gpu_compositing {
            return self.graph_accelerator.clone().map(|backend| backend as _);
        }
        None
    }

    fn reschedule_autosave_if_changed(&mut self, previous_minutes: u32) {
        let minutes = self.ui.prefs.autosave_minutes;
        if minutes != previous_minutes {
            self.next_autosave = self.last_time + autosave_period_seconds(minutes);
        }
    }

    /// Every `prefs.autosave_minutes`, write a modified project to the autosave folder (encoded
    /// and written on a worker thread; the project data is shared copy-on-write).
    #[cfg(not(target_arch = "wasm32"))]
    fn autosave(&mut self, now: f64) {
        let period = autosave_period_seconds(self.ui.prefs.autosave_minutes);

        // The clock starts at the first unsaved change
        if period <= 0.0 || !self.session.modified {
            self.next_autosave = now + period;
            return;
        }

        if now < self.next_autosave {
            return;
        }

        let Some(mut reservation) = texturecraft_engine::formats::RamReservation::try_for_save() else {
            return;
        };

        self.next_autosave = now + period;
        let dir = user_dir("autosave");
        let path = dir.join(format!(
            "{}.{}",
            texturecraft_engine::formats::export::sanitize_file_stem(&self.session.doc.project.name),
            texturecraft_engine::core::PROJECT_EXT
        ));
        let snapshot = self.session.project_snapshot();
        let spawned = std::thread::Builder::new().name("texturecraft-autosave".into()).spawn(move || {
            let result =
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string()).and_then(|_| reservation.save(&path, &snapshot).map_err(|error| error.to_string()));
            drop(snapshot);
            drop(reservation);

            match result {
                Ok(()) => log::info!("autosaved {}", path.display()),
                Err(e) => log::warn!("autosave to {} failed: {e}", path.display()),
            }
        });
        if let Err(e) = spawned {
            log::warn!("could not start autosave: {e}");
        }
    }
}

/// Whether command `id` puts the camera at a place it names (a view, a saved camera, explicit values or
/// framing), so a glide toward an earlier destination must not go on even when the camera is already there
fn sets_camera(id: &str) -> bool {
    matches!(id, "camera.set" | "camera.orbit" | "camera.select" | "camera.next" | "camera.previous")
        || ["camera.view.", "camera.bookmark", "camera.snapRotate", "camera.rotation", "viewport.frame"].iter().any(|prefix| id.starts_with(prefix))
}

/// A smooth move of the viewport camera (see [`TextureCraftApp::run`]).
struct CameraGlide {
    from: texturecraft_engine::geom::OrbitCamera,
    to: texturecraft_engine::geom::OrbitCamera,
    start: f64,
}

const GLIDE_SECONDS: f64 = 0.35;

impl TextureCraftApp {
    /// Advance the camera glide by one frame.
    fn step_camera_glide(&mut self, ctx: &egui::Context) {
        let Some(g) = &self.camera_glide else { return };
        let t = ((self.last_time - g.start) / GLIDE_SECONDS).clamp(0.0, 1.0) as f32;
        let e = t * t * (3.0 - 2.0 * t);
        let (a, b) = (g.from, g.to);
        let mix = |x: f32, y: f32| x + (y - x) * e;
        let mut c = b;
        c.target = a.target.lerp(b.target, e);
        c.yaw = mix(a.yaw, b.yaw);
        c.orient = a.orient.slerp(b.orient, e);
        c.pitch = mix(a.pitch, b.pitch);
        c.distance = mix(a.distance, b.distance);
        c.fov = mix(a.fov, b.fov);

        // Switch orthographic mode at the end because projections cannot blend
        c.ortho = if t >= 1.0 { b.ortho } else { a.ortho };
        self.session.camera = c;
        self.session.revision = self.session.revision.wrapping_add(1);
        if t >= 1.0 {
            self.camera_glide = None;
        }
        ctx.request_repaint();
    }
}

impl eframe::App for TextureCraftApp {
    /// The browser never tells eframe the page is closing, so save often there.
    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(if cfg!(target_arch = "wasm32") { 5 } else { 30 })
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.ui.tool_settings = texturecraft_engine::ToolSettings::of(&self.session.tool);
        self.ui_store.save(&self.ui, storage);
        self.report_ui_storage_warning();
        export_templates::save_library(self, storage);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let t0 = now_ms();
        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_time) as f32;
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + (1.0 / dt) * 0.1;
        }
        self.last_time = now;
        let style = (self.ui.prefs.brightness, self.ui.prefs.accent);
        if self.styled != Some(style) {
            theme::apply(ctx, style.0, style.1);
            self.styled = Some(style);
        }
        self.step_camera_glide(ctx);
        self.ui.prefs.apply_resource_limits();
        self.session.doc.graphs.set_accelerator(self.material_graph_accelerator());
        #[cfg(not(target_arch = "wasm32"))]
        {
            let paint_view = self.session.quick_mask.editing
                || !matches!(self.session.view, texturecraft_engine::ViewMode::MeshMap(_) | texturecraft_engine::ViewMode::Image);
            self.session.brush_device = if self.ui.prefs.gpu_compositing && self.ui.mode == state::Mode::Painting && paint_view && self.session.solo.is_none() {
                self.gpu.as_ref().and_then(|gpu| gpu.brush_device(&self.session))
            } else {
                None
            };
        }
        control::poll(self, ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.run_file_dialog_answers();
        web_files::poll(self);
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());

        // A folder, a zip or several PBR maps dropped together open the PBR import dialog
        #[cfg(not(target_arch = "wasm32"))]
        let consumed = if !dropped.is_empty() {
            let paths: Vec<std::path::PathBuf> = dropped.iter().map(|f| f.path().to_path_buf()).collect();
            pbr_import::dropped(self, &paths)
        } else {
            Vec::new()
        };
        #[cfg(target_arch = "wasm32")]
        let consumed: Vec<usize> = Vec::new();
        for (index, file) in dropped.iter().enumerate() {
            if consumed.binary_search(&index).is_err() {
                web_files::dropped(self, ctx, file);
            }
        }
        self.poll_bake(ctx);
        self.poll_resize(ctx);
        self.poll_link_cooks(ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.poll_link_imports(ctx);
        self.thumbs.sync_images(&self.session.images, self.session.revision);

        // The library shelf rescans when the window comes back (MaterialCraft may have sent a material)
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        if focused && !self.focused {
            self.library = None;
        }
        self.focused = focused;
        let scale = self.ui.prefs.ui_scale.clamp(0.5, 3.0);
        if (ctx.zoom_factor() - scale).abs() > 1e-3 {
            ctx.set_zoom_factor(scale);
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.autosave(now);
        i18n::set_language(&self.ui.prefs.language);
        #[cfg(target_arch = "wasm32")]
        {
            keymap::sync_captured(&self.ui.keymap);
            web_files::sync_unsaved(self);
        }
        self.fonts.update(ctx, i18n::current().code, self.ui.windows.settings);
        #[cfg(not(target_arch = "wasm32"))]
        updates_ui::tick(self, ctx);
        self.bindings.sync(&self.ui.keymap, craft_keymap::egui::platform(ctx));
        menus::shortcuts(self, ctx);

        // The Path tool's curve repaints (a preview while placed) as its points, brush or material change
        if let Err(e) = self.session.path_live() {
            self.session.cancel_path();
            self.toast(e.to_string(), true);
        }
        profile::drive(self, ctx);
        self.toasts.retain(|t| t.until > now);
        self.profiler.logic_ms = (now_ms() - t0) as f32;

        // Last, so it sees every close request this frame's work made
        windows::close_request(self, ctx);
    }

    #[cfg_attr(target_arch = "wasm32", allow(unused_variables))]
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let started_ms = now_ms();

        if import_ui::show_preview(self, ui) {
            windows::unsaved(self, &ctx);
            #[cfg(not(target_arch = "wasm32"))]
            self.file_dialogs.show_pending(frame);
            self.finish_ui_frame(started_ms);
            return;
        }

        // Baking and rendering need a project (a saved layout may start in one of them)
        if !self.session.project_open {
            self.ui.mode = state::Mode::Painting;
        }

        if !self.ui.hide_ui {
            titlebar::show(self, ui);
            banner(self, ui);
            toolbar::status_bar(self, ui);
            if self.ui.toolbar_tools {
                toolbar::tool_shelf(self, ui);
            }
            self.ui.sync_mode_dock();
            if self.ui.mode != state::Mode::Baking && self.ui.toolbar_docks {
                toolbar::rail(self, ui);
            }
        }

        let hide_ui = self.ui.hide_ui;
        let tokens = theme::Tokens::get(&ctx);

        // The dock keeps the cards' gutter from the bars and rails around it
        let panel_frame = if hide_ui { egui::Frame::NONE.fill(tokens.viewport) } else { egui::Frame::NONE.fill(tokens.chrome).inner_margin(theme::GUTTER) };
        egui::CentralPanel::default().frame(panel_frame).show(ui, |ui| if hide_ui { viewport::central(self, ui) } else { docking::show(self, ui) });
        windows::show(self, &ctx);

        // Finish after the dock and windows have requested their thumbnails
        self.layer_panel.end(&ctx, &self.session);

        #[cfg(not(target_arch = "wasm32"))]
        updates_ui::show(self, &ctx);
        toasts(self, &ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.file_dialogs.show_pending(frame);

        self.finish_ui_frame(started_ms);
    }
}

/// The data folder chosen in Settings, fixed at startup (unset: the default).
#[cfg(not(target_arch = "wasm32"))]
static DATA_FOLDER: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// The data folder for a `Prefs::data_folder` value: that path when it is absolute, otherwise
/// `Documents/TextureCraft` in the user's home folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn data_folder(pref: &str) -> std::path::PathBuf {
    let chosen = std::path::Path::new(pref.trim());
    if chosen.is_absolute() {
        return chosen.to_path_buf();
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    std::path::Path::new(&home).join("Documents").join("TextureCraft")
}

/// The data folder in use (Settings › Folders › Data folder).
#[cfg(not(target_arch = "wasm32"))]
pub fn data_dir() -> std::path::PathBuf {
    DATA_FOLDER.get().cloned().unwrap_or_else(|| data_folder(""))
}

/// `<sub>` in the data folder in use (Settings › Folders › Data folder).
#[cfg(not(target_arch = "wasm32"))]
pub fn user_dir(sub: &str) -> std::path::PathBuf {
    data_dir().join(sub)
}

/// Folder holding the user's smart materials.
#[cfg(not(target_arch = "wasm32"))]
pub fn library_dir() -> std::path::PathBuf {
    user_dir("library")
}

/// Milliseconds since the first call (0 on the web, where `std::time` is unavailable).
pub fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.0
    }
    #[cfg(target_arch = "wasm32")]
    0.0
}

/// The dismissable notice under the menu bar.
fn banner(app: &mut TextureCraftApp, ui: &mut egui::Ui) {
    let Some(text) = app.banner else { return };
    let tk = theme::Tokens::get(ui.ctx());
    egui::Panel::top("banner").show_separator_line(false).frame(egui::Frame::NONE.fill(tk.row_selected).inner_margin(egui::Margin::symmetric(10, 5))).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(egui_phosphor::regular::INFO).color(tk.accent));

                // Leave room for the banner's dismiss button
                let width = (ui.available_width() - 28.0).max(80.0);
                ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(i18n::tr(text)).color(tk.text_strong)).wrap());
                });
                if widgets::icon_button(ui, egui_phosphor::regular::X, i18n::tr("Dismiss"), false, 18.0).clicked() {
                    app.banner = None;
                }
            });
        },
    );
}

fn toasts(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }

    // Repaint while a toast has a deadline
    if app.toasts.iter().any(|t| t.until.is_finite()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    let tk = theme::Tokens::get(ctx);
    let mut dismissed = None;
    let mut show_log = false;

    // Limit toast dimensions so the dismiss button stays visible
    let screen = ctx.content_rect().size();
    let max_width = (screen.x - 52.0).clamp(120.0, 360.0);
    let max_text_height = (screen.y * 0.35).clamp(48.0, 220.0);
    egui::Area::new(egui::Id::unique("toasts")).anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -36.0)).show(ctx, |ui| {
        for (i, t) in app.toasts.iter().enumerate() {
            egui::Frame::NONE
                .fill(tk.panel)
                .stroke(egui::Stroke::new(1.0, if t.error { tk.danger } else { tk.accent }))
                .corner_radius(crate::theme::CARD_RADIUS)
                .shadow(ui.visuals().popup_shadow)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.set_max_width(max_width);
                    let sticky = t.until.is_infinite();
                    ui.horizontal_top(|ui| {
                        // The text wraps in the width the dismiss button leaves
                        let width = if sticky { max_width - 28.0 } else { max_width };
                        ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            egui::ScrollArea::vertical().id_salt(("toast", i)).max_height(max_text_height).show(ui, |ui| {
                                ui.set_max_width(width);
                                ui.add(egui::Label::new(egui::RichText::new(&t.text).color(tk.text_strong)).wrap());
                            });
                            if t.show_log && ui.link(i18n::tr("Open Log")).clicked() {
                                show_log = true;
                                dismissed = Some(i);
                            }
                        });
                        if sticky && widgets::icon_button(ui, egui_phosphor::regular::X, i18n::tr("Dismiss"), false, 18.0).clicked() {
                            dismissed = Some(i);
                        }
                    });
                });
            ui.add_space(4.0);
        }
    });
    if let Some(i) = dismissed.filter(|i| *i < app.toasts.len()) {
        app.toasts.remove(i);
    }

    if show_log {
        app.ui.log_channel = log_panel::LogChannel::General;
        crate::rail_popup::open(app, crate::docking::Panel::Log);
    }
}
