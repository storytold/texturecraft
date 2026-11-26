//! TextureCraft desktop app.
//!
//! Usage: `texturecraft [project.tcraft | mesh.glb] [--control <port>] [--empty] [--version]`
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(test)]
mod build_channel;
mod build_flavor;
mod control_server;
mod gpu;
mod launch_profile;
mod nightly_session;
mod process_identity;
mod profile_root;
mod window_frame;

use std::ffi::OsString;

use launch_profile::Profile;
use texturecraft_engine::Session;
use texturecraft_ui_egui::os_window::{self, LaunchIdentity};
use texturecraft_ui_egui::titlebar::TitleBar;
use texturecraft_ui_egui::ui_state_store;
use texturecraft_ui_egui::{TextureCraftApp, UiState};

/// The app name eframe keeps the settings under (`eframe::storage_dir`).
const APP_NAME: &str = "TextureCraft";

/// The data folder Dylan's #801 packaging lets a Stable wrapper set. A Nightly launch never reads
/// it: its profile arrives as the Launcher's literal selector/root pair, and nothing else.
const DATA_DIR_ENV: &str = "TEXTURECRAFT_DATA_DIR";

/// The window's size when it opens, in points.
const DEFAULT_SIZE: [f32; 2] = [1600.0, 960.0];
/// The smallest size the window can be resized to, in points.
const MIN_SIZE: [f32; 2] = [960.0, 600.0];

const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.texturecraft.png");
/// The Nightly window icon: the same artwork on the deeper background the package icon uses.
const NIGHTLY_ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/texturecraft-nightly.png");

/// The version line: a Nightly build names its exact feed identity and channel, and every other
/// build keeps the workspace version with the commit and date it was built from.
fn version() -> String {
    if build_flavor::COMPILED == build_flavor::BuildFlavor::NightlyV1 {
        return build_flavor::version(build_flavor::COMPILED);
    }
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

fn parse_args_from<I: IntoIterator<Item = String>>(it: I) -> Result<Option<Args>, String> {
    let mut args = Args { file: None, control: None, empty: false, gpu: None, power_pref: None };
    let mut it = it.into_iter();
    // Everything after `--` is a file, even when it looks like an option: the Launcher's nightly
    // protocol passes the selected file that way (and a file may be called `--version`).
    let mut after_delimiter = false;
    while let Some(a) = it.next() {
        if after_delimiter {
            // A Launcher hand-off may carry more than one file. Opening the first one beats a
            // silent exit on a Windows build, which has no console to show the refusal on.
            if args.file.is_none() {
                args.file = Some(a);
            }
            continue;
        }
        match a.as_str() {
            "--" => after_delimiter = true,
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
    let png = if build_flavor::COMPILED == build_flavor::BuildFlavor::NightlyV1 { NIGHTLY_ICON_PNG } else { ICON_PNG };
    let img = craft_image_codecs::decode(png).ok()?;
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

/// Directory for saved settings, UI state, and error logs: a Nightly launch's validated profile,
/// else `TEXTURECRAFT_DATA_DIR` (when non-empty, as a Stable package wrapper sets it), else
/// eframe's own folder for this app.
fn storage_dir() -> Option<std::path::PathBuf> {
    settings_folder(
        launch_profile::current().root(),
        std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()).map(std::path::PathBuf::from),
        eframe::storage_dir(APP_NAME),
    )
}

/// The settings folder, in the order a launch decides it: a Nightly profile, a Stable wrapper's
/// `TEXTURECRAFT_DATA_DIR`, else eframe's own folder for this app.
///
/// The last one is passed in rather than left to eframe. With `with_app_id` set (the Linux window
/// class the Stable entry needs), eframe derives its folder from the app id, so Stable's `app.ron`
/// — window state and the user's export presets — would move to `ai.storyteller.texturecraft` while
/// `ui.json` and `errors.log` stayed behind. `persistence_path` therefore names the file explicitly.
fn settings_folder(
    profile: Option<&std::path::Path>,
    wrapper_dir: Option<std::path::PathBuf>,
    legacy: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    profile.map(std::path::Path::to_path_buf).or(wrapper_dir).or(legacy)
}

/// Preferences needed before opening the initial project or window, with legacy storage fallback.
fn saved_prefs(ui_file: Option<&std::path::Path>, persistence_path: Option<&std::path::Path>) -> texturecraft_ui_egui::state::Prefs {
    if let Some(ui) = ui_file.and_then(ui_state_store::read) {
        return ui.prefs;
    }

    let file = persistence_path.map(std::path::Path::to_path_buf).or_else(|| storage_dir().map(|folder| folder.join("app.ron")));
    let saved = file.and_then(|file| std::fs::read_to_string(file).ok());
    let entries = saved.and_then(|text| ron::from_str::<std::collections::HashMap<String, String>>(&text).ok());
    entries.and_then(|entries| ron::from_str::<UiState>(entries.get("ui")?).ok()).map(|ui| ui.prefs).unwrap_or_default()
}

/// The app with the window chrome that needs the native window: the Windows borderless guard, and the native window
/// the framework runtime answers the hit test of.
struct Shell {
    app: TextureCraftApp,
    window_frame: Option<window_frame::WindowFrameFn>,
    /// Runs once when the window closes. On macOS that is the only place a clean quit is handled:
    /// Cmd+Q ends the process inside AppKit's terminate, after eframe's `on_exit` and before
    /// `eframe::run_native` returns, so the Nightly session record is finished here, not there.
    exit_hook: Option<Box<dyn FnOnce()>>,
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
        texturecraft_ui_egui::framework::before_ui(ui.ctx(), &*frame);
        self.app.ui(ui, frame);
    }

    fn on_exit(&mut self) {
        if let Some(hook) = self.exit_hook.take() {
            hook();
        }
    }
}

/// A script driving the app gets no network check and no update prompt over its window.
fn checks_for_updates_at_start(control: Option<u16>) -> bool {
    control.is_none()
}

/// The AppUserModelID a Nightly window is grouped under, set before any native window exists.
fn initialize_nightly_process_identity() -> Result<(), process_identity::IdentityError> {
    process_identity::initialize(build_flavor::COMPILED == build_flavor::BuildFlavor::NightlyV1, os_window::id_for(LaunchIdentity::NightlyV1))
}

/// Whether a refusal may also open a native message box. Standard error always carries the reason;
/// this only adds a box where one renders without the app's event loop. Never on macOS: rfd's alert
/// there needs the AppKit run loop this start has not reached, so a refused nightly launch hung in
/// an invisible modal instead of exiting (GridCraft Mac smoke, 2026-11-26).
fn refusal_dialog(flavor: build_flavor::BuildFlavor, desktop_session: bool, macos: bool) -> bool {
    build_flavor::needs_native_error(flavor, desktop_session) && !macos
}

/// A refused Nightly start explains itself in a dialog where the desktop shows one without the
/// app's run loop (Windows, X11/Wayland); macOS gets the standard-error line only.
fn report_launch_profile_error(error: &str) {
    eprintln!("texturecraft: {error}");
    let desktop_session = cfg!(windows)
        || cfg!(target_os = "macos")
        || (cfg!(target_os = "linux") && ["DISPLAY", "WAYLAND_DISPLAY"].into_iter().any(|key| std::env::var_os(key).is_some_and(|v| !v.is_empty())));
    if refusal_dialog(build_flavor::COMPILED, desktop_session, cfg!(target_os = "macos")) {
        if let Err(error) = initialize_nightly_process_identity() {
            eprintln!("texturecraft: {error}");
            return;
        }
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        let _ = rfd::MessageDialog::new()
            .set_title(os_window::name_for(LaunchIdentity::NightlyV1))
            .set_description(error)
            .set_level(rfd::MessageLevel::Error)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
    }
}

/// Select the immutable launch profile before any startup service or side effect, and answer the
/// Launcher's version queries before root validation, logging, IPC or a window. `None` means the
/// launch is already answered and the process returns.
fn prepare_launch() -> Option<Vec<OsString>> {
    let argv: Vec<OsString> = std::env::args_os().skip(1).collect();
    // Plain `--version`, or the Launcher's `--launch-profile nightly-v1 --profile-root <dir>
    // --version`: both are answered here, with nothing but standard output written.
    if build_flavor::COMPILED == build_flavor::BuildFlavor::NightlyV1 && (build_flavor::version_query(&argv) || launch_profile::version_query(&argv)) {
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), "{}", build_flavor::version(build_flavor::COMPILED));
        return None;
    }
    let early = launch_profile::parse(argv)
        .and_then(|launch| {
            build_flavor::require_profile(build_flavor::COMPILED, &launch.profile)?;
            initialize_nightly_process_identity().map_err(|error| error.to_string())?;
            Ok(launch)
        })
        .and_then(|launch| {
            launch_profile::prepare(launch, |root| launch_profile::validate_root(root, |key| std::env::var_os(key), std::env::current_exe().ok().as_deref()))
        })
        .unwrap_or_else(|error| {
            report_launch_profile_error(&error);
            std::process::exit(2)
        });
    let identity = if early.profile.is_nightly() { LaunchIdentity::NightlyV1 } else { LaunchIdentity::Stable };
    if let Err(error) = launch_profile::install(early.profile).and_then(|()| os_window::select_identity(identity)) {
        report_launch_profile_error(&error);
        std::process::exit(2);
    }
    if let Profile::NightlyV1(root) = launch_profile::current() {
        // Every data folder follows the profile before the first one is used.
        texturecraft_ui_egui::set_data_root(root.clone());
        texturecraft_ui_egui::os_window::set_display_version(build_flavor::BUILT_VERSION.to_owned());
    }
    Some(early.args)
}

/// The launch arguments as text. A Nightly protocol argument is a path the app opens, so a value
/// that is not valid Unicode is refused rather than silently replaced.
fn launch_arguments(arguments: Vec<OsString>) -> Result<Vec<String>, String> {
    arguments.into_iter().map(|value| value.into_string().map_err(|_| "an argument is not valid Unicode".to_string())).collect()
}

fn main() {
    // Windows: the update helper is a copy of this program started with `--finish-update`. A
    // Nightly install is updated by ArtCraft Launcher, so it never installs an update itself.
    #[cfg(windows)]
    if build_flavor::COMPILED == build_flavor::BuildFlavor::Stable
        && let Some(result) = craft_update::self_update::finish(&texturecraft_ui_egui::UPDATE_APP, &std::env::args_os().skip(1).collect::<Vec<_>>())
    {
        std::process::exit(i32::from(result.is_err()));
    }

    // Windows: the eyedropper's pointer keeper (screen_pick) is a copy of this program started with
    // its own argument. It reads no user state and is started without the profile tokens, so it
    // runs before the launch protocol would refuse it.
    #[cfg(windows)]
    if std::env::args_os().nth(1).is_some_and(|arg| arg == texturecraft_ui_egui::screen_pick::KEEPER_ARG) {
        std::process::exit(texturecraft_ui_egui::screen_pick::keep_pointer());
    }

    let Some(arguments) = prepare_launch() else { return };
    let nightly = launch_profile::current().is_nightly();
    let args = match launch_arguments(arguments).and_then(parse_args_from) {
        Ok(Some(a)) => a,
        Ok(None) => return,
        Err(e) => {
            eprintln!("texturecraft: {e}");
            std::process::exit(2);
        }
    };
    // The control server is a stable-install service: a Nightly window is driven by the Launcher,
    // and its token file would sit in the shared temp folder rather than its private profile.
    if nightly && args.control.is_some() {
        eprintln!("texturecraft: TextureCraft Nightly cannot start a control server (--control); use the stable app or texturecraft-cli");
        std::process::exit(2);
    }

    // Errors before the window opens speak the system's language; the app then applies the saved one
    texturecraft_ui_egui::i18n::set_language(texturecraft_ui_egui::i18n::os_language());
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A message box needs the main thread on macOS; other threads' panics are only logged
        fatal(&format!("internal error: {info}"), std::thread::current().name() == Some("main"));
        previous_hook(info);
    }));

    // Scripted runs use temporary files so they cannot overwrite saved settings. Otherwise eframe
    // is told the file explicitly, so `with_app_id` cannot move Stable's `app.ron` out of the
    // folder `storage_dir` keeps `ui.json` and `errors.log` in (see `settings_folder`).
    let no_prefs_path =
        std::env::var_os("TEXTURECRAFT_NO_PREFS").map(|_| std::env::temp_dir().join(format!("texturecraft-noprefs-{}.ron", std::process::id())));
    let persistence_path = no_prefs_path.clone().or_else(|| storage_dir().map(|dir| dir.join("app.ron")));
    let ui_file = no_prefs_path.as_ref().map(|path| path.with_extension("json")).or_else(|| storage_dir().map(|folder| folder.join("ui.json")));
    if let Some(file) = &ui_file {
        let _ = ui_state_store::FILE.set(file.clone());
    }
    let prefs = saved_prefs(ui_file.as_deref(), persistence_path.as_deref());

    // The worker pool's size can only be set before its first job
    prefs.apply_work_threads();
    prefs.apply_resource_limits();
    let session = initial_session(&args);
    let frame_decision = TitleBar::decide(prefs.system_title_bar);
    let title_bar = TitleBar::of(&frame_decision.frame);
    let mut viewport = egui::ViewportBuilder::default()
        .with_app_id(os_window::app_id())
        .with_title(os_window::name())
        .with_inner_size(DEFAULT_SIZE)
        .with_min_inner_size(MIN_SIZE);
    if let Some(i) = icon() {
        viewport = viewport.with_icon(i);
    }
    let viewport = texturecraft_ui_egui::titlebar::viewport(&frame_decision, viewport);
    let mut options = eframe::NativeOptions { viewport, renderer: eframe::Renderer::Wgpu, persistence_path, ..Default::default() };
    if let Err(e) = gpu::configure(&mut options, args.gpu, args.power_pref) {
        fatal(&e.to_string(), true);
        std::process::exit(2);
    }
    let control = args.control;
    // The Nightly session record: one owner per profile, an unclean exit reported by the next
    // start, and no document name, path or diagnostic payload collected.
    let (nightly_session, session_health_warning) = match nightly_session::Session::begin_with_health(
        launch_profile::current().root(),
        build_flavor::BUILT_VERSION,
        Some(env!("TEXTURECRAFT_BUILD_SHA")),
    ) {
        Ok(state) => state,
        Err(error) => {
            log::warn!("Nightly session health is unavailable: {error}");
            (None, true)
        }
    };
    let previous_nightly_unclean = nightly_session.as_ref().is_some_and(nightly_session::Session::previous_unclean);
    // Set once a window has posted the interruption notice; until then the record must survive.
    let unclean_notice_shown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let notice_shown_in_window = unclean_notice_shown.clone();
    let nightly_session: nightly_session::Shared = std::sync::Arc::new(std::sync::Mutex::new(nightly_session));
    // The exit hook is the controlled return on macOS, where every quit ends the process inside
    // AppKit's terminate and `run_native` never returns. Elsewhere the return path below finishes.
    let finish_at_exit = {
        let shared = nightly_session.clone();
        let shown = unclean_notice_shown.clone();
        move || nightly_session::finish_once(&shared, shown.load(std::sync::atomic::Ordering::Relaxed))
    };
    let window_name = os_window::name();
    let result = eframe::run_native(
        window_name,
        options,
        Box::new(move |cc| {
            #[cfg(windows)]
            if let Some(w) = cc.winit_window() {
                set_taskbar_icon(w);
            }

            // winit's Quit ends the process without asking; this menu's asks like the close button
            #[cfg(target_os = "macos")]
            if let Err(e) = craft_menu::mac::install_app_menu(&cc.egui_ctx, window_name) {
                log::warn!("the app menu could not be installed: {e}");
            }
            texturecraft_ui_egui::framework::install(&cc.egui_ctx, frame_decision)?;
            let frame_guard = window_frame::service(cc, matches!(title_bar, TitleBar::App(_)));
            let mut app = TextureCraftApp::new(cc, session);
            app.title_bar = title_bar;
            app.set_startup_update_check(checks_for_updates_at_start(control));
            if let Some(port) = control {
                app.control = Some(control_server::start(port, cc.egui_ctx.clone()));
            }
            if previous_nightly_unclean || session_health_warning {
                use texturecraft_ui_egui::i18n::tr;
                app.banner = Some(if previous_nightly_unclean {
                    tr("The previous TextureCraft Nightly session did not finish normally. No diagnostics have been sent.")
                } else {
                    tr("TextureCraft Nightly could not record session health in its profile; abnormal-exit detection is unavailable.")
                });
                notice_shown_in_window.store(previous_nightly_unclean, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(Box::new(Shell { app, window_frame: frame_guard, exit_hook: Some(Box::new(finish_at_exit)) }))
        }),
    );
    // A controlled return, including a handled startup error. On macOS a window already finished in
    // its exit hook, so this is then a no-op.
    nightly_session::finish_once(&nightly_session, unclean_notice_shown.load(std::sync::atomic::Ordering::Relaxed));
    if let Err(e) = result {
        fatal(&e.to_string(), true);
        std::process::exit(1);
    }
}

/// The file startup errors and crashes are appended to, beside the saved settings.
fn error_log() -> Option<std::path::PathBuf> {
    storage_dir().map(|d| d.join("errors.log"))
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
            .set_title(os_window::name())
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
    fn stable_settings_stay_in_the_folder_eframe_used_before_the_app_id() {
        let legacy = std::path::PathBuf::from("/legacy/TextureCraft");
        // Stable, no wrapper folder: the folder eframe picked for `APP_NAME` before `with_app_id`
        // existed, so `app.ron` (window state and the export presets stored in it) does not move.
        let folder = settings_folder(None, None, Some(legacy.clone())).expect("a legacy folder");
        assert_eq!(folder.join("app.ron"), legacy.join("app.ron"));
        // A Stable wrapper's data folder still wins, which is what #801's packaging expects.
        let wrapper = std::path::PathBuf::from("/wrapper/data");
        assert_eq!(settings_folder(None, Some(wrapper.clone()), Some(legacy.clone())), Some(wrapper.clone()));
        // A Nightly profile wins over both, so no Stable folder is ever read.
        assert_eq!(settings_folder(Some(std::path::Path::new("/profile")), Some(wrapper), Some(legacy)), Some(std::path::PathBuf::from("/profile")));
        // Nothing resolvable: there is no folder to fall back to and eframe keeps its own default.
        assert_eq!(settings_folder(None, None, None), None);
        // The app's wiring: with no profile and no wrapper folder, `app.ron` sits beside `ui.json`
        // in eframe's folder for `APP_NAME`, not in the folder of the app id `with_app_id` sets.
        if std::env::var_os(DATA_DIR_ENV).is_none()
            && !launch_profile::current().is_nightly()
            && let Some(stable) = eframe::storage_dir(APP_NAME)
        {
            assert_eq!(storage_dir(), Some(stable.clone()));
            assert_eq!(storage_dir().map(|dir| dir.join("app.ron")), Some(stable.join("app.ron")));
        }
    }

    #[test]
    fn a_file_after_the_delimiter_is_a_file_even_when_it_looks_like_an_option() {
        let args = parse_args_from(["--".to_string(), "--version".to_string()]).unwrap().unwrap();
        assert_eq!(args.file.as_deref(), Some("--version"));
        let args = parse_args_from(["--".to_string(), "--launch-profile".to_string()]).unwrap().unwrap();
        assert_eq!(args.file.as_deref(), Some("--launch-profile"));
        // Options before the delimiter still parse as options.
        let args = parse_args_from(["--gpu".to_string(), "nvidia".to_string(), "--".to_string(), "a.glb".to_string()]).unwrap().unwrap();
        assert_eq!((args.gpu.as_deref(), args.file.as_deref()), (Some("nvidia"), Some("a.glb")));
        // One window opens one project: extra files after the delimiter are ignored, in that order.
        let args = parse_args_from(["--".to_string(), "a.glb".to_string(), "b.glb".to_string()]).unwrap().unwrap();
        assert_eq!(args.file.as_deref(), Some("a.glb"));
    }

    #[test]
    fn launch_arguments_refuse_a_name_that_is_not_unicode() {
        assert_eq!(launch_arguments(vec![OsString::from("--version")]).unwrap(), vec!["--version".to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt as _;
            assert!(launch_arguments(vec![OsString::from_vec(vec![b'a', 0xff, b'b'])]).is_err());
        }
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

    /// One frame of the window: the system's events go through the window's raw input hook (the harness runs it before
    /// every pass, as eframe does); returns what the app asked of the window
    fn window_frame(harness: &mut egui_kittest::Harness<'static, Shell>, events: Vec<egui::Event>) -> Vec<egui::ViewportCommand> {
        harness.input_mut().events.extend(events);
        harness.step();
        harness.output().viewport_output.get(&egui::ViewportId::ROOT).map(|viewport| viewport.commands.clone()).unwrap_or_default()
    }

    /// Holding the fill's preview square in the window holds the system's pointer: put back to the press point every frame, or locked where the system locks it
    #[test]
    fn the_window_holds_the_pointer_while_the_preview_square_is_held() {
        let session = Session::new_project("Hold", texturecraft_engine::geom::primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 128).unwrap();
        let builder = egui_kittest::Harness::builder()
            .with_size(egui::vec2(DEFAULT_SIZE[0], DEFAULT_SIZE[1]))
            .with_context_setup(texturecraft_ui_egui::framework::install_headless);
        let mut harness = builder.build_eframe(|cc| Shell { app: TextureCraftApp::new(cc, session), window_frame: None, exit_hook: None });
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

#[cfg(test)]
mod refusal_dialog_tests {
    use super::refusal_dialog;
    use crate::build_flavor::BuildFlavor;

    #[test]
    fn a_refusal_never_blocks_on_a_modal_before_the_event_loop_on_macos() {
        assert!(refusal_dialog(BuildFlavor::NightlyV1, true, false), "a desktop refusal still shows the box where one renders");
        assert!(!refusal_dialog(BuildFlavor::NightlyV1, true, true), "macOS has no run loop yet: the modal would hang");
        assert!(!refusal_dialog(BuildFlavor::NightlyV1, false, false), "a headless session has no box");
        assert!(!refusal_dialog(BuildFlavor::Stable, true, false), "only a nightly refusal explains itself in a box");
    }
}
