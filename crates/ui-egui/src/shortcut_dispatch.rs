//! The frame's key presses and clipboard events as the shortcuts they run: egui's events fed to
//! `craft_keymap::Matcher`, one press at a time in press order.
//!
//! - A press that runs a command is removed from egui's input, with the text it typed; one that
//!   runs nothing, or whose hit the caller declines, stays.
//! - Held keys run their command again on each repeat (a held arrow nudges); repeats never count
//!   toward a double press.
//! - egui's Copy, Cut and Paste events count as their plain chords (Cmd+C, Cmd+X, Cmd+V;
//!   Shift+Delete and Shift+Insert), so the desktop's clipboard events run the edit commands. The
//!   web build sends a clipboard press and the clipboard event it causes: the press decides, and
//!   its event goes with it when it runs a command and stays otherwise.
//! - The keyboard layout is learned from what plain presses type, and taken from egui when the
//!   integration reports whether it is Latin.

use craft_keymap::{Active, Hit, Key, KeyInput, Keymap, Layout, Matcher, Mods, Platform, Press};
use egui::{Event, InputState, Key as EguiKey};

/// eframe's web build sends a press's Text event first when no text field has focus.
const TEXT_FIRST: bool = cfg!(target_arch = "wasm32");

/// What matching carries between frames: the previous press (double presses) and the layout.
#[derive(Default)]
pub(crate) struct Dispatcher {
    matcher: Matcher,
    layout: Layout,
    /// Whether right Alt is held. egui-winit reports AltGr with neither Ctrl nor Alt, so its
    /// presses would otherwise teach the layout as plain ones.
    right_alt: bool,
}

impl Dispatcher {
    /// The keyboard layout learned so far: what a press stands for as a chord.
    pub(crate) fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The commands this frame's key presses and clipboard events run, in press order, with
    /// `accept` deciding each hit before its event is removed. A declined hit is left out and its
    /// event, with any text it typed, stays in egui's input in its place.
    pub(crate) fn dispatch_filtered(&mut self, input: &mut InputState, keymap: &Keymap, mut accept: impl FnMut(&Hit) -> bool) -> Vec<Hit> {
        let platform = keymap.platform();
        let now = input.time;
        let frame_mods = Mods::from_egui(&input.modifiers, platform);
        let events = std::mem::take(&mut input.events);
        let text_of = text_of_press(&events, TEXT_FIRST);
        if let Some(latin) = input.keyboard_layout_is_latin() {
            self.layout.set_latin(Some(latin));
        }
        self.learn(&events, &text_of, platform);
        let causes = clipboard_event_follows(&events, &self.layout, platform, input);
        let mut caused_taken: Vec<Option<bool>> = vec![None; events.len()];
        let mut drop = vec![false; events.len()];
        let mut hits = Vec::new();
        let mut kept = Vec::with_capacity(events.len());
        for (i, event) in events.into_iter().enumerate() {
            match &event {
                Event::Key { pressed: true, repeat, .. } => {
                    let key = key_input(&event, platform, input);
                    if self.layout.resolve(&key).is_some() {
                        let press = Press { input: &key, repeat: *repeat, now, active: &Active::NONE, outer: &[] };
                        let hit = self.matcher.press(&self.layout, keymap, &press).filter(&mut accept);
                        if let Some((j, _)) = causes[i] {
                            caused_taken[j] = Some(hit.is_some());
                        }
                        if let Some(hit) = hit {
                            hits.push(hit);
                            if let Some(j) = text_of[i].filter(|_| !(key.mods.cmd || key.mods.ctrl)) {
                                drop[j] = true;
                            }
                            continue;
                        }
                    }
                }
                // Desktop clipboard events, and web ones whose press ran nothing.
                Event::Copy | Event::Cut | Event::Paste(_) => match caused_taken[i] {
                    Some(true) => continue,
                    Some(false) => {}
                    None => {
                        let (key, held) = clipboard_chord(&event, frame_mods);
                        let press = Press { input: &KeyInput::new(held).with_typed(key), repeat: false, now, active: &Active::NONE, outer: &[] };
                        if let Some(hit) = self.matcher.press(&self.layout, keymap, &press).filter(&mut accept) {
                            hits.push(hit);
                            continue;
                        }
                    }
                },
                _ => {}
            }
            kept.push((i, event));
        }
        input.events = kept.into_iter().filter(|(i, _)| !drop[*i]).map(|(_, e)| e).collect();
        hits
    }

    /// Teaches the layout what this frame's presses typed.
    fn learn(&mut self, events: &[Event], text_of: &[Option<usize>], platform: Platform) {
        for (event, text) in events.iter().zip(text_of) {
            match event {
                Event::Key { physical_key: Some(EguiKey::AltRight), pressed, .. } => self.right_alt = *pressed,
                Event::WindowFocused(false) => self.right_alt = false,
                _ => {}
            }
            if self.right_alt {
                continue;
            }
            let Some(Event::Text(text)) = text.and_then(|j| events.get(j)) else { continue };
            if let Some(KeyInput { mods, physical: Some(physical), .. }) = KeyInput::from_egui(event, platform) {
                self.layout.observe(physical, mods, text);
            }
        }
    }
}

/// A press as [`KeyInput::from_egui`] reads it, with the Latin layout's character `input` reports
/// for its key.
fn key_input(event: &Event, platform: Platform, input: &InputState) -> KeyInput {
    let mut key = KeyInput::from_egui(event, platform).unwrap_or(KeyInput::new(Mods::NONE));
    if let Event::Key { physical_key: Some(physical), .. } = event {
        key.latin = input.latin_layout_char(*physical);
    }
    key
}

/// A clipboard event's kind.
#[derive(Clone, Copy)]
enum Clip {
    Copy,
    Cut,
    Paste,
}

fn event_clip(event: &Event) -> Option<Clip> {
    match event {
        Event::Copy => Some(Clip::Copy),
        Event::Cut => Some(Clip::Cut),
        Event::Paste(_) => Some(Clip::Paste),
        _ => None,
    }
}

/// The clipboard event a backend sends for a press, if any.
fn clipboard_kind(key: Key, held: Mods) -> Option<Clip> {
    match key {
        Key::C if held.cmd => Some(Clip::Copy),
        Key::X if held.cmd => Some(Clip::Cut),
        Key::V if held.cmd => Some(Clip::Paste),
        Key::Delete if held.shift => Some(Clip::Cut),
        Key::Insert if held.shift => Some(Clip::Paste),
        Key::Insert if held.cmd || held.ctrl => Some(Clip::Copy),
        _ => None,
    }
}

/// For each clipboard key press whose clipboard event comes later in the frame, as the web build
/// sends them: the index of that event, and the text of a Paste event.
fn clipboard_event_follows(events: &[Event], layout: &Layout, platform: Platform, input: &InputState) -> Vec<Option<(usize, Option<String>)>> {
    let mut out = vec![None; events.len()];
    let mut ahead: [Option<usize>; 3] = [None; 3];
    for (i, event) in events.iter().enumerate().rev() {
        if let Some(seen) = event_clip(event).and_then(|c| ahead.get_mut(c as usize)) {
            *seen = Some(i);
        } else if let Event::Key { pressed: true, .. } = event {
            let chord = layout.resolve(&key_input(event, platform, input));
            let clip = chord.and_then(|c| clipboard_kind(c.key, c.mods));
            if let Some(j) = clip.and_then(|c| ahead.get_mut(c as usize)).and_then(Option::take) {
                let text = if let Some(Event::Paste(text)) = events.get(j) { Some(text.clone()) } else { None };
                out[i] = Some((j, text));
            }
        }
    }
    out
}

/// For each key press, the index of the Text event it typed. egui-winit, and eframe's web text
/// agent while a text field has focus, send the Text after the press, before the next one.
/// With `text_first`, eframe's web build with no text field focused, the Text comes right before
/// its press, and only for a press without Ctrl or Cmd.
fn text_of_press(events: &[Event], text_first: bool) -> Vec<Option<usize>> {
    let is_text = |e: Option<&Event>| matches!(e, Some(Event::Text(t)) if !t.is_empty());
    let mut out = vec![None; events.len()];
    if text_first {
        for (i, event) in events.iter().enumerate().skip(1) {
            if let Event::Key { pressed: true, modifiers, .. } = event
                && !(modifiers.ctrl || modifiers.command || modifiers.mac_cmd)
                && is_text(events.get(i - 1))
            {
                out[i] = Some(i - 1);
            }
        }
        return out;
    }
    let mut text_ahead = None;
    for (i, event) in events.iter().enumerate().rev() {
        match event {
            Event::Key { pressed: true, .. } => out[i] = text_ahead.take(),
            _ if is_text(Some(event)) => text_ahead = Some(i),
            _ => {}
        }
    }
    out
}

/// The press a clipboard event stands for, with the modifiers held. Off macOS egui-winit also
/// sends Cut for Shift+Delete and Paste for Shift+Insert.
fn clipboard_chord(event: &Event, held: Mods) -> (Key, Mods) {
    let with_cmd = Mods { cmd: true, ..held };
    let shift_only = held.shift && !held.cmd;
    match event {
        Event::Cut if shift_only => (Key::Delete, held),
        Event::Cut => (Key::X, with_cmd),
        Event::Paste(_) if shift_only => (Key::Insert, held),
        Event::Paste(_) => (Key::V, with_cmd),
        _ => (Key::C, with_cmd),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_keymap::{Binding, CommandDef, Scope, Scopes};

    fn keymap() -> Keymap {
        let def = |id: &'static str, binding: &str| CommandDef::new(id, id, Scope::app()).with_default(Binding::parse(binding).unwrap());
        Keymap::new(vec![def("tool.marquee", "M"), def("edit.copy", "Ctrl+C")], Scopes::new(), Platform::Windows)
    }

    fn press(key: EguiKey, modifiers: egui::Modifiers) -> Event {
        Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
    }

    /// A press that runs a command leaves egui's input with the text it typed; the others stay, in order.
    #[test]
    fn a_bound_press_and_its_text_leave_the_input() {
        let mut input = InputState::default();
        input.events =
            vec![press(EguiKey::M, egui::Modifiers::NONE), Event::Text("m".into()), press(EguiKey::Q, egui::Modifiers::NONE), Event::Text("q".into())];
        let hits = Dispatcher::default().dispatch_filtered(&mut input, &keymap(), |_| true);
        assert_eq!(hits.iter().map(|hit| hit.command.id.as_str()).collect::<Vec<_>>(), ["tool.marquee"]);
        assert_eq!(input.events, [press(EguiKey::Q, egui::Modifiers::NONE), Event::Text("q".into())]);
    }

    /// The desktop's Copy event runs the copy command; declined, it stays for egui.
    #[test]
    fn a_clipboard_event_runs_its_chord_unless_declined() {
        let mut input = InputState::default();
        input.events = vec![Event::Copy];
        let hits = Dispatcher::default().dispatch_filtered(&mut input, &keymap(), |_| true);
        assert_eq!(hits.iter().map(|hit| hit.command.id.as_str()).collect::<Vec<_>>(), ["edit.copy"]);
        assert!(input.events.is_empty());

        input.events = vec![Event::Copy];
        assert!(Dispatcher::default().dispatch_filtered(&mut input, &keymap(), |_| false).is_empty());
        assert_eq!(input.events, [Event::Copy]);
    }
}
