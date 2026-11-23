//! TextureCraft desktop app.
//!
//! Usage: `texturecraft [project.tcraft | mesh.glb] [--control <port>] [--empty] [--version]`
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod control_server;
mod gpu;
mod window_frame;

use texturecraft_engine::Session;
use texturecraft_ui_egui::titlebar::TitleBar;
use texturecraft_ui_egui::ui_state_store;
use texturecraft_ui_egui::{TextureCraftApp, UiState};

/// The app name eframe keeps the settings under (`eframe::storage_dir`).
const APP_NAME: &str = "TextureCraft";

/// The window's size when it opens, in points.
const DEFAULT_SIZE: [f32; 2] = [1600.0, 960.0];
/// The smallest size the window can be resized to, in points.
const MIN_SIZE: [f32; 2] = [960.0, 600.0];

const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.texturecraft.png");

fn version() -> String {
    let sha = env!("TEXTURECRAFT_BUILD_SHA");
    let short = sha.get(..7).unwrap_or(sha);
    let date = env!("TEXTURECRAFT_BUILD_DATE");
    match (short.is_empty(), date.is_empty()) {
        (true, true) => format!("TextureCraft {}", texturecraft_engine::VERSION),
        (true, false) => format!("TextureCraft {} ({date})", texturecraft_engine::VERSION),
        _ => format!("TextureCraft {} ({short} {date})", texturecraft_engine::VERSION),
    }
}

struct Args {
    file: Option<String>,
    control: Option<u16>,
    /// With `--control`: start with an empty material ball instead of the sample project
    empty: bool,
    gpu: Option<String>,
    power_pref: Option<eframe::wgpu::PowerPreference>,
}

fn parse_args() -> Result<Option<Args>, String> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from<I: IntoIterator<Item = String>>(it: I) -> Result<Option<Args>, String> {
    let mut args = Args { file: None, control: None, empty: false, gpu: None, power_pref: None };
    let mut it = it.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--version" | "-V" => {
                println!("{}", version());
                return Ok(None);
            }
            "--help" | "-h" => {
                println!(
                    "{}\n\nUsage: texturecraft [project.tcraft | mesh file] [--control <port>] [--gpu <name>] [--power-pref <high|low|none>] [--empty]",
                    version()
                );
                return Ok(None);
            }
            "--control" => {
                let p = it.next().ok_or("--control needs a port")?;
                args.control = Some(p.parse().map_err(|_| format!("invalid port `{p}`"))?);
            }
            "--gpu" => {
                let g = it.next().ok_or("--gpu needs a GPU name or pattern")?;
                args.gpu = Some(g);
            }
            "--power-pref" => {
                let p = it.next().ok_or("--power-pref needs high, low, or none")?;
                args.power_pref = Some(gpu::parse_power_preference(&p)?);
            }
            "--empty" => args.empty = true,
            f if !f.starts_with('-') => args.file = Some(f.to_string()),
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    Ok(Some(args))
}

fn initial_session(args: &Args) -> Session {
    if let Some(path) = &args.file {
        match texturecraft_engine::file_cmds::open_path(path) {
            Ok(session) => return session,
            Err(error) => eprintln!("texturecraft: could not open {path}: {error}"),
        }
    }

    // Control runs retain their sample project; interactive runs start empty
    if args.control.is_some() {
        if !args.empty
            && let Ok(session) = texturecraft_engine::sample_session(2048, 512)
        {
            return session;
        }

        return match Session::new_project("Untitled", texturecraft_engine::geom::primitives::material_ball(), "builtin:material_ball", None, 2048) {
            Ok(session) => session,
            Err(error) => {
                eprintln!("texturecraft: {error}");
                std::process::exit(1);
            }
        };
    }

    match Session::closed() {
        Ok(session) => session,
        Err(error) => {
            eprintln!("texturecraft: {error}");
            std::process::exit(1);
        }
    }
}

fn icon() -> Option<egui::IconData> {
    let img = craft_image_codecs::decode(ICON_PNG).ok()?;
    Some(egui::IconData { rgba: img.to_rgba8(), width: img.width(), height: img.height() })
}

/// Windows shows a window's big icon (`ICON_BIG`) in the taskbar and Alt+Tab; eframe sets only the
/// small one, so the taskbar would show a generic icon. Set the big one too.
#[cfg(windows)]
fn set_taskbar_icon(w: &winit::window::Window) {
    use winit::platform::windows::WindowExtWindows as _;
    // An icon that can't be made leaves the generic one: nothing else depends on it.
    if let Some(i) = icon().and_then(|i| winit::window::Icon::from_rgba(i.rgba, i.width, i.height).ok()) {
        w.set_taskbar_icon(Some(i));
    }
}

/// Preferences needed before opening the initial project or window, with legacy storage fallback.
fn saved_prefs(ui_file: Option<&std::path::Path>, persistence_path: Option<&std::path::Path>) -> texturecraft_ui_egui::state::Prefs {
    if let Some(ui) = ui_file.and_then(ui_state_store::read) {
        return ui.prefs;
    }

    let file = persistence_path.map(std::path::Path::to_path_buf).or_else(|| eframe::storage_dir(APP_NAME).map(|folder| folder.join("app.ron")));
    let saved = file.and_then(|file| std::fs::read_to_string(file).ok());
    let entries = saved.and_then(|text| ron::from_str::<std::collections::HashMap<String, String>>(&text).ok());
    entries.and_then(|entries| ron::from_str::<UiState>(entries.get("ui")?).ok()).map(|ui| ui.prefs).unwrap_or_default()
}

/// The app with the window chrome that needs the native window: the Windows borderless guard, and
/// on macOS the traffic lights and AppKit's `movable` flag.
struct Shell {
    app: TextureCraftApp,
    window_frame: Option<window_frame::WindowFrameFn>,
    #[cfg(target_os = "macos")]
    movable: Option<bool>,
}

impl eframe::App for Shell {
    fn auto_save_interval(&self) -> std::time::Duration {
        self.app.auto_save_interval()
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.save(storage);
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Restore the frameless style after winit changes the window flags
        if let Some(guard) = self.window_frame.as_mut() {
            guard();
        }

        // The app's own hook: a drag holding the pointer reads its moves from it
        self.app.raw_input_hook(ctx, raw);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app.logic(ctx, frame);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.app.ui(ui, frame);
        #[cfg(target_os = "macos")]
        texturecraft_ui_egui::titlebar::sync_native(&self.app.title_bar, ui, frame, &mut self.movable);
    }
}

/// A script driving the app gets no network check and no update prompt over its window.
fn checks_for_updates_at_start(control: Option<u16>) -> bool {
    control.is_none()
}

fn main() {
    // Windows: the update helper is a copy of this program started with `--finish-update`
    #[cfg(windows)]
    if let Some(result) = craft_update::self_update::finish(&texturecraft_ui_egui::UPDATE_APP, &std::env::args_os().skip(1).collect::<Vec<_>>()) {
        std::process::exit(i32::from(result.is_err()));
    }

    // Windows: the eyedropper's pointer keeper (screen_pick) is a copy of this program started with its own argument
    #[cfg(windows)]
    if std::env::args_os().nth(1).is_some_and(|arg| arg == texturecraft_ui_egui::screen_pick::KEEPER_ARG) {
        std::process::exit(texturecraft_ui_egui::screen_pick::keep_pointer());
    }

    let args = match parse_args() {
        Ok(Some(a)) => a,
        Ok(None) => return,
        Err(e) => {
            eprintln!("texturecraft: {e}");
            std::process::exit(2);
        }
    };

    // Errors before the window opens speak the system's language; the app then applies the saved one
    texturecraft_ui_egui::i18n::set_language(texturecraft_ui_egui::i18n::os_language());
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A message box needs the main thread on macOS; other threads' panics are only logged
        fatal(&format!("internal error: {info}"), std::thread::current().name() == Some("main"));
        previous_hook(info);
    }));

    // Scripted runs use temporary files so they cannot overwrite saved settings
    let persistence_path =
        std::env::var_os("TEXTURECRAFT_NO_PREFS").map(|_| std::env::temp_dir().join(format!("texturecraft-noprefs-{}.ron", std::process::id())));
    let ui_file =
        persistence_path.as_ref().map(|path| path.with_extension("json")).or_else(|| eframe::storage_dir(APP_NAME).map(|folder| folder.join("ui.json")));
    if let Some(file) = &ui_file {
        let _ = ui_state_store::FILE.set(file.clone());
    }
    let prefs = saved_prefs(ui_file.as_deref(), persistence_path.as_deref());

    // The worker pool's size can only be set before its first job
    prefs.apply_work_threads();
    prefs.apply_resource_limits();
    let session = initial_session(&args);
    let title_bar = TitleBar::decide(prefs.system_title_bar);
    let mut viewport = egui::ViewportBuilder::default().with_title(APP_NAME).with_inner_size(DEFAULT_SIZE).with_min_inner_size(MIN_SIZE);
    if let Some(i) = icon() {
        viewport = viewport.with_icon(i);
    }
    let viewport = title_bar.viewport(viewport);
    let mut options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, persistence_path, ..Default::default() };
    if let Err(e) = gpu::configure(&mut options, args.gpu, args.power_pref) {
        fatal(&e.to_string(), true);
        std::process::exit(2);
    }
    let control = args.control;
    let result = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            #[cfg(windows)]
            if let Some(w) = cc.winit_window() {
                set_taskbar_icon(w);
            }

            // winit's Quit ends the process without asking; this menu's asks like the close button
            #[cfg(target_os = "macos")]
            if let Err(e) = craft_menu::mac::install_app_menu(&cc.egui_ctx, APP_NAME) {
                log::warn!("the app menu could not be installed: {e}");
            }
            let window_frame = window_frame::service(cc, matches!(title_bar, TitleBar::App(_)));
            let mut app = TextureCraftApp::new(cc, session);
            app.title_bar = title_bar;
            app.set_startup_update_check(checks_for_updates_at_start(control));
            if let Some(port) = control {
                app.control = Some(control_server::start(port, cc.egui_ctx.clone()));
            }
            Ok(Box::new(Shell {
                app,
                window_frame,
                #[cfg(target_os = "macos")]
                movable: None,
            }))
        }),
    );
    if let Err(e) = result {
        fatal(&e.to_string(), true);
        std::process::exit(1);
    }
}

/// The file startup errors and crashes are appended to, beside the saved settings.
fn error_log() -> Option<std::path::PathBuf> {
    eframe::storage_dir(APP_NAME).map(|d| d.join("errors.log"))
}

/// Append a line with the time (Unix seconds) and the app version; true when it was written.
fn append_line(file: &std::path::Path, msg: &str) -> bool {
    use std::io::Write as _;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let open = || std::fs::OpenOptions::new().create(true).append(true).open(file);
    file.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && open().and_then(|mut f| writeln!(f, "[{secs}] {}: {msg}", version())).is_ok()
}

/// An error that ends the app: printed, logged, and (with `dialog`) shown in a message box, since
/// a Windows release build has no console and would otherwise close without a word.
fn fatal(msg: &str, dialog: bool) {
    eprintln!("texturecraft: {msg}");
    let log = error_log().filter(|f| append_line(f, msg));
    if dialog {
        use texturecraft_ui_egui::i18n::{tr, trf};
        let saved = log.map(|f| format!("\n\n{}", trf("Details saved to {file}", &[("file", &f.display())]))).unwrap_or_default();
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title(APP_NAME)
            .set_description(format!("{}\n\n{msg}{saved}", tr("TextureCraft had to close:")))
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gpu_and_power_preference_args() {
        let args = parse_args_from(["--gpu".to_string(), "nvidia".to_string()]).unwrap().unwrap();
        assert_eq!(args.gpu.as_deref(), Some("nvidia"));
        assert_eq!(args.power_pref, None);

        let args = parse_args_from(["--power-pref".to_string(), "high".to_string()]).unwrap().unwrap();
        assert_eq!(args.power_pref, Some(eframe::wgpu::PowerPreference::HighPerformance));

        let args = parse_args_from(["--power-pref".to_string(), "low".to_string(), "--gpu".to_string(), "3080".to_string()]).unwrap().unwrap();
        assert_eq!(args.power_pref, Some(eframe::wgpu::PowerPreference::LowPower));
        assert_eq!(args.gpu.as_deref(), Some("3080"));
    }

    #[test]
    fn only_a_run_without_a_control_port_checks_for_updates_at_start() {
        assert!(checks_for_updates_at_start(None));
        assert!(!checks_for_updates_at_start(Some(9000)));
    }

    #[test]
    fn bundled_app_icon_decodes_to_rgba() {
        let icon = icon().unwrap();
        assert!(icon.width > 0 && icon.height > 0);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }

    #[test]
    fn rejects_missing_or_invalid_gpu_args() {
        assert!(parse_args_from(["--gpu".to_string()]).is_err());
        assert!(parse_args_from(["--power-pref".to_string()]).is_err());
        assert!(parse_args_from(["--power-pref".to_string(), "unrecognized".to_string()]).is_err());
    }

    #[test]
    fn errors_are_appended_to_the_log_in_a_new_folder() {
        let dir = std::env::temp_dir().join(format!("texturecraft-errlog-test-{}", std::process::id()));
        let file = dir.join("nested").join("errors.log");
        assert!(append_line(&file, "first"));
        assert!(append_line(&file, "second"));
        let text = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with(&format!("{}: first", version())) && lines[1].ends_with(": second"), "{text}");
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!append_line(std::path::Path::new(""), "no folder"), "a path without a folder is refused");
    }

    #[test]
    fn startup_preferences_use_ui_json_then_legacy_storage_then_defaults() {
        let ron_path = std::env::temp_dir().join(format!("texturecraft-titlebar-test-{}.ron", std::process::id()));
        let json_path = ron_path.with_extension("json");
        for on in [false, true] {
            let mut ui = UiState::default();
            ui.prefs.system_title_bar = on;
            ui.prefs.project_open_limit_gib = 16;

            // Written as eframe writes it: a pretty RON map of RON strings
            let kv = std::collections::HashMap::from([("ui".to_string(), ron::to_string(&ui).unwrap())]);
            std::fs::write(&ron_path, ron::Options::default().to_string_pretty(&kv, Default::default()).unwrap()).unwrap();

            let legacy = saved_prefs(None, Some(&ron_path));
            assert_eq!(legacy.system_title_bar, on);
            assert_eq!(legacy.project_open_limit_gib, 16);

            ui.prefs.system_title_bar = !on;
            ui.prefs.project_open_limit_gib = 32;
            std::fs::write(&json_path, serde_json::to_vec(&ui).unwrap()).unwrap();

            let current = saved_prefs(Some(&json_path), Some(&ron_path));
            assert_eq!(current.system_title_bar, !on, "ui.json comes first");
            assert_eq!(current.project_open_limit_gib, 32);
        }

        std::fs::write(&json_path, "not json").unwrap();
        std::fs::write(&ron_path, "not ron").unwrap();
        assert_eq!(saved_prefs(Some(&json_path), Some(&ron_path)), texturecraft_ui_egui::state::Prefs::default());
        std::fs::remove_file(&json_path).unwrap();
        std::fs::remove_file(&ron_path).unwrap();
        assert_eq!(saved_prefs(Some(&json_path), Some(&ron_path)), texturecraft_ui_egui::state::Prefs::default());
    }

    /// How far the test's drag moves the mouse in one frame, in points
    const DRAG_STEP: egui::Vec2 = egui::vec2(80.0, 0.0);

    /// Points within which a put-back lands on the press point (the system rounds it to whole pixels)
    const PUT_BACK_REACH: f32 = 1.0;

    /// One frame of the window: the system's events go through the window's raw input hook, as eframe sends them; returns what the app asked of the window
    fn window_frame(harness: &mut egui_kittest::Harness<'static, Shell>, events: Vec<egui::Event>) -> Vec<egui::ViewportCommand> {
        let ctx = harness.ctx.clone();
        let mut raw = harness.input_mut().clone();
        raw.events.extend(events);
        eframe::App::raw_input_hook(harness.state_mut(), &ctx, &mut raw);
        *harness.input_mut() = raw;
        harness.step();
        harness.output().viewport_output.get(&egui::ViewportId::ROOT).map(|viewport| viewport.commands.clone()).unwrap_or_default()
    }

    /// Holding the fill's preview square in the window holds the system's pointer: put back to the press point every frame, or locked where the system locks it
    #[test]
    fn the_window_holds_the_pointer_while_the_preview_square_is_held() {
        let session = Session::new_project("Hold", texturecraft_engine::geom::primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 128).unwrap();
        let builder = egui_kittest::Harness::builder().with_size(egui::vec2(DEFAULT_SIZE[0], DEFAULT_SIZE[1]));
        let mut harness = builder.build_eframe(|cc| Shell {
            app: TextureCraftApp::new(cc, session),
            window_frame: None,
            #[cfg(target_os = "macos")]
            movable: None,
        });
        harness.run_steps(4);
        let layer = harness.state_mut().app.run("layer.add.fill", serde_json::json!({})).unwrap()["layer"].clone();
        harness.state_mut().app.run("layer.fill.projection", serde_json::json!({"layer": layer, "projection": "triplanar"})).unwrap();
        harness.run_steps(2);

        let pad = egui_kittest::kittest::Queryable::get_by_label(&harness, "Move on model").rect().center();
        let press = egui::Event::PointerButton { pos: pad, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE };
        let pressed = window_frame(&mut harness, vec![egui::Event::PointerMoved(pad), press]);
        let locked = pressed.contains(&egui::ViewportCommand::CursorGrab(egui::CursorGrab::Locked));

        // A locked pointer stays still and the system reports only its raw motion
        let mut moves = vec![egui::Event::MouseMoved(DRAG_STEP)];
        if !locked {
            moves.push(egui::Event::PointerMoved(pad + DRAG_STEP));
        }
        let moved = window_frame(&mut harness, moves);
        let put_back = moved.iter().any(|command| matches!(command, egui::ViewportCommand::CursorPosition(to) if to.distance(pad) <= PUT_BACK_REACH));

        assert!(locked || put_back, "the window puts the pointer back where the press began, so it never leaves: {moved:?}");
        let seen = harness.ctx.input(|input| input.pointer.latest_pos());
        assert!(seen.is_some_and(|seen| seen.distance(pad + DRAG_STEP) <= PUT_BACK_REACH), "the app reads the whole move: {seen:?}");
    }
}
