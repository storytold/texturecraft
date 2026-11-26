//! Pen pressure on X11 (and Xwayland), which winit 0.30 drops: XInput2 raw events read over a
//! second, pure-Rust X connection (x11rb's `RustConnection`, no libxcb) and written into the
//! [`PenFeed`] that `pen_pressure` reads.
//!
//! Adapted from PhotoCraft's `photocraft-tablet` (`crates/tablet/src/x11.rs`, storytold/photocraft
//! at `b609bc7`, MIT OR Apache-2.0). Unlike it, events are keyed by `sourceid`, the physical device,
//! as storytold/photocraft#6871 found Xwayland needs.
//!
//! Raw events selected on the root window reach every client that asks, also while winit holds
//! a drag's pointer grab, but don't say which window the pen is over. That's fine for pressure:
//! it is only used while a view receives pointer events, and any other pointer moving resets it
//! to "mouse".
//!
//! Replace this module with the shared reader once craft-libs has one (storytold/texturecraft#153);
//! `PenFeed` stays. winit 0.31 doesn't make it redundant: its X11 backend still reports pens as a
//! mouse.

mod xi;

use texturecraft_ui_egui::pen_feed::{PenFeed, TabletReader};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xinput::{self, ConnectionExt as _, DeviceClassData, DeviceType, XIEventMask};
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use xi::{Device, State, Update, fp3232, valuators};

/// `XIAllDevices`: slaves included.
const ALL_DEVICES: u16 = 0;

/// The interface `ui.inspect` and About name while the reader runs.
const INTERFACE: &str = "XInput2";

/// Installs a pen feed in `ctx` and, unless the window is a native Wayland one (whose pen events
/// never reach an X client), starts reading tablet pressure on `$DISPLAY`. Without an X server or
/// XInput 2.1, a pen keeps working as a mouse and the feed says why.
pub fn start(ctx: &egui::Context, native_wayland: bool) {
    let feed = PenFeed::new(TabletReader::Reading(INTERFACE));
    feed.install(ctx);
    if native_wayland {
        feed.set_reader(TabletReader::Unavailable("native Wayland windows get no tablet input (run under X11 or Xwayland)".into()));
        return;
    }
    if let Err(reason) = spawn(feed.clone(), ctx.clone()) {
        feed.set_reader(TabletReader::Unavailable(reason));
    }
}

fn spawn(feed: PenFeed, ctx: egui::Context) -> Result<(), String> {
    let (conn, screen) = RustConnection::connect(None).map_err(|e| format!("cannot connect to the X server: {e}"))?;
    let root = conn.setup().roots.get(screen).map(|s| s.root).ok_or_else(|| format!("X screen {screen} not found"))?;
    // Raw events for every client (not just the grabbing one) need XI 2.1
    let version = conn.xinput_xi_query_version(2, 2).map_err(|e| e.to_string())?.reply().map_err(|e| format!("no XInput2: {e}"))?;
    if (version.major_version, version.minor_version) < (2, 1) {
        return Err(format!("XInput {}.{} (raw events need 2.1)", version.major_version, version.minor_version));
    }
    let mask = XIEventMask::RAW_MOTION | XIEventMask::RAW_BUTTON_PRESS | XIEventMask::RAW_BUTTON_RELEASE | XIEventMask::HIERARCHY | XIEventMask::DEVICE_CHANGED;
    conn.xinput_xi_select_events(root, &[xinput::EventMask { deviceid: ALL_DEVICES, mask: vec![mask] }])
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| format!("XISelectEvents: {e}"))?;
    let mut state = State::default();
    state.set_devices(query_devices(&conn));
    std::thread::Builder::new()
        .name("texturecraft-tablet".into())
        .spawn(move || {
            run(&conn, &mut state, &|pressure| {
                feed.set(pressure);
                // A pen pressing harder without moving sends no window event
                if pressure.is_some_and(|p| p > 0.0) {
                    ctx.request_repaint();
                }
            });
            feed.set(None);
            feed.set_reader(TabletReader::Unavailable("the X connection closed".into()));
        })
        .map(drop)
        .map_err(|e| format!("cannot start the tablet thread: {e}"))
}

/// Ends when the X connection closes.
fn run(conn: &RustConnection, state: &mut State, callback: &dyn Fn(Option<f32>)) {
    while let Ok(event) = conn.wait_for_event() {
        let update = match event {
            Event::XinputRawMotion(e) | Event::XinputRawButtonPress(e) | Event::XinputRawButtonRelease(e) => {
                let values: Vec<f64> = e.axisvalues.iter().map(|v| fp3232(v.integral, v.frac)).collect();
                state.handle(e.sourceid, valuators(&e.valuator_mask, &values))
            }
            Event::XinputHierarchy(_) | Event::XinputDeviceChanged(_) => {
                state.set_devices(query_devices(conn));
                Update::Keep
            }
            _ => Update::Keep,
        };
        if let Update::Set(pressure) = update {
            callback(pressure);
        }
    }
}

/// The slave pointer devices and their valuators (empty on any error).
fn query_devices(conn: &RustConnection) -> Vec<Device> {
    let Ok(reply) = conn.xinput_xi_query_device(ALL_DEVICES).map_err(drop).and_then(|c| c.reply().map_err(drop)) else { return Vec::new() };
    let mut out = Vec::new();
    for info in reply.infos.iter().take(xi::MAX_DEVICES) {
        if info.type_ != DeviceType::SLAVE_POINTER && info.type_ != DeviceType::FLOATING_SLAVE {
            continue;
        }
        let valuators = info.classes.iter().take(xi::MAX_AXES).filter_map(|class| match &class.data {
            DeviceClassData::Valuator(v) => Some((v.number, atom_name(conn, v.label)?, fp3232(v.min.integral, v.min.frac), fp3232(v.max.integral, v.max.frac))),
            _ => None,
        });
        out.push(Device::new(info.deviceid, valuators.collect::<Vec<_>>()));
    }
    out
}

fn atom_name(conn: &RustConnection, atom: u32) -> Option<String> {
    if atom == 0 {
        return None;
    }
    let reply = conn.get_atom_name(atom).ok()?.reply().ok()?;
    Some(String::from_utf8_lossy(&reply.name).into_owned())
}

/// Whether `window` is a native Wayland surface rather than an X11 (or Xwayland) one.
pub fn native_wayland(window: &winit::window::Window) -> bool {
    use winit::raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    window.window_handle().is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(_)))
}
