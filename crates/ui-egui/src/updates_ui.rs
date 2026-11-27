//! Software updates on the desktop: the check at start, Help > Check for Updates and the update
//! prompt, as craft-update's dialogs on the app's [`DialogHost`](craft_ui::dialog::DialogHost). An
//! update that is ready starts its install helper when the app is dropped at exit.

use craft_ui::dialog::DialogResult;
use craft_update::stack_dialogs::{CheckChoice, UpdateCheckDialog};
use craft_update::updater::{Action, Status, Updater};

use crate::TextureCraftApp;

pub(crate) struct Runtime {
    updater: Updater,
    /// The desktop app turns the check at start on; tests leave it off.
    startup_check: bool,
    check_requested: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self { updater: Updater::new(&crate::UPDATE_APP, env!("CARGO_PKG_VERSION")), startup_check: false, check_requested: false }
    }
}

impl TextureCraftApp {
    /// Whether the app asks for a newer release when it starts (off until the desktop app turns it on).
    pub fn set_startup_update_check(&mut self, on: bool) {
        // A Nightly install is updated by ArtCraft Launcher with the rest of the suite, never by
        // the app: an in-app update would replace a copy the Launcher believes it manages.
        self.updates_rt.startup_check = on && crate::os_window::stable_services_allowed();
    }
}

/// Help > Check for Updates: the next frame starts a check that reports its outcome. In a Nightly
/// build the Launcher owns updates, so it says so instead of checking.
pub(crate) fn request_check(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if !crate::os_window::stable_services_allowed() {
        app.toast(crate::i18n::tr("TextureCraft Nightly is updated by ArtCraft Launcher."), false);
        return;
    }
    app.updates_rt.check_requested = true;
    ctx.request_repaint();
}

/// The update prompt and the modal result window hold the keyboard while they are open, so keys
/// do not act on the project behind them.
pub(crate) fn blocks_input(app: &TextureCraftApp) -> bool {
    holds_keyboard(app.updates_rt.updater.status(), app.updates_rt.updater.prompt_open())
}

/// The prompt is modal, and the result window is modal for every status but [`Status::Updated`],
/// a notice that leaves the app usable behind it.
fn holds_keyboard(status: &Status, prompt_open: bool) -> bool {
    prompt_open || !matches!(status, Status::Idle | Status::Updated(_))
}

pub(crate) fn tick(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if !crate::os_window::stable_services_allowed() {
        return;
    }
    let rt = &mut app.updates_rt;
    if std::mem::take(&mut rt.check_requested) {
        rt.updater.check_now();
    } else if rt.startup_check {
        rt.updater.check_at_start();
    }
    rt.updater.tick(ctx);
}

/// Whether the result window shows: for every status but idle (a requested check, and the first
/// start after an update, which reports that it installed or why it failed), and never over the
/// update prompt.
fn result_window_shows(status: &Status, prompt_open: bool) -> bool {
    !prompt_open && !matches!(status, Status::Idle)
}

/// Whether an update can restart the app with all edits saved and pending work finished.
fn safe_to_close(app: &TextureCraftApp) -> bool {
    !app.session.modified && !app.session.stroke_active() && app.save_job.is_none() && app.bake.is_none() && app.resize.is_none() && app.high_poly_job.is_none()
}

/// One pass of the update flow, before the dialog host runs: answers the result dialog closed in
/// the last pass, drives the update prompt, and keeps the result dialog open and current while the
/// updater has something to report.
pub(crate) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let safe = safe_to_close(app);
    // Answered before the dialog is kept open below, so a dismissed outcome does not open again at once
    match app.framework_dialogs.take::<UpdateCheckDialog>() {
        Some(DialogResult::Ok(CheckChoice::CheckNow | CheckChoice::CheckAgain)) => app.updates_rt.check_requested = true,
        // The next `run` asks the window to close once it is safe to
        Some(DialogResult::Ok(CheckChoice::RestartToUpdate)) => app.updates_rt.updater.restart_to_update(),
        Some(DialogResult::Ok(CheckChoice::Update)) => {
            app.updates_rt.updater.request_install(safe);
        }
        Some(DialogResult::Ok(CheckChoice::Download)) => ctx.open_url(egui::OpenUrl::new_tab(crate::UPDATE_APP.releases_url())),
        // Closing the dialog dismisses the outcome it showed
        Some(_) => app.updates_rt.updater.dismiss_status(),
        None => {}
    }
    match app.updates_rt.updater.run(&mut app.framework_dialogs, safe) {
        Action::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
        Action::Close => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        _ => {}
    }
    let updater = &app.updates_rt.updater;
    let shows = result_window_shows(updater.status(), updater.prompt_open());
    let (status, installing, message) = (updater.status().clone(), updater.is_installing(), updater.status_message());
    let dialogs = &mut app.framework_dialogs;
    if shows && !dialogs.is_open::<UpdateCheckDialog>() {
        dialogs.open(UpdateCheckDialog::new(env!("CARGO_PKG_VERSION")));
    } else if !shows && dialogs.is_open::<UpdateCheckDialog>() {
        dialogs.cancel::<UpdateCheckDialog>(ctx);
        // Hidden for the prompt, not dismissed: the outcome shows again after it
        let _ = dialogs.take::<UpdateCheckDialog>();
    }
    if let Some(dialog) = dialogs.get_mut::<UpdateCheckDialog>() {
        dialog.set_status(status);
        dialog.set_installing(installing);
        dialog.set_message(message);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc::{Receiver, channel};

    use craft_update::updater::CheckSource;
    use craft_update::{Arch, Asset, Os, PackageKind, Paths, Platform, Release, pick_asset};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;
    use serde_json::json;
    use texturecraft_engine::Tool;
    use texturecraft_engine::bake::Progress;
    use texturecraft_engine::geom::glam::Vec2;

    use super::*;
    use crate::state::Mode;

    /// Stands in for GitHub: each check sends one message and finds nothing newer.
    fn source() -> (CheckSource, Receiver<()>) {
        let (sent, asked) = channel();
        let ask: CheckSource = Arc::new(move || {
            let _ = sent.send(());
            Ok(None)
        });
        (ask, asked)
    }

    /// The harness draws the UI, and the i18n tests switch the process-wide language to ones
    /// its fonts cannot draw.
    fn language() -> std::sync::MutexGuard<'static, ()> {
        crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The app with an updater that asks `ask` and keeps its records out of the user's folders.
    fn harness(modified: bool, test: &str, ask: CheckSource) -> Harness<'static, TextureCraftApp> {
        let root = std::env::temp_dir().join(format!("texturecraft-updates-{}-{test}", std::process::id()));
        let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
        session.modified = modified;
        let mut h = Harness::builder()
            .with_size(egui::vec2(1600.0, 960.0))
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|cc| TextureCraftApp::new(cc, session));
        h.state_mut().updates_rt.updater =
            Updater::new(&crate::UPDATE_APP, "0.0.1").with_source(ask).with_paths(Paths::under(&root)).with_pins_path(root.join("pins.json"));
        h.run_steps(2);
        h
    }

    fn offer(h: &mut Harness<'static, TextureCraftApp>) {
        let release = Release {
            tag: "v99.0.0".into(),
            version: craft_update::parse_version("99.0.0"),
            name: "TextureCraft 99.0.0".into(),
            html_url: "https://github.com/storytold/texturecraft/releases/tag/v99.0.0".into(),
            ..Default::default()
        };
        h.state_mut().updates_rt.updater.offer(release);
        // The offer reads on a thread whether this copy can install the release itself; a later frame's tick opens
        // the prompt.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !h.state().updates_rt.updater.prompt_open() && std::time::Instant::now() < deadline {
            h.run_steps(1);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        h.run_steps(2);
    }

    #[test]
    fn the_menu_item_runs_a_check_that_reports_its_outcome() {
        let _language = language();
        let (ask, asked) = source();
        let mut h = harness(false, "menu", ask);
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, "ui.help.checkForUpdates");
        h.step();
        let status = h.state().updates_rt.updater.status().clone();
        assert!(matches!(status, Status::Checking | Status::UpToDate), "{status:?}");
        // Dropping the app drops the only sender left when no check started, so this cannot hang.
        drop(h);
        assert!(asked.recv().is_ok(), "the check asked for the latest release");
    }

    #[test]
    fn the_check_at_start_runs_only_when_the_desktop_app_turns_it_on() {
        let _language = language();
        for on in [false, true] {
            let (ask, asked) = source();
            let mut h = harness(false, if on { "start-on" } else { "start-off" }, ask);
            h.state_mut().set_startup_update_check(on);
            h.step();
            drop(h);
            assert_eq!(asked.recv().is_ok(), on, "startup check {on}");
        }
    }

    #[test]
    fn an_offered_update_shows_the_prompt_and_later_closes_it() {
        let _language = language();
        let (ask, _asked) = source();
        let mut h = harness(false, "prompt", ask);
        offer(&mut h);
        assert!(h.state().updates_rt.updater.prompt_open());
        h.get_by_label("Later").click();
        h.run_steps(2);
        assert!(!h.state().updates_rt.updater.prompt_open());
    }

    #[test]
    fn keys_do_not_reach_the_project_behind_the_prompt() {
        let _language = language();
        let (ask, _asked) = source();
        let mut h = harness(false, "keys", ask);
        assert_eq!(h.state().ui.mode, Mode::Painting);
        offer(&mut h);
        assert!(crate::docking::input_blocked(h.state(), &h.ctx), "floating panels stop taking input");
        h.key_press(egui::Key::F8);
        h.step();
        assert_eq!(h.state().ui.mode, Mode::Painting, "the prompt holds the keyboard");
        h.get_by_label("Later").click();
        h.run_steps(2);
        assert!(!crate::docking::input_blocked(h.state(), &h.ctx));
        h.key_press(egui::Key::F8);
        h.step();
        assert_eq!(h.state().ui.mode, Mode::Baking, "the key works once the prompt is closed");
    }

    #[test]
    fn the_path_tools_keys_do_not_reach_the_path_behind_the_prompt() {
        let _language = language();
        let (ask, _asked) = source();
        let mut h = harness(false, "path-keys", ask);
        let state = h.state_mut();
        state.session.tool.tool = Tool::Path;
        texturecraft_engine::execute(&mut state.session, "tool.path", &json!({"uvPoints": [[0.2, 0.5], [0.8, 0.5]]})).unwrap();
        h.run_steps(2);
        let points = |h: &Harness<'static, TextureCraftApp>| h.state().session.tool.path.points.len();
        assert_eq!(points(&h), 2);
        offer(&mut h);
        h.key_press(egui::Key::Backspace);
        h.step();
        assert_eq!(points(&h), 2, "the prompt holds the keyboard");
        h.get_by_label("Later").click();
        h.run_steps(2);
        h.key_press(egui::Key::Backspace);
        h.step();
        assert_eq!(points(&h), 1, "the key works once the prompt is closed");
    }

    #[test]
    fn the_result_window_holds_the_keyboard_unless_it_is_the_updated_notice() {
        let modal = [
            Status::Checking,
            Status::UpToDate,
            Status::Failed("offline".into()),
            Status::Installing,
            Status::Ready,
            Status::Managed,
            Status::InstallFailed("locked".into()),
        ];
        for status in &modal {
            assert!(holds_keyboard(status, false), "{status:?}");
        }
        assert!(!holds_keyboard(&Status::Idle, false));
        assert!(!holds_keyboard(&Status::Updated("99.0.0".into()), false));
        assert!(holds_keyboard(&Status::Idle, true), "the prompt is modal");
    }

    #[test]
    fn unsaved_changes_and_running_work_hold_an_update_back() {
        let _language = language();
        let (ask, _asked) = source();
        let mut h = harness(false, "safe-to-close", ask);
        assert!(safe_to_close(h.state()));

        h.state_mut().session.modified = true;
        assert!(!safe_to_close(h.state()), "unsaved changes");
        h.state_mut().session.modified = false;

        h.state_mut().bake = Some(crate::BakeJob { queue: Vec::new(), current: None, total: 0, mesh_id: 0, only: None });
        assert!(!safe_to_close(h.state()), "a bake");
        h.state_mut().bake = None;

        h.state_mut().load_high_polys(["missing-high.obj".to_owned()]);
        assert!(!safe_to_close(h.state()), "an accepted high-poly import is still pending");
        h.state_mut().high_poly_job = None;

        h.state_mut().resize = Some(crate::ResizeJob { progress: Arc::new(Progress::new()), rx: channel().1 });
        assert!(!safe_to_close(h.state()), "a resize");
        h.state_mut().resize = None;

        let session = &mut h.state_mut().session;
        texturecraft_engine::execute(session, "layer.add.paint", &serde_json::Value::Null).unwrap();
        session.modified = false;
        session.begin_stroke_uv(Vec2::new(0.5, 0.5), 1.0).unwrap();

        assert!(!safe_to_close(h.state()), "a stroke");
        h.state_mut().session.cancel_stroke();
        assert!(safe_to_close(h.state()));

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("project.tcraft");
        assert!(h.state_mut().start_save(path.to_str().unwrap(), true, false, false));
        assert!(!safe_to_close(h.state()), "the accepted archive write must finish before restarting");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while h.state().save_job.is_some() && std::time::Instant::now() < deadline {
            h.step();
        }

        assert!(safe_to_close(h.state()));
        assert!(path.exists());
    }

    #[test]
    fn result_window_shows_every_status_but_idle() {
        let shown = [
            Status::Checking,
            Status::UpToDate,
            Status::Failed("offline".into()),
            Status::Installing,
            Status::Ready,
            Status::Managed,
            Status::Updated("99.0.0".into()),
            Status::InstallFailed("locked".into()),
        ];
        for status in &shown {
            assert!(result_window_shows(status, false), "{status:?} shows");
            assert!(!result_window_shows(status, true), "{status:?} waits for the prompt");
        }
        assert!(!result_window_shows(&Status::Idle, false));
    }

    #[test]
    fn the_update_app_finds_the_assets_a_release_publishes() {
        let names = [
            "texturecraft-0.4.0-macos-universal.dmg",
            "texturecraft-0.4.0-windows-x64.msi",
            "texturecraft-0.4.0-windows-x64-portable.zip",
            "texturecraft-0.4.0-linux-x86_64.AppImage",
            "texturecraft-0.4.0-linux-x86_64.tar.gz",
            "texturecraft-cli-0.4.0-macos-universal.zip",
            "texturecraft-web-0.4.0.zip",
        ];
        let assets = names.iter().map(|name| Asset { name: (*name).into(), size: 1, url: String::new(), sha256: None }).collect();
        let release = Release { assets, ..Default::default() };
        let stems = [crate::UPDATE_APP.id];
        let picked = |os, arch, kind| pick_asset(&release, &stems, Platform { os, arch }, &[kind]).map(|(asset, _)| asset.name.clone());
        assert_eq!(picked(Os::MacOs, Arch::Arm64, PackageKind::Dmg).as_deref(), Some(names[0]));
        assert_eq!(picked(Os::Windows, Arch::X64, PackageKind::Msi).as_deref(), Some(names[1]));
        assert_eq!(picked(Os::Windows, Arch::X64, PackageKind::PortableZip).as_deref(), Some(names[2]));
        assert_eq!(picked(Os::Linux, Arch::X64, PackageKind::AppImage).as_deref(), Some(names[3]));
        assert_eq!(picked(Os::Linux, Arch::X64, PackageKind::TarGz).as_deref(), Some(names[4]));
        assert_eq!(crate::UPDATE_APP.bundle_id(), "ai.storyteller.texturecraft");
        assert_eq!(crate::UPDATE_APP.repo, "storytold/texturecraft");
    }
}
