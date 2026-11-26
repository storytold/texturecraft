//! The strip at the top of the window: the app icon at the very left, the menus and the mode label,
//! and the window's title bar unless the system draws one. As the title bar it is craft-ui's
//! `shell::TitleBar`: its free area moves the window and follows the system's double-click
//! setting, and on a window without an OS frame it has the caption buttons of the desktop's
//! layout; the window keeps invisible resize zones along its edges where the runtime does not
//! answer the window's hit test itself (everywhere but Windows).
//!
//! Who draws the frame is decided once at startup ([`TitleBar::decide`]): Windows draws no OS
//! frame, macOS keeps its traffic lights over the strip (a full-size content view with a hidden
//! title), Linux and the BSDs follow craft-window's probe (the app draws the frame where the
//! compositor leaves it to the app, as GNOME does). `TEXTURECRAFT_TITLEBAR=app` or `=system`
//! overrides that, and Settings › Interface › System title bar brings the OS frame back on the
//! next start. The web build has no window frame: the strip is the menu bar.

use craft_ui::contract::ShowContainer as _;
use craft_ui::runtime::windows::WindowHost;
use craft_window::policy::{Decision, TitleBarPref};
use craft_window::{Frame, Layout};
use egui::emath::GuiRounding as _;
use egui::{Color32, CursorIcon, Id, LayerId, Order, PointerButton, Rect, ResizeDirection, Sense, Ui, ViewportCommand, pos2, vec2};

use crate::TextureCraftApp;
use crate::theme::Tokens;

/// Who draws the window's title bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TitleBar {
    /// The system (or the browser) draws it: the strip is the menu bar.
    #[default]
    System,
    /// macOS: the traffic lights sit over the strip, which keeps room for them at its left.
    Integrated,
    /// No OS frame: the strip is the title bar, with the window buttons in this layout.
    App(Layout),
}

/// The app icon's side, in points.
const ICON: f32 = 16.0;
/// The space between the icon and the menus.
const PAD: f32 = 8.0;
/// Resize zone thickness along an edge, and the side of a corner zone.
const EDGE: f32 = 5.0;
const CORNER: f32 = 12.0;

/// The app icon's PNGs (`assets/app-icon/hicolor`), smallest first: the strip draws the one that
/// fills [`ICON`] points at the screen's scale without being stretched.
const ICONS: [(f32, &[u8]); 5] = [
    (16.0, include_bytes!("../../../assets/app-icon/hicolor/16x16/apps/ai.storyteller.texturecraft.png")),
    (24.0, include_bytes!("../../../assets/app-icon/hicolor/24x24/apps/ai.storyteller.texturecraft.png")),
    (32.0, include_bytes!("../../../assets/app-icon/hicolor/32x32/apps/ai.storyteller.texturecraft.png")),
    (48.0, include_bytes!("../../../assets/app-icon/hicolor/48x48/apps/ai.storyteller.texturecraft.png")),
    (64.0, include_bytes!("../../../assets/app-icon/hicolor/64x64/apps/ai.storyteller.texturecraft.png")),
];

/// The override variable's prefix: `TEXTURECRAFT_TITLEBAR`.
const PREFIX: craft_window::Prefix = craft_window::Prefix::new("TEXTURECRAFT");

/// The Nightly strip's icon PNGs, same sizes: the deeper background tells the two windows apart at
/// a glance and matches the packaged Nightly icon.
const NIGHTLY_ICONS: [(f32, &[u8]); 5] = [
    (16.0, include_bytes!("../../../assets/app-icon/nightly/16.png")),
    (24.0, include_bytes!("../../../assets/app-icon/nightly/24.png")),
    (32.0, include_bytes!("../../../assets/app-icon/nightly/32.png")),
    (48.0, include_bytes!("../../../assets/app-icon/nightly/48.png")),
    (64.0, include_bytes!("../../../assets/app-icon/nightly/64.png")),
];

impl TitleBar {
    /// Decide who draws the title bar; call once at startup, before the window is created, and
    /// hand the decision to the window ([`viewport`]) and the runtime (`framework::install`).
    /// `system` is Settings › Interface › System title bar.
    pub fn decide(system: bool) -> Decision {
        let choice = craft_window::Choice::probe(PREFIX);
        let wanted = Self::from_choice(choice.frame(), choice.overridden(), system);
        let pref = if wanted == TitleBar::System { TitleBarPref::System } else { TitleBarPref::Integrated };
        craft_window::policy::decide_for(PREFIX, craft_host::target::Backend::from_env(), pref)
    }

    /// The bar a decided `frame` gives.
    pub fn of(frame: &craft_window::policy::Frame) -> TitleBar {
        use craft_window::policy::Frame as Decided;
        match frame {
            Decided::Native => TitleBar::System,
            Decided::Inset(_) => TitleBar::Integrated,
            Decided::Integrated(captions) => TitleBar::App(captions.layout.clone()),
        }
    }

    /// `TEXTURECRAFT_TITLEBAR` (`overridden`) wins, then the System title bar setting. Otherwise
    /// Windows draws its own caption, macOS integrates the traffic lights and the other systems
    /// take craft-window's `frame`.
    fn from_choice(frame: &Frame, overridden: Option<bool>, system: bool) -> TitleBar {
        if overridden.is_none() && system {
            return TitleBar::System;
        }
        match frame {
            Frame::App(layout) => TitleBar::App(layout.clone()),
            Frame::Desktop if overridden.is_some() => TitleBar::System,
            Frame::Desktop if cfg!(windows) => TitleBar::App(Layout::default()),
            Frame::Desktop if cfg!(target_os = "macos") => TitleBar::Integrated,
            Frame::Desktop => TitleBar::System,
        }
    }

    /// The name `ui.inspect` reports: `app`, `integrated` or `system`.
    pub fn name(&self) -> &'static str {
        match self {
            TitleBar::System => "system",
            TitleBar::Integrated => "integrated",
            TitleBar::App(_) => "app",
        }
    }
}

/// `builder` with the decided frame: decorations off for the app's own, and on macOS the content
/// under a transparent title bar without its title.
pub fn viewport(decision: &Decision, builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    craft_window::policy::apply(builder, &decision.frame)
}

/// The strip's height: the body text with room around it, 28 to 32 points (so the macOS traffic
/// lights and Windows-sized caption buttons fit it).
pub fn height(ui: &Ui) -> f32 {
    (ui.text_style_height(&egui::TextStyle::Body) + 14.0).clamp(28.0, 32.0)
}

/// The strip: the app icon, then the menus. Where the system draws the frame it is the menu bar;
/// otherwise it is the window's title bar (craft-ui's), with the caption buttons where the frame
/// has them and resize zones along the window's edges.
pub fn show(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    let frame = WindowHost::of(ui.ctx()).frame();
    if matches!(frame, craft_window::policy::Frame::Native) {
        egui::Panel::top("menu_bar").exact_size(height(ui)).show_separator_line(false).frame(egui::Frame::NONE.fill(tk.chrome)).show(ui, |ui| {
            let rect = ui.max_rect();
            let content = Rect::from_min_max(pos2(rect.left() + PAD, rect.top()), pos2(rect.right() - PAD, rect.bottom()));
            ui.scope_builder(egui::UiBuilder::new().max_rect(content).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                paint_icon(ui);
                crate::menus::menu_bar(app, ui);
            });
        });
        return;
    }
    egui::Panel::top("menu_bar").show_separator_line(false).frame(egui::Frame::NONE.fill(tk.chrome)).show(ui, |ui| {
        let bar = craft_ui::shell::TitleBar::new()
            .show(ui, |ui, _| {
                paint_icon(ui);
                crate::menus::menu_bar(app, ui);
            })
            .output;
        if bar.close_requested {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
    });
    // Windows' frame answers the resize edges itself (the runtime's hit test).
    if matches!(frame, craft_window::policy::Frame::Integrated(_)) && !cfg!(windows) {
        resize_zones(ui);
    }
}

/// The app icon at the cursor, [`ICON`] points square, centred on the row and on whole pixels.
fn paint_icon(ui: &mut Ui) {
    let ppp = ui.ctx().pixels_per_point();
    let icons = if crate::os_window::stable_services_allowed() { &ICONS } else { &NIGHTLY_ICONS };
    let Some(&(px, png)) = icons.iter().find(|(px, _)| *px >= ICON * ppp - 0.5).or(icons.last()) else { return };
    let id = Id::unique(("app_icon", px as u32));
    let texture = match ui.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)) {
        Some(t) => t,
        None => {
            // Decoded once per size; a PNG that doesn't decode leaves the space empty.
            let t = craft_image_codecs::decode(png).ok().map(|img| {
                let size = [img.width() as usize, img.height() as usize];
                let rgba = img.to_rgba8();
                ui.ctx().load_texture("app_icon", egui::ColorImage::from_rgba_unmultiplied(size, &rgba), egui::TextureOptions::LINEAR)
            });
            ui.data_mut(|d| d.insert_temp(id, t.clone()));
            t
        }
    };
    let row = ui.max_rect();
    let min = pos2(ui.cursor().min.x, row.center().y - ICON / 2.0).round_to_pixels(ppp);
    if let Some(t) = texture {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(t.id(), Rect::from_min_size(min, vec2(ICON, ICON)), uv, Color32::WHITE);
    }
    // The menu titles have no padding of their own: the gap the strip keeps at its edges.
    ui.add_space(ICON + PAD);
}

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

fn zones(w: Rect) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    use ResizeDirection::*;
    let (e, c) = (EDGE, CORNER);
    [
        (Rect::from_min_max(w.left_top(), pos2(w.right(), w.top() + e)), North, CursorIcon::ResizeVertical),
        (Rect::from_min_max(pos2(w.left(), w.bottom() - e), w.right_bottom()), South, CursorIcon::ResizeVertical),
        (Rect::from_min_max(w.left_top(), pos2(w.left() + e, w.bottom())), West, CursorIcon::ResizeHorizontal),
        (Rect::from_min_max(pos2(w.right() - e, w.top()), w.right_bottom()), East, CursorIcon::ResizeHorizontal),
        (Rect::from_min_size(w.left_top(), vec2(c, c)), NorthWest, CursorIcon::ResizeNwSe),
        (Rect::from_min_size(w.right_top() - vec2(c, 0.0), vec2(c, c)), NorthEast, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.left_bottom() - vec2(0.0, c), vec2(c, c)), SouthWest, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.right_bottom() - vec2(c, c), vec2(c, c)), SouthEast, CursorIcon::ResizeNwSe),
    ]
}

/// Invisible zones along the window's edges, above everything else; a press in one starts the
/// system's resize in that direction. None while the window is maximized or full screen.
fn resize_zones(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if maximized(&ctx) || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let w = ctx.content_rect();
    let zui = ui.new_child(egui::UiBuilder::new().layer_id(LayerId::new(Order::Foreground, Id::unique("titlebar_resize"))).max_rect(w));
    for (i, (r, dir, cursor)) in zones(w).into_iter().enumerate() {
        // Drag only (not focusable): Tab does not stop on the zones.
        let resp = zui.interact(r, Id::unique(("titlebar_resize", i)), Sense::DRAG).on_hover_cursor(cursor);
        if resp.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            // The window manager runs the resize and, on X11 and Wayland, keeps the button
            // release: end egui's drag now, or the next press would start nothing.
            ctx.stop_dragging();
            ctx.input_mut(|i| i.pointer = Default::default());
        }
    }
}

/// Once per frame after the UI, on macOS with the integrated strip: the traffic lights centred on
/// the strip, inset from its left as far as from its top, and AppKit's `movable` flag following
/// what is under the pointer (`movable` keeps the value last set).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_override_wins_then_the_setting_then_the_platform() {
        let gnome = Frame::App(Layout::parse("appmenu:close"));
        // TEXTURECRAFT_TITLEBAR=app / =system, whatever the setting.
        for system in [false, true] {
            assert_eq!(TitleBar::from_choice(&gnome, Some(true), system), TitleBar::App(Layout::parse("appmenu:close")));
            assert_eq!(TitleBar::from_choice(&Frame::Desktop, Some(false), system), TitleBar::System);
        }
        // The setting brings the system's frame back.
        assert_eq!(TitleBar::from_choice(&gnome, None, true), TitleBar::System);
        assert_eq!(TitleBar::from_choice(&Frame::Desktop, None, true), TitleBar::System);
        // The platform: a compositor that leaves the frame to the app gets the app's.
        assert_eq!(TitleBar::from_choice(&gnome, None, false), TitleBar::App(Layout::parse("appmenu:close")));
        let desktop = TitleBar::from_choice(&Frame::Desktop, None, false);
        let expected = if cfg!(windows) {
            TitleBar::App(Layout::default())
        } else if cfg!(target_os = "macos") {
            TitleBar::Integrated
        } else {
            TitleBar::System
        };
        assert_eq!(desktop, expected);
    }

    #[test]
    fn resize_corners_win_over_edges() {
        let w = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0));
        let at = |p: egui::Pos2| zones(w).into_iter().rev().find(|(r, ..)| r.contains(p)).map(|(_, d, _)| d);
        assert_eq!(at(pos2(2.0, 2.0)), Some(ResizeDirection::NorthWest));
        assert_eq!(at(pos2(500.0, 1.0)), Some(ResizeDirection::North));
        assert_eq!(at(pos2(998.0, 300.0)), Some(ResizeDirection::East));
        assert_eq!(at(pos2(500.0, 300.0)), None);
    }
}
