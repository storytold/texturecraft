//! Keyboard shortcuts: every menu item and every command a key can run, the user's
//! overrides (`UiState::keymap`, id → "Ctrl+Shift+E" or "" for unbound), conflict detection and
//! the shortcut editor window (Help ▸ Shortcuts list) and the Edit ▸ Settings ▸ Shortcuts page.

use std::collections::BTreeMap;

use craft_keymap::egui::Dispatcher;
use craft_keymap::{Active, Bindable, Chord, CommandRef, ConflictKind, KeyInput, Layout, Lookup, Mods, Overrides, Platform, Scope, Scopes};
use egui::{Event, Id, Key, KeyboardShortcut, Modifiers, RichText, vec2};
use egui_phosphor::regular as ic;

use crate::TextureCraftApp;
use crate::i18n::{tr, trf};
use crate::menus::CATALOG;
use crate::theme::{Tokens, caption};
use crate::unavailable::NOT_AVAILABLE;
use crate::widgets::{icon_button, section};

mod shortcut_list_entry;

use shortcut_list_entry::ShortcutListEntry;

pub type Keymap = BTreeMap<String, String>;

/// Something a shortcut can trigger.
pub struct Binding {
    pub id: &'static str,
    /// Menu (or command group) it belongs to.
    pub group: &'static str,
    pub label: &'static str,
    pub default: Option<&'static str>,
}

/// Rows the reference lists whose function TextureCraft does not have yet, as (listed before the
/// row of this id, action, shortcut). Settings > Shortcuts shows them disabled.
///
/// They are not in [`bindings`], so no key runs them, but another item given their key is reported
/// as sharing it.
pub static NOT_AVAILABLE_ACTIONS: [(&str, Binding); 2] = [
    ("file.reimportMesh", Binding { id: "ui.pauseEngine", group: "Commands", label: "Pause Engine Computation", default: Some("Shift+Esc") }),
    ("viewport.frame", Binding { id: "ui.toggleAnimation", group: "Viewport", label: "Toggle animation", default: Some("Space") }),
];

/// The shortcut list's first rows, in the reference's order. The `>` and `<` of the reference are
/// the shifted Period and Comma keys.
const LISTED_SHORTCUTS: &[ShortcutListEntry] = &[
    ShortcutListEntry::new("ui.hideUi", "Hide UI", Some("Tab")),
    ShortcutListEntry::new("edit.undo", "Undo", Some("Ctrl+Z")),
    ShortcutListEntry::new("edit.redo", "Redo", Some("Ctrl+Y")),
    ShortcutListEntry::new("file.open", "Open project", Some("Ctrl+O")),
    ShortcutListEntry::new("file.close", "Close project", Some("Ctrl+F4")),
    ShortcutListEntry::new("file.save", "Save project", Some("Ctrl+S")),
    ShortcutListEntry::new("file.saveAs", "Save as project", Some("Ctrl+Shift+S")),
    ShortcutListEntry::new("ui.newProject", "New project", Some("Ctrl+N")),
    ShortcutListEntry::new("ui.quit", "Quit application", Some("Ctrl+Q")),
    ShortcutListEntry::new("ui.export", "Export Textures...", Some("Ctrl+Shift+E")),
    ShortcutListEntry::new("file.reimportMesh", "Reimport mesh", Some("Ctrl+Shift+R")),
    ShortcutListEntry::new("ui.mode.bake", "Switch to baking mode", Some("Ctrl+Shift+B")),
    ShortcutListEntry::new("ui.mode.paint", "Switch to edition mode", Some("F9")),
    ShortcutListEntry::new("ui.mode.render", "Switch to rendering mode (path traced)", Some("F10")),
    ShortcutListEntry::new("ui.wireframe", "Show mesh wireframe", None),
    ShortcutListEntry::new("camera.perspective", "Camera projection: Perspective view", Some("F5")),
    ShortcutListEntry::new("camera.orthographic", "Camera projection: Orthographic view", Some("F6")),
    ShortcutListEntry::new("viewport.nextChannel", "Display next channel", Some("C")),
    ShortcutListEntry::new("viewport.previousChannel", "Display previous channel", Some("Shift+C")),
    ShortcutListEntry::new("viewport.displayMaterial", "Display material", Some("M")),
    ShortcutListEntry::new("viewport.nextMeshMap", "Display next mesh map", Some("B")),
    ShortcutListEntry::new("viewport.previousMeshMap", "Display previous mesh map", Some("Shift+B")),
    ShortcutListEntry::new("viewport.frame", "Show entire mesh/path", Some("F")),
    ShortcutListEntry::new("camera.next", "Select next camera", Some("Shift+Period")),
    ShortcutListEntry::new("camera.previous", "Select previous camera", Some("Shift+Comma")),
    ShortcutListEntry::new("viewport.quickMask", "Enable quick mask", Some("Y")),
    ShortcutListEntry::new("viewport.quickMask.edit", "Edit quick mask", Some("U")),
    ShortcutListEntry::new("viewport.quickMask.invert", "Invert quick mask", Some("Ctrl+Shift+I")),
    ShortcutListEntry::new("ui.layout.split", "Viewers layout: 3D/2D", Some("F1")),
    ShortcutListEntry::new("ui.layout.3d", "Viewers layout: 3D only", Some("F2")),
    ShortcutListEntry::new("ui.layout.2d", "Viewers layout: 2D only", Some("F3")),
    ShortcutListEntry::new("ui.layout.swap", "Viewers layout: Swap 3D/2D", Some("F4")),
];

/// The menu a command without a menu entry is listed under, from its id.
fn group_of(id: &str) -> &'static str {
    match id.split('.').next().unwrap_or("") {
        "layer" | "mask" | "effect" | "material" | "textureSet" => "Layers",
        "tool" | "brush" | "alpha" | "text" => "Tools",
        "camera" | "viewport" => "Viewport",
        "file" | "project" | "export" | "resource" => "File",
        "edit" => "Edit",
        "bake" => "Mode",
        _ => "Commands",
    }
}

/// Every bindable item: the menu catalog, then every command that has a default shortcut or that
/// a key can run (see [`texturecraft_engine::CommandSpec::bindable`]).
pub fn bindings() -> &'static [Binding] {
    static ALL: std::sync::OnceLock<Vec<Binding>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut v: Vec<Binding> = Vec::new();
        for entry in LISTED_SHORTCUTS {
            let group = match CATALOG.iter().find(|i| i.id == entry.id) {
                Some(item) => item.menu,
                None => texturecraft_engine::command_specs().iter().find(|c| c.id == entry.id).and_then(|c| c.menu.first().copied()).unwrap_or("Viewport"),
            };
            v.push(Binding { id: entry.id, group, label: entry.label, default: entry.default });
        }
        for i in CATALOG.iter().filter(|i| i.label != "-" && !crate::menus::UNAVAILABLE.contains(&i.id)) {
            if !v.iter().any(|b| b.id == i.id) {
                v.push(Binding { id: i.id, group: i.menu, label: i.label, default: i.shortcut });
            }
        }
        for c in texturecraft_engine::command_specs().iter().filter(|c| c.shortcut.is_some() || c.bindable()) {
            if !v.iter().any(|b| b.id == c.id) {
                v.push(Binding { id: c.id, group: c.menu.first().copied().unwrap_or_else(|| group_of(c.id)), label: c.label, default: c.shortcut });
            }
        }
        v
    })
}

/// The shortcut `b` currently has.
pub fn shortcut<'a>(keymap: &'a Keymap, b: &'a Binding) -> Option<&'a str> {
    match keymap.get(b.id) {
        Some(s) => (!s.is_empty()).then_some(s.as_str()),
        None => b.default,
    }
}

/// The shortcut of item `id` (for menu labels and tooltips).
pub fn shortcut_of<'a>(keymap: &'a Keymap, id: &str) -> Option<&'a str> {
    bindings().iter().find(|b| b.id == id).and_then(|b| shortcut(keymap, b))
}

/// "Label (Shortcut)" for an icon button's tooltip.
pub fn tip(keymap: &Keymap, label: &str, id: &str) -> String {
    match shortcut_of(keymap, id) {
        Some(s) => format!("{label} ({s})"),
        None => label.to_owned(),
    }
}

/// craft-keymap's text of a shortcut: "Ctrl" here is the command modifier (Cmd on macOS), which the
/// crate writes "Cmd".
fn crate_text(text: &str) -> String {
    text.split('+').map(|part| if part == "Ctrl" { "Cmd" } else { part }).collect::<Vec<_>>().join("+")
}

/// Read "Ctrl+Shift+E" notation.
pub fn parse(text: &str) -> Option<Chord> {
    Chord::parse(&crate_text(text)).ok()
}

/// "Ctrl+Shift+E" notation (the inverse of [`parse`]).
pub fn format_shortcut(sc: &KeyboardShortcut) -> String {
    let m = sc.modifiers;
    let mut s = String::new();
    if m.command || m.ctrl || m.mac_cmd {
        s.push_str("Ctrl+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    if m.alt {
        s.push_str("Alt+");
    }
    s.push_str(sc.logical_key.name());
    s
}

/// Every bound shortcut as egui's shortcut type.
fn active(keymap: &Keymap) -> Vec<KeyboardShortcut> {
    bindings().iter().filter_map(|b| Some(parse(shortcut(keymap, b)?)?.to_egui_shortcut())).collect()
}

/// The catalog as craft-keymap sees it, in `bindings()` order: the first one listed wins a shared
/// shortcut.
fn catalog() -> Vec<Bindable> {
    bindings()
        .iter()
        .map(|b| {
            let item = Bindable::new(b.id, b.label, Scope::app());
            match b.default.and_then(parse) {
                Some(chord) => item.with_default(craft_keymap::Binding::new(chord)),
                None => item,
            }
        })
        .collect()
}

/// craft-keymap's text of a stored shortcut. A symbol that needs Shift is looked up without it, so
/// "Shift+Colon" is ":".
fn override_text(text: &str) -> String {
    match parse(text) {
        Some(chord) => Layout::new().resolve(&KeyInput::new(chord.mods).with_typed(chord.key)).unwrap_or(chord).to_string(),
        None => crate_text(text),
    }
}

/// The stored shortcuts as craft-keymap overrides ("" = no shortcut).
fn overrides(keymap: &Keymap) -> Overrides {
    let mut out = Overrides::new();
    for (id, text) in keymap {
        let texts = if text.is_empty() { Vec::new() } else { vec![override_text(text)] };
        // Only an overlong id or text is refused, and neither can be a shortcut.
        let _ = out.insert(id.clone(), texts);
    }
    out
}

/// The keymap with `keymap`'s shortcuts for `platform`.
fn resolved(keymap: &Keymap, platform: Platform) -> craft_keymap::Keymap {
    craft_keymap::Keymap::new(catalog(), Scopes::new(), platform).with_overrides(overrides(keymap))
}

/// The resolved keymap and the dispatcher that runs it against each frame's keys.
pub struct Bindings {
    stored: Keymap,
    map: craft_keymap::Keymap,
    dispatcher: Dispatcher,
}

impl Default for Bindings {
    fn default() -> Bindings {
        let stored = Keymap::new();
        let map = resolved(&stored, Platform::current());
        // Held keys repeat their command. Copy, Cut and Paste events count as Ctrl+C, X and V.
        Bindings { stored, map, dispatcher: Dispatcher::new().with_repeat(true).with_clipboard_events(true) }
    }
}

impl Bindings {
    /// Follows the stored shortcuts and the platform; a no-op while neither changed.
    pub(crate) fn sync(&mut self, keymap: &Keymap, platform: Platform) {
        if self.stored != *keymap || self.map.platform() != platform {
            self.stored = keymap.clone();
            self.map = resolved(keymap, platform);
        }
    }

    /// The ids of the commands this frame's key presses run, in press order. The presses are
    /// removed from the input.
    pub(crate) fn dispatch(&mut self, input: &mut egui::InputState) -> Vec<String> {
        self.dispatcher.dispatch(input, &self.map, &Active::NONE).into_iter().map(|hit| hit.command.id).collect()
    }

    /// The id of the command a key press outside the main window runs. The press is matched as
    /// the main window's presses are.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn command_of(&self, press: &Event) -> Option<String> {
        let input = KeyInput::from_egui(press, self.map.platform())?;
        self.dispatcher.layout().candidates(&input).into_iter().find_map(|chord| match self.map.lookup(&chord, false, &Active::NONE) {
            Lookup::Run(command) => Some(command.id),
            Lookup::Cycle(commands) => commands.into_iter().next().map(|command| command.id),
            _ => None,
        })
    }

    /// The shortcut a press records: the chord it runs, keeping the held Ctrl and Cmd as they are.
    fn recorded(&self, press: &Event, key: Key, modifiers: Modifiers, platform: Platform) -> KeyboardShortcut {
        let chord = KeyInput::from_egui(press, platform).and_then(|input| self.dispatcher.layout().resolve(&input));
        match chord {
            Some(chord) => KeyboardShortcut::new(Modifiers { shift: chord.mods.shift, ..modifiers }, chord.to_egui_shortcut().logical_key),
            None => KeyboardShortcut::new(modifiers, key),
        }
    }

    /// Whether a Shift-only Cut or Paste, read as Shift+Delete or Shift+Insert, runs nothing.
    pub(crate) fn shift_clipboard_is_free(&self, event: &Event) -> bool {
        let key = match event {
            Event::Cut => craft_keymap::Key::Delete,
            Event::Paste(_) => craft_keymap::Key::Insert,
            _ => return false,
        };
        matches!(self.map.lookup(&Chord::new(Mods::SHIFT, key), false, &Active::NONE), Lookup::None)
    }

    /// Another binding on the same shortcut as `b`.
    fn conflict(&self, b: &Binding) -> Option<&'static Binding> {
        let clash = self.map.conflicts_with(&CommandRef::new(b.id)).into_iter().find(|c| c.kind == ConflictKind::Clash);
        if let Some(clash) = clash {
            return bindings().iter().find(|x| x.id == clash.b.0.id);
        }

        // The rows the reference lists and TextureCraft cannot run yet still own their keys
        let mine = parse(shortcut(&self.stored, b)?)?;
        NOT_AVAILABLE_ACTIONS.iter().map(|(_, row)| row).find(|row| row.default.and_then(parse) == Some(mine))
    }
}

/// Bind `b` to `value` ("" = no shortcut); the default is stored as no override.
pub fn rebind(keymap: &mut Keymap, b: &Binding, value: &str) {
    if b.default.unwrap_or("") == value {
        keymap.remove(b.id);
    } else {
        keymap.insert(b.id.to_owned(), value.to_owned());
    }
}

/// The bound shortcuts the browser must not act on, for the web build's `keydown` listener:
/// the keymap they were taken from and the shortcuts.
static CAPTURED: std::sync::Mutex<Option<(Keymap, Vec<KeyboardShortcut>)>> = std::sync::Mutex::new(None);

/// Refresh [`captured`] when the keymap changed (once per frame; cheap when it didn't).
pub fn sync_captured(keymap: &Keymap) {
    let mut c = CAPTURED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if c.as_ref().is_some_and(|(k, _)| k == keymap) {
        return;
    }
    // Copy, cut and paste stay with the browser: egui reads them as clipboard events.
    let clipboard = [Key::C, Key::V, Key::X];
    let keys: Vec<KeyboardShortcut> = active(keymap)
        .into_iter()
        .filter(|sc| {
            let fkey = sc.logical_key.name().strip_prefix('F').is_some_and(|n| n.parse::<u8>().is_ok());
            (sc.modifiers.command || sc.modifiers.alt || fkey) && !(sc.modifiers.command && !sc.modifiers.shift && clipboard.contains(&sc.logical_key))
        })
        .collect();
    log::debug!("browser shortcuts taken over: {}", keys.len());
    *c = Some((keymap.clone(), keys));
}

/// Whether a key press is one of the app's shortcuts the browser must leave alone.
pub fn captured(mods: Modifiers, key: Key) -> bool {
    let c = CAPTURED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    c.as_ref().is_some_and(|(_, keys)| keys.iter().any(|sc| sc.logical_key == key && mods.matches_logically(sc.modifiers)))
}

const CAPTURE: &str = "keymap_capture";
/// The binding Settings > Shortcuts waits a key combination for.
const SETTINGS_CAPTURE: &str = "settings_keymap_capture";

/// Whether the editor is waiting for a key combination (global shortcuts pause meanwhile).
pub fn capturing(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<usize>(Id::unique(CAPTURE)).is_some() || d.get_temp::<usize>(Id::unique(SETTINGS_CAPTURE)).is_some())
}

/// Settings > Shortcuts stops waiting for a key combination (another page, OK or Cancel).
pub(crate) fn stop_settings_capture(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<usize>(Id::unique(SETTINGS_CAPTURE)));
}

/// The mouse shortcuts of the viewports the reference lists on its Shortcuts page, as (action,
/// binding). The viewports read these gestures from code, so the rows are shown and not editable.
pub const MOUSE_SHORTCUTS: [(&str, &str); 9] = [
    ("Use tool", "Left"),
    ("Camera rotate", "Alt+Left"),
    ("Camera snap rotate", "Alt+Shift+Left"),
    ("Camera translate", "Alt+Middle"),
    ("Camera translate (alt.)", "Ctrl+Alt+Left"),
    ("Camera zoom", "Alt+Right"),
    ("Rotate environment", "Shift+Right"),
    ("Auto focus", "Ctrl+Middle"),
    ("Context menu", "Right"),
];

/// The hold-key modifiers the reference lists on its Shortcuts page, as (action, key); not editable yet.
pub const MODIFIER_SHORTCUTS: [(&str, &str); 2] = [("Ignore stencil mask", "N"), ("Resume previous stroke", "A")];

/// The reference's shortcut text of `text`: `>` and `<` are the shifted Period and Comma keys.
pub fn shown(text: &str) -> &str {
    match text {
        "Shift+Period" => ">",
        "Shift+Comma" => "<",
        other => other,
    }
}

/// Whether the shortcuts in `edited` (kept in step with the working copy) have an item that
/// shares its shortcut with another.
pub(crate) fn has_conflict(edited: &Bindings) -> bool {
    bindings().iter().any(|b| edited.conflict(b).is_some())
}

/// Follow `keymap` with the checker of the Settings window.
pub(crate) fn follow(edited: &mut Bindings, keymap: &Keymap, ctx: &egui::Context) {
    edited.sync(keymap, craft_keymap::egui::platform(ctx));
}

/// Settings > Shortcuts: the sections "Actions", "Mouse shortcuts" and "Modifiers shortcuts" and
/// "Reset all to default". An action row has its name, its shortcut in `keymap`, a pencil that
/// waits for the new key combination (Esc cancels, Delete clears the shortcut) and a warning
/// naming the other action on the same shortcut. `recorder` reads the pressed keys as the app's
/// shortcuts do; `edited` follows `keymap` to show shared shortcuts.
pub(crate) fn actions_page(ui: &mut egui::Ui, keymap: &mut Keymap, recorder: &Bindings, edited: &mut Bindings) {
    let ctx = ui.ctx().clone();
    let tk = Tokens::get(&ctx);
    let capture_id = Id::unique(SETTINGS_CAPTURE);
    let mut capture: Option<usize> = ctx.data(|d| d.get_temp(capture_id));
    if let (Some(i), Some(sc)) = (capture, pressed(&ctx, recorder)) {
        if let Some(b) = bindings().get(i) {
            if sc.logical_key == Key::Delete && sc.modifiers.is_none() {
                rebind(keymap, b, "");
            } else if sc.logical_key != Key::Escape || !sc.modifiers.is_none() {
                rebind(keymap, b, &format_shortcut(&sc));
            }
        }
        capture = None;
    }
    follow(edited, keymap, &ctx);

    let cell = |ui: &mut egui::Ui, width: f32, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.allocate_ui_with_layout(vec2(width, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(width);
            add(ui);
        });
    };

    // A pencil that cannot be used yet
    let locked_pencil = |ui: &mut egui::Ui, action: &str| {
        let edit = trf("Edit the shortcut of {item}", &[("item", &tr(action))]);
        ui.add_enabled_ui(false, |ui| icon_button(ui, ic::PENCIL_SIMPLE, &edit, false, 20.0)).inner.on_disabled_hover_text(tr(NOT_AVAILABLE));
    };
    let locked_row = |ui: &mut egui::Ui, action: &str, binding: &str| {
        ui.horizontal(|ui| {
            cell(ui, 300.0, &mut |ui| {
                ui.add(egui::Label::new(tr(action)).truncate());
            });
            cell(ui, 130.0, &mut |ui| {
                ui.label(binding);
            });
            locked_pencil(ui, action);
        });
    };

    ui.label(caption(tr("Actions")));
    ui.add_space(4.0);
    for (i, b) in bindings().iter().enumerate() {
        for (_, row) in NOT_AVAILABLE_ACTIONS.iter().filter(|(before, _)| *before == b.id) {
            locked_row(ui, row.label, row.default.unwrap_or(""));
        }
        let waiting = capture == Some(i);
        ui.horizontal(|ui| {
            cell(ui, 300.0, &mut |ui| {
                ui.add(egui::Label::new(tr(b.label)).truncate()).on_hover_text(tr(b.group));
            });
            cell(ui, 130.0, &mut |ui| {
                if waiting {
                    ui.label(RichText::new(tr("Press a key...")).color(tk.accent));
                } else {
                    ui.label(shown(shortcut(keymap, b).unwrap_or("")));
                }
            });
            let edit = trf("Edit the shortcut of {item}", &[("item", &tr(b.label))]);
            if icon_button(ui, ic::PENCIL_SIMPLE, &edit, waiting, 20.0).clicked() {
                ctx.data_mut(|data| data.remove::<usize>(Id::unique(CAPTURE)));
                capture = if waiting { None } else { Some(i) };
            }
            if let Some(other) = edited.conflict(b) {
                ui.label(RichText::new(ic::WARNING).color(tk.danger)).on_hover_text(tr(other.label));
            }
        });
    }

    ui.add_space(10.0);
    ui.label(caption(tr("Mouse shortcuts")));
    ui.add_space(4.0);
    for (action, binding) in MOUSE_SHORTCUTS {
        locked_row(ui, action, binding);
    }

    ui.add_space(10.0);
    ui.label(caption(tr("Modifiers shortcuts")));
    ui.add_space(4.0);
    for (action, binding) in MODIFIER_SHORTCUTS {
        locked_row(ui, action, binding);
    }

    ui.add_space(12.0);
    if ui.button(tr("Reset all to default")).clicked() {
        keymap.clear();
        capture = None;
        follow(edited, keymap, &ctx);
    }

    ctx.data_mut(|d| match capture {
        Some(i) => {
            d.insert_temp(capture_id, i);
        }
        None => {
            d.remove::<usize>(capture_id);
        }
    });
}

/// The first key press this frame as a shortcut.
fn pressed(ctx: &egui::Context, bindings: &Bindings) -> Option<KeyboardShortcut> {
    let platform = craft_keymap::egui::platform(ctx);
    ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } => Some(bindings.recorded(e, *key, *modifiers, platform)),
            _ => None,
        })
    })
}

/// The shortcut editor window.
pub fn window(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let tk = Tokens::get(ctx);
    if !app.ui.windows.shortcuts {
        ctx.data_mut(|data| data.remove::<usize>(Id::unique(CAPTURE)));
        return;
    }

    let capture_id = Id::unique(CAPTURE);
    let filter_id = Id::unique("keymap_filter");
    let mut capture: Option<usize> = ctx.data(|d| d.get_temp(capture_id));
    if let (Some(i), Some(sc)) = (capture, pressed(ctx, &app.bindings)) {
        if (sc.logical_key != Key::Escape || !sc.modifiers.is_none())
            && let Some(b) = bindings().get(i)
        {
            rebind(&mut app.ui.keymap, b, &format_shortcut(&sc));
        }
        capture = None;
    }
    app.bindings.sync(&app.ui.keymap, craft_keymap::egui::platform(ctx));
    let mut filter: String = ctx.data(|d| d.get_temp(filter_id)).unwrap_or_default();
    let mut open = true;
    crate::windows::window("keyboard_shortcuts", tr("Keyboard shortcuts"), &mut open).default_width(480.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(ic::MAGNIFYING_GLASS).color(tk.text_dim));
            ui.add(egui::TextEdit::singleline(&mut filter).hint_text(tr("Search commands or keys")).desired_width(300.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!app.ui.keymap.is_empty(), egui::Button::new(tr("Reset all"))).on_hover_text(tr("Restore every default shortcut")).clicked() {
                    app.ui.keymap.clear();
                }
            });
        });
        let clashes = bindings().iter().filter(|b| app.bindings.conflict(b).is_some()).count();
        if clashes > 0 {
            ui.label(
                RichText::new(format!("{} {}", ic::WARNING, trf("{n} items share a shortcut; the first one listed wins.", &[("n", &clashes)])))
                    .color(tk.danger),
            );
        }
        ui.add_space(4.0);
        let needle = filter.to_lowercase();
        egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("keymap_grid").num_columns(3).striped(true).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
                for (i, b) in bindings().iter().enumerate() {
                    for (_, row) in NOT_AVAILABLE_ACTIONS.iter().filter(|(before, _)| *before == b.id) {
                        let key = row.default.unwrap_or("");
                        if !needle.is_empty() && ![row.label, row.group, tr(row.label), tr(row.group), key].iter().any(|t| t.to_lowercase().contains(&needle)) {
                            continue;
                        }

                        ui.horizontal(|ui| {
                            ui.label(tr(row.label));
                            ui.label(RichText::new(tr(row.group)).size(10.5).color(tk.text_dim));
                        });
                        let button = egui::Button::new(RichText::new(key).monospace()).min_size(vec2(130.0, 0.0));
                        ui.add_enabled(false, button).on_disabled_hover_text(tr(NOT_AVAILABLE));
                        ui.end_row();
                    }
                    let current = shortcut(&app.ui.keymap, b);
                    let current_text = current.map(shown);
                    if !needle.is_empty()
                        && ![b.label, b.group, tr(b.label), tr(b.group), b.id, current.unwrap_or(""), current_text.unwrap_or("")]
                            .iter()
                            .any(|t| t.to_lowercase().contains(&needle))
                    {
                        continue;
                    }

                    ui.horizontal(|ui| {
                        ui.label(tr(b.label));
                        ui.label(RichText::new(tr(b.group)).size(10.5).color(tk.text_dim));
                    });
                    let waiting = capture == Some(i);
                    let text = if waiting { tr("Press a key...") } else { current_text.unwrap_or("-") };
                    let key = egui::Button::new(RichText::new(text).monospace()).selected(waiting).min_size(vec2(130.0, 0.0));
                    if ui.add(key).on_hover_text(tr("Click, then press the new shortcut (Esc cancels)")).clicked() {
                        stop_settings_capture(ctx);
                        capture = if waiting { None } else { Some(i) };
                    }
                    let bound = current.is_some();
                    ui.horizontal(|ui| {
                        if let Some(other) = app.bindings.conflict(b) {
                            ui.label(RichText::new(ic::WARNING).color(tk.danger))
                                .on_hover_text(trf("Also bound to {item} ({menu})", &[("item", &tr(other.label)), ("menu", &tr(other.group))]));
                        }
                        if bound && icon_button(ui, ic::X, tr("Remove the shortcut"), false, 18.0).clicked() {
                            rebind(&mut app.ui.keymap, b, "");
                        }
                        if app.ui.keymap.contains_key(b.id)
                            && icon_button(
                                ui,
                                ic::ARROW_COUNTER_CLOCKWISE,
                                &trf("Reset to {shortcut}", &[("shortcut", &b.default.unwrap_or(tr("none")))]),
                                false,
                                18.0,
                            )
                            .clicked()
                        {
                            app.ui.keymap.remove(b.id);
                        }
                    });
                    ui.end_row();
                }
            });
        });
        ui.add_space(4.0);
        section(ui, "keymap_nav", tr("Viewport and brush (fixed)"), false, |ui| {
            fixed_gestures_grid(ui);
        });
    });
    app.ui.windows.shortcuts = open;
    if !open {
        capture = None;
    }
    ctx.data_mut(|d| {
        d.insert_temp(filter_id, filter);
        match capture {
            Some(i) => {
                d.insert_temp(capture_id, i);
            }
            None => {
                d.remove::<usize>(capture_id);
            }
        }
    });
}

/// The fixed gestures as a two-column grid (the "Viewport and brush" section of the shortcut window)
pub fn fixed_gestures_grid(ui: &mut egui::Ui) {
    egui::Grid::new("keymap_fixed").striped(true).show(ui, |ui| {
        for (k, v) in fixed_gestures() {
            ui.label(RichText::new(k).strong());
            ui.label(v);
            ui.end_row();
        }
    });
}

/// The mouse gestures and hold keys of the viewports, as (gesture, what it does) rows for the
/// shortcut window. The hold keys come from the constants the viewport reads, so a rebound hold key
/// is named from the same constant.
pub fn fixed_gestures() -> Vec<(String, String)> {
    let stencil = crate::viewport::STENCIL_KEY.name();
    let clone = crate::viewport::CLONE_SOURCE_KEY.name();
    let keyed = |template: &str| trf(template, &[("key", &stencil)]);
    vec![
        (tr("Alt + LMB drag").into(), tr("Orbit").into()),
        (tr("Alt + Shift + LMB drag").into(), tr("Camera snap rotate").into()),
        (tr("RMB drag").into(), tr("Orbit (3D view)").into()),
        (tr("MMB drag / Alt + MMB").into(), tr("Pan").into()),
        (tr("Wheel / Alt + RMB drag").into(), tr("Zoom").into()),
        (tr("Shift + RMB drag").into(), tr("Turn the environment light (3D view)").into()),
        (tr("LMB").into(), tr("Paint with the current tool").into()),
        (tr("Shift + LMB").into(), tr("Paint a straight line from where the last stroke ended").into()),
        (tr("Ctrl + Shift + LMB").into(), tr("Draw snapped straight lines").into()),
        ("[ / ]".into(), tr("Brush size down / up").into()),
        (tr("Shift + wheel").into(), tr("Brush size").into()),
        (tr("Ctrl + wheel").into(), tr("Brush size (wheel up smaller, down larger)").into()),
        (tr("Ctrl + RMB drag").into(), tr("Brush size (left/right) and hardness (up/down)").into()),
        (tr("Ctrl + LMB drag").into(), tr("Brush flow (left/right) and angle (up/down)").into()),
        (tr("Ctrl + Shift + RMB drag").into(), tr("Brush size and hardness in fine steps").into()),
        (keyed("{key} + LMB drag"), tr("Projection: rotate the stencil").into()),
        (keyed("{key} + RMB drag"), tr("Projection: scale the stencil").into()),
        (keyed("{key} + MMB drag"), tr("Projection: move the stencil").into()),
        (trf("{key} + LMB", &[("key", &clone)]), tr("Clone: pick the point to copy from").into()),
        (tr("Click (Path tool)").into(), tr("Path: add a point; on the curve it inserts one").into()),
        (tr("LMB drag on a point (Path tool)").into(), tr("Path: move the point").into()),
        (tr("Click the first point (Path tool)").into(), tr("Path: close it (needs 3 points)").into()),
        ("Enter".into(), tr("Path: apply it with the brush").into()),
        ("Backspace".into(), tr("Path: remove the selected or last point").into()),
        ("Esc".into(), tr("Cancel the stroke").into()),
        (tr("Esc (Path tool)").into(), tr("Path: cancel it").into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_browser_leaves_app_shortcuts_alone_but_keeps_the_clipboard() {
        let mut keymap = Keymap::new();
        keymap.insert("file.close".into(), "Alt+W".into());
        sync_captured(&keymap);
        let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
        assert!(captured(ctrl, Key::S) && captured(ctrl, Key::O) && captured(ctrl, Key::D));
        assert!(captured(Modifiers::NONE, Key::F9) && captured(Modifiers::ALT, Key::W));
        assert!(!captured(ctrl, Key::C) && !captured(ctrl, Key::V) && !captured(ctrl, Key::X), "clipboard keys reach the browser");
        assert!(!captured(Modifiers::NONE, Key::M) && !captured(Modifiers::NONE, Key::F), "plain keys need no capture");
    }

    #[test]
    fn gesture_rows_name_the_hold_keys_and_cover_every_gesture() {
        let rows = fixed_gestures();
        let has = |gesture: &str, what: &str| rows.iter().any(|(g, w)| g == gesture && w.contains(what));
        let (s, v) = (crate::viewport::STENCIL_KEY.name(), crate::viewport::CLONE_SOURCE_KEY.name());
        assert!(has(&format!("{s} + LMB drag"), "rotate") && has(&format!("{s} + RMB drag"), "scale") && has(&format!("{s} + MMB drag"), "move"));
        assert!(has(&format!("{v} + LMB"), "Clone"));
        for (gesture, what) in [
            ("Shift + LMB", "straight line"),
            ("Shift + RMB drag", "environment light"),
            ("Enter", "apply"),
            ("Backspace", "remove"),
            ("Esc (Path tool)", "cancel"),
            ("Click the first point (Path tool)", "close"),
            ("LMB drag on a point (Path tool)", "move"),
            ("Click (Path tool)", "add a point"),
        ] {
            assert!(has(gesture, what), "no row {gesture} / {what}");
        }
        let mut seen = std::collections::HashSet::new();
        assert!(rows.iter().all(|(g, w)| seen.insert((g.clone(), w.clone()))), "duplicate row");
    }

    #[test]
    fn shortcuts_round_trip_through_parse_and_format() {
        for b in bindings() {
            if let Some(s) = b.default {
                let chord = parse(s).unwrap_or_else(|| panic!("{} has an unparsable shortcut {s}", b.id));
                assert_eq!(format_shortcut(&chord.to_egui_shortcut()), s);
            }
        }
        for s in ["Ctrl+Shift+Alt+F12", "Alt+OpenBracket", "Shift+Delete", "Ctrl+9"] {
            assert_eq!(parse(s).map(|chord| format_shortcut(&chord.to_egui_shortcut())).as_deref(), Some(s));
        }
        assert!(parse("Ctrl+").is_none() && parse("Ctrl+NoSuchKey").is_none());
    }

    fn bindings_for(keymap: &Keymap) -> Bindings {
        let mut out = Bindings::default();
        out.sync(keymap, Platform::Windows);
        out
    }

    fn binding(id: &str) -> &'static Binding {
        bindings().iter().find(|b| b.id == id).unwrap()
    }

    #[test]
    fn default_shortcuts_do_not_conflict() {
        let resolved = bindings_for(&Keymap::new());
        for b in bindings() {
            assert!(resolved.conflict(b).is_none(), "{} shares its default shortcut", b.id);
        }
    }

    #[test]
    fn rebinding_detects_conflicts_and_resets() {
        let (save, undo) = (binding("file.save"), binding("edit.undo"));
        let mut keymap = Keymap::new();
        rebind(&mut keymap, save, "Ctrl+Z");
        let resolved = bindings_for(&keymap);
        assert_eq!(resolved.conflict(save).map(|b| b.id), Some("edit.undo"));
        assert_eq!(resolved.conflict(undo).map(|b| b.id), Some("file.save"));
        assert_eq!(shortcut_of(&keymap, "file.save"), Some("Ctrl+Z"));
        // Unbinding clears the conflict; binding back to the default drops the override.
        rebind(&mut keymap, save, "");
        assert_eq!(shortcut_of(&keymap, "file.save"), None);
        assert!(bindings_for(&keymap).conflict(undo).is_none());
        rebind(&mut keymap, save, "Ctrl+S");
        assert!(keymap.is_empty());
    }

    fn press(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn press_at(key: Key, physical: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: Some(physical), pressed: true, repeat: false, modifiers }
    }

    fn dispatched(bindings: &mut Bindings, events: Vec<Event>, modifiers: Modifiers) -> Vec<String> {
        let mut input = egui::InputState::default();
        input.modifiers = modifiers;
        input.events = events;
        bindings.dispatch(&mut input)
    }

    #[test]
    fn a_stored_shortcut_on_a_shifted_symbol_runs_on_that_press() {
        for (stored, key, physical, modifiers) in [
            ("Shift+Colon", Key::Colon, Key::Semicolon, Modifiers::SHIFT),
            ("Shift+Exclamationmark", Key::Exclamationmark, Key::Num1, Modifiers::SHIFT),
            ("Shift+Questionmark", Key::Questionmark, Key::Slash, Modifiers::SHIFT),
            ("Ctrl+Shift+Plus", Key::Plus, Key::Equals, Modifiers::COMMAND | Modifiers::SHIFT),
        ] {
            let mut keymap = Keymap::new();
            keymap.insert("file.save".into(), stored.into());
            let mut resolved = bindings_for(&keymap);
            assert_eq!(dispatched(&mut resolved, vec![press_at(key, physical, modifiers)], modifiers), ["file.save"], "{stored}");
            assert_eq!(resolved.command_of(&press_at(key, physical, modifiers)).as_deref(), Some("file.save"), "{stored} in a floating panel");
        }
    }

    #[test]
    fn a_recorded_shifted_symbol_is_the_chord_it_runs() {
        let resolved = bindings_for(&Keymap::new());
        let recorded = |key, physical, modifiers| format_shortcut(&resolved.recorded(&press_at(key, physical, modifiers), key, modifiers, Platform::Windows));
        assert_eq!(recorded(Key::Colon, Key::Semicolon, Modifiers::SHIFT), "Colon");
        // The layout needs Shift for this slash; the key at that position is not a symbol.
        assert_eq!(recorded(Key::Slash, Key::Num7, Modifiers::SHIFT), "Slash");
        assert_eq!(recorded(Key::E, Key::E, Modifiers::COMMAND | Modifiers::SHIFT), "Ctrl+Shift+E");
    }

    #[test]
    fn a_floating_panel_matches_presses_as_the_main_window_does() {
        let mut keymap = Keymap::new();
        keymap.insert("file.save".into(), "Equals".into());
        let resolved = bindings_for(&keymap);
        assert_eq!(resolved.command_of(&press(Key::Plus, Modifiers::NONE)).as_deref(), Some("file.save"), "+ stands for =");
    }

    #[test]
    fn a_floating_panel_leaves_a_bound_shift_insert_to_its_binding() {
        let paste = || Event::Paste("text".into());
        let free = bindings_for(&Keymap::new());
        let event = crate::menus::forwarded_press(&free, paste(), Modifiers::SHIFT);
        assert_eq!(free.command_of(&event).as_deref(), Some("layer.paste"));
        let mut keymap = Keymap::new();
        keymap.insert("file.save".into(), "Shift+Insert".into());
        keymap.insert("file.saveAs".into(), "Shift+Delete".into());
        let bound = bindings_for(&keymap);
        let event = crate::menus::forwarded_press(&bound, paste(), Modifiers::SHIFT);
        assert_eq!(bound.command_of(&event).as_deref(), Some("file.save"));
        let event = crate::menus::forwarded_press(&bound, Event::Cut, Modifiers::SHIFT);
        assert_eq!(bound.command_of(&event).as_deref(), Some("file.saveAs"));
    }

    #[test]
    fn a_press_runs_the_command_bound_to_exactly_that_chord() {
        let resolved = bindings_for(&Keymap::new());
        let run = |mods: Modifiers, key: Key| resolved.command_of(&press(key, mods));
        assert_eq!(run(Modifiers::COMMAND, Key::S).as_deref(), Some("file.save"));
        assert_eq!(run(Modifiers::COMMAND | Modifiers::SHIFT, Key::S).as_deref(), Some("file.saveAs"));
        assert_eq!(run(Modifiers::SHIFT, Key::C).as_deref(), Some("viewport.previousChannel"));
        assert_eq!(run(Modifiers::NONE, Key::C).as_deref(), Some("viewport.nextChannel"));
        assert_eq!(run(Modifiers::ALT, Key::C), None, "Alt+C is not bound");
        assert_eq!(run(Modifiers::SHIFT, Key::M), None, "Shift+M is not bound");
    }

    #[test]
    fn a_stored_shortcut_replaces_the_default_and_an_empty_one_unbinds() {
        let mut keymap = Keymap::new();

        // F7 is bound to nothing else, so the stored shortcut alone decides (F5 is Perspective)
        keymap.insert("file.save".into(), "F7".into());
        keymap.insert("edit.undo".into(), String::new());
        let resolved = bindings_for(&keymap);
        assert_eq!(resolved.command_of(&press(Key::F7, Modifiers::NONE)).as_deref(), Some("file.save"));
        assert_eq!(resolved.command_of(&press(Key::S, Modifiers::COMMAND)), None);
        assert_eq!(resolved.command_of(&press(Key::Z, Modifiers::COMMAND)), None);
    }

    #[test]
    fn ctrl_in_the_notation_is_the_command_key_on_macos() {
        let mut resolved = Bindings::default();
        resolved.sync(&Keymap::new(), Platform::Mac);
        let cmd = Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE };
        assert_eq!(resolved.command_of(&press(Key::S, cmd)).as_deref(), Some("file.save"));
        assert_eq!(resolved.command_of(&press(Key::S, Modifiers::CTRL)), None, "the Control key is another key");
    }

    #[test]
    fn commands_without_a_default_key_are_listed_unbound_and_filtered_by_their_parameters() {
        for id in ["layer.add.fill", "layer.add.paint", "layer.add.group", "mask.invert", "layer.visible"] {
            let b = binding(id);
            assert!(b.default.is_none() && shortcut(&Keymap::new(), b).is_none(), "{id} is listed without a key");
        }
        assert_eq!(binding("mask.invert").group, "Layers");
        for id in ["textureSet.select", "layer.rename", "tool.select", "layer.list", "project.inspect"] {
            assert!(bindings().iter().all(|b| b.id != id), "{id} cannot run from a key");
        }
        let ids: Vec<_> = bindings().iter().map(|b| b.id).collect();
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "every id is listed once");
    }

    #[test]
    fn every_listed_item_is_implemented() {
        for b in bindings() {
            assert!(crate::menus::implemented(b.id), "{} is listed but nothing runs it", b.id);
        }
    }

    #[test]
    fn a_command_without_a_default_key_can_be_bound_unbound_and_conflicts() {
        let (invert, save) = (binding("mask.invert"), binding("file.save"));
        let mut keymap = Keymap::new();
        rebind(&mut keymap, invert, "Ctrl+Alt+I");
        assert_eq!(shortcut_of(&keymap, "mask.invert"), Some("Ctrl+Alt+I"));
        let resolved = bindings_for(&keymap);
        assert_eq!(resolved.command_of(&press(Key::I, Modifiers::COMMAND | Modifiers::ALT)).as_deref(), Some("mask.invert"));
        assert!(resolved.conflict(invert).is_none());

        // Taking the key of a default shortcut is a conflict, on both items
        rebind(&mut keymap, invert, "Ctrl+S");
        let resolved = bindings_for(&keymap);
        assert_eq!(resolved.conflict(invert).map(|b| b.id), Some("file.save"));
        assert_eq!(resolved.conflict(save).map(|b| b.id), Some("mask.invert"));

        // Unbinding drops the override of an item that has no default
        rebind(&mut keymap, invert, "");
        assert_eq!(shortcut_of(&keymap, "mask.invert"), None);
        assert!(bindings_for(&keymap).conflict(save).is_none());
    }

    #[test]
    fn a_saved_keymap_from_before_every_command_was_bindable_still_loads() {
        let old: Keymap = serde_json::from_str(r#"{"file.save":"F5","edit.undo":"","ui.layout.3d":"Ctrl+Shift+H"}"#).unwrap();
        let resolved = bindings_for(&old);
        assert_eq!(resolved.command_of(&press(Key::F5, Modifiers::NONE)).as_deref(), Some("file.save"));
        assert_eq!(resolved.command_of(&press(Key::Z, Modifiers::COMMAND)), None);
        assert_eq!(shortcut_of(&old, "ui.layout.3d"), Some("Ctrl+Shift+H"));
        assert_eq!(shortcut_of(&old, "mask.invert"), None);
    }

    #[test]
    fn the_first_item_listed_wins_a_shared_shortcut() {
        let mut keymap = Keymap::new();
        keymap.insert("file.save".into(), "Ctrl+Z".into());
        let resolved = bindings_for(&keymap);
        let first = bindings().iter().find(|b| matches!(b.id, "file.save" | "edit.undo")).unwrap().id;
        assert_eq!(resolved.command_of(&press(Key::Z, Modifiers::COMMAND)).as_deref(), Some(first));
    }
}
