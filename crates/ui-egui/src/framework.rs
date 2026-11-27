//! The craft-ui runtime, hosted from TextureCraft's own eframe `App` (craft-libs docs/craft-framework.md, section 2.2).
//! The desktop app calls [`install`] once, [`TextureCraftApp::before_input`] in `raw_input_hook`,
//! [`TextureCraftApp::close_path`] last in `logic`, and [`before_ui`] first in `ui`; the browser build and tests
//! install with [`install_headless`]. The framework dialogs run on [`TextureCraftApp::framework_dialogs`] after
//! TextureCraft's own UI.
//!
//! The runtime takes Escape, Tab and the clipboard events for the whole window unless one of TextureCraft's own egui
//! widgets has the keyboard. The window's Escape is read with [`escape_pressed`] and taken with
//! [`take_window_cancel`], its plain Enter taken with [`take_window_submit`]; the 3D and 2D views are declared to the runtime with [`host_canvas`], which keeps the edit
//! chords and the clipboard events in egui's input for TextureCraft's shortcuts. A bare Tab with nothing or a view
//! focused toggles the window's chrome in the runtime ([`RootEnv::tab_toggles_chrome`]), which `frame_input` turns into
//! Hide UI. A focused view leaves the keyboard to the shortcuts ([`widget_has_keyboard`]).

use std::sync::{Arc, OnceLock};

use craft_host::target::ShellInfo;
use craft_host::{AppInfo, Host};
use craft_keymap::Overrides;
use craft_ui::fonts::{I18nScripts, SystemFonts, UiFonts};
use craft_ui::runtime::commands::{ChromeCommand, CommandInput, CommandSet as _, EditCommands, NavCommand, QuitCommand};
use craft_ui::runtime::env::{RootEnv, Scale};
use craft_ui::runtime::focus::{EditSet, KeyUse};
use craft_ui::runtime::frame::{ChromeToggle, FrameInput, FrameOutput};
use craft_ui::runtime::keys::{Intent, with_window_intents};
use craft_ui::shell::embed::{self, CloseDecision};
use craft_window::policy::Decision;
use craft_window::system::WindowSystem;
use egui::ViewportId;

use crate::TextureCraftApp;

/// Each pass of the root window, first in `App::ui`: hands the runtime the native window it answers the hit test of
/// (Windows) and fits the window to its monitor once.
#[cfg(not(target_arch = "wasm32"))]
pub use craft_ui::shell::embed::before_ui;

/// The scripts of the interface languages, for the font stack.
static SCRIPTS: I18nScripts = I18nScripts;

/// The runtime's first font stack: egui's bundled faces and no installed system fonts. TextureCraft's own definitions
/// ([`crate::theme::install`]) replace it before anything is drawn.
fn ui_fonts() -> UiFonts<'static> {
    UiFonts::new(&SCRIPTS).system_fonts(SystemFonts::Off)
}

fn install_with(ctx: &egui::Context, shell: ShellInfo, windows: Arc<dyn WindowSystem>, frame: Option<Decision>) -> Result<(), String> {
    let mut spec = craft_ui::InstallSpec::new(shell, windows).app_id(crate::os_window::app_id());
    if let Some(frame) = frame {
        spec = spec.frame(frame);
    }

    embed::install(ctx, spec, &ui_fonts()).map(drop).map_err(|error| format!("the UI runtime could not be installed: {error}"))
}

/// Installs the runtime on the desktop window's context, once, before the first pass, with the window's `frame`
/// ([`crate::titlebar::TitleBar::decide`]) and the `backend` the window opened on.
#[cfg(not(target_arch = "wasm32"))]
pub fn install(ctx: &egui::Context, frame: Decision, backend: craft_host::target::Backend) -> Result<(), String> {
    let shell = ShellInfo::default().backend(backend);
    install_with(ctx, shell, Arc::new(craft_window::system::SystemWindows), Some(frame))
}

/// Installs the runtime on a context with no native window of its own: the browser build's canvas and test windows.
pub fn install_headless(ctx: &egui::Context) {
    if let Err(error) = install_with(ctx, ShellInfo::default(), Arc::new(craft_window::system::NoWindows), None) {
        log::warn!("{error}");
    }
}

/// Who TextureCraft is to the framework's dialogs (the update dialogs show its name and icon).
pub(crate) fn host() -> Arc<Host> {
    Arc::new(
        Host::builder(AppInfo { id: "texturecraft", name: "TextureCraft", version: texturecraft_engine::VERSION })
            .icon_png(include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.texturecraft.png"))
            .build(),
    )
}

/// TextureCraft's own shortcut table keeps the keys the framework binds by default: Ctrl+Q is `ui.quit`, F6 is
/// `camera.orthographic` (and Shift+F6 stays free for it), so the framework's quit and pane moves are unbound, as is
/// Shift+Tab's panels toggle except tools (TextureCraft has one Hide UI, on Tab).
/// Overrides are keyed by command id, which the command sets give.
fn command_overrides() -> Arc<Overrides> {
    static OVERRIDES: OnceLock<Arc<Overrides>> = OnceLock::new();
    OVERRIDES
        .get_or_init(|| {
            let mut overrides = Overrides::new();
            for id in [QuitCommand::Quit.id(), NavCommand::NextPane.id(), NavCommand::PreviousPane.id(), ChromeCommand::TogglePanelsKeepTools.id()] {
                overrides.insert(id, Vec::new());
            }
            Arc::new(overrides)
        })
        .clone()
}

impl TextureCraftApp {
    /// The framework's part of `App::raw_input_hook`, once per frame before the first pass: takes what the runtime
    /// gathered since the last frame and gives the coming passes the root values (theme mode and interface scale).
    pub fn before_input(&mut self, ctx: &egui::Context) {
        // TextureCraft applies the interface scale itself (`logic`), so the runtime keeps the zoom it applied last.
        let scale = ctx.zoom_factor();
        embed::before_input(ctx, |output| self.frame_input(ctx, output, scale));
    }

    /// The root values of this frame. TextureCraft registers no commands with the runtime (its own shortcut table
    /// runs every key, [`command_overrides`]). The runtime toggles the chrome on a bare Tab with nothing or a
    /// view focused (`tab_toggles_chrome`, the views being the registered canvases) and takes it out of egui's input, and
    /// each panels toggle it recorded for the root window flips Hide UI, as the `ui.hideUi` command does. While one of
    /// TextureCraft's own egui modals shows in the root window, the runtime blocks the panels floated into windows of
    /// their own.
    pub(crate) fn frame_input(&mut self, ctx: &egui::Context, output: FrameOutput, scale: f32) -> FrameInput {
        for (viewport, toggle) in &output.chrome_toggles {
            if *viewport == ViewportId::ROOT && *toggle == ChromeToggle::Panels {
                self.ui.hide_ui = !self.ui.hide_ui;
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.fired_clipboard.extend(clipboard_events(&output));
        use crate::theme::Brightness;
        let mode = match self.ui.prefs.brightness {
            Brightness::Light | Brightness::MediumLight => craft_theme::Mode::Light,
            Brightness::Dark | Brightness::MediumDark => craft_theme::Mode::Dark,
        };
        let mut root = RootEnv::new().mode(mode).scale(Scale::new(scale));
        // Hide UI on another key leaves Tab to focus traversal and the shortcuts
        root.tab_toggles_chrome = crate::keymap::shortcut_of(&self.ui.keymap, "ui.hideUi") == Some("Tab");
        let app_modal = crate::docking::input_blocked(self, ctx).then_some(ViewportId::ROOT);
        FrameInput::new(root).commands(CommandInput::new().overrides(command_overrides())).host(Arc::clone(&self.framework_host)).app_modal(app_modal)
    }

    /// The framework's part of `App::logic`, after TextureCraft's own: the root window's close path. Unsaved changes
    /// hold the window open while the Unsaved Changes dialog asks ([`crate::windows::close_requested`]); otherwise
    /// it closes.
    pub fn close_path(&mut self, ctx: &egui::Context) {
        embed::logic(ctx, || if crate::windows::close_requested(self) { CloseDecision::Hold } else { CloseDecision::Close });
    }
}

/// The Copy, Cut and Paste events the runtime took from windows with no canvas of their own (a floating panel) and ran as
/// its `edit.copy`, `edit.cut` and `edit.paste`: the clipboard events TextureCraft's shortcuts read in the root window.
/// The other edit commands are not read here: TextureCraft's own loop still forwards their keys.
#[cfg(not(target_arch = "wasm32"))]
fn clipboard_events(output: &FrameOutput) -> impl Iterator<Item = egui::Event> + '_ {
    output.fired.iter().filter_map(|fired| match EditCommands::from_ref(&fired.command)? {
        EditCommands::Copy => Some(egui::Event::Copy),
        EditCommands::Cut => Some(egui::Event::Cut),
        // The layer paste takes no content from the event; the shortcut only needs the paste chord.
        EditCommands::Paste => Some(egui::Event::Paste(String::new())),
        _ => None,
    })
}

/// Declares the view `id`, an egui widget drawn in this pass, as a document canvas of the window. With nothing or the
/// view focused, the edit chords and the clipboard events stay in egui's input for TextureCraft's shortcuts, and a bare
/// Tab is the runtime's chrome toggle (`frame_input`). Tab and Shift+Tab stop at the view, Shift+Tab moves on from it,
/// and a click gives it no focus.
pub(crate) fn host_canvas(ctx: &egui::Context, id: egui::Id) {
    const KEYS: KeyUse = KeyUse::CANVAS.with_edits(EditSet::ALL);
    craft_ui::runtime::focus::host_canvas(ctx, id, KEYS);
}

/// The widget that has the keyboard, the 3D and 2D views left out: a focused view leaves the keyboard to
/// TextureCraft's shortcuts, as with nothing focused.
pub(crate) fn focused_widget(ctx: &egui::Context) -> Option<egui::Id> {
    use crate::viewport::{view_2d_id, view_3d_id};
    ctx.memory(|memory| memory.focused()).filter(|focused| ![view_3d_id(), view_2d_id()].contains(focused))
}

/// Whether a widget other than the 3D and 2D views has the keyboard.
pub(crate) fn widget_has_keyboard(ctx: &egui::Context) -> bool {
    focused_widget(ctx).is_some()
}

/// Whether Escape was pressed in this pass: from egui while one of TextureCraft's own widgets or popups has the
/// keyboard, otherwise the window's cancel, which the runtime routes to no component. Leaves it for later readers.
pub(crate) fn escape_pressed(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.key_pressed(egui::Key::Escape)) || window_cancel(ctx, false)
}

/// Takes the window's cancel (an Escape while nothing has framework focus) when it is there: no later reader sees it.
pub(crate) fn take_window_cancel(ctx: &egui::Context) -> bool {
    window_cancel(ctx, true)
}

/// Takes the window's submit (a plain Enter while nothing has framework focus) when it is there: no later reader sees
/// it.
pub(crate) fn take_window_submit(ctx: &egui::Context) -> bool {
    window_intent(ctx, Intent::Submit, true)
}

/// Whether the window's cancel is there and untaken; `take` takes it.
fn window_cancel(ctx: &egui::Context, take: bool) -> bool {
    window_intent(ctx, Intent::Cancel, take)
}

/// Whether the window's `intent` is there and untaken; `take` takes it.
fn window_intent(ctx: &egui::Context, intent: Intent, take: bool) -> bool {
    let mut pending = false;
    with_window_intents(ctx, false, |intents| {
        pending = if take { intents.take(intent.clone()) } else { intents.untaken(&intent) };
        take && pending
    });
    pending
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use craft_ui::runtime::commands::{FirePhase, FireSource, Fired};
    use egui_kittest::Harness;

    fn fired(command: EditCommands) -> Fired {
        Fired::new(command.into(), FireSource::Key, FirePhase::Press, 0.0)
    }

    /// With nothing focused in a floating panel, the runtime runs the chord as `edit.copy` and takes the clipboard event
    /// from egui's input, so TextureCraft's own loop never sees it: the fired command reaches the layer clipboard once.
    #[test]
    fn fired_copy_and_paste_from_a_float_reach_the_layer_clipboard_once() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let session = texturecraft_engine::sample_session(256, 64).unwrap();
        let mut h =
            Harness::builder().with_size(egui::vec2(1600.0, 960.0)).with_context_setup(install_headless).build_eframe(|cc| TextureCraftApp::new(cc, session));
        h.run_steps(4);
        let ctx = h.ctx.clone();
        let layers = |h: &Harness<'_, TextureCraftApp>| h.state().session.doc.project.layers.len();
        let before = layers(&h);

        let mut output = FrameOutput::default();
        output.fired.push(fired(EditCommands::Copy));
        h.state_mut().frame_input(&ctx, output, 1.0);
        h.run_steps(3);
        assert!(h.state().session.clipboard.is_some(), "the fired Copy copied the layer");
        assert_eq!(layers(&h), before);

        let mut output = FrameOutput::default();
        output.fired.push(fired(EditCommands::Paste));
        h.state_mut().frame_input(&ctx, output, 1.0);
        h.run_steps(3);
        assert_eq!(layers(&h), before + 1, "the fired Paste pasted the layer once");
    }
}
