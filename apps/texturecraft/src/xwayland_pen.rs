//! A pen on a Wayland desktop opens the window through Xwayland (#816).
//!
//! Copyright (c) 2026 ArtCraft Team and the PhotoCraft contributors for the adapted code.
//! Licensed under the MIT License or the Apache License, Version 2.0, at your option.
//!
//! Wayland compositors give a pen only to clients that bind the tablet protocol (`zwp_tablet_v2`),
//! which winit 0.30 doesn't (and binding it on winit's connection needs `unsafe`, which this app
//! forbids). Some compositors (KWin) then send the window nothing from the pen: no cursor, no
//! painting. Xwayland binds the protocol and serves the pen as an X11 pointer, so with a pen
//! attached the window opens there. `TEXTURECRAFT_NATIVE_WAYLAND=1` keeps native Wayland.
//!
//! Adapted from PhotoCraft (storytold/photocraft `830316b7`, MIT OR Apache-2.0):
//! `apps/photocraft/src/tablet.rs` (`display_session`, `pen_needs_xwayland`, `has_pen`, `bit_set`)
//! and `apps/photocraft/src/linux_libs.rs` (`session_from_env`, `ldconfig_has`, the library
//! search), reduced to the pen decision. See the repository's NOTICE for the port.

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use x11rb_protocol::connect::Connect;
use x11rb_protocol::xauth::{Family, get_auth};

/// Set (and not `0`) to keep native Wayland with a pen attached
pub(super) const NATIVE_WAYLAND_VARIABLE: &str = "TEXTURECRAFT_NATIVE_WAYLAND";

/// `BTN_TOOL_PEN` (linux/input-event-codes.h): the key a tablet's stylus, a pen display's or a
/// pen-enabled touchscreen's pen reports
const BTN_TOOL_PEN: u32 = 0x140;

/// `ABS_X` and `ABS_Y` (linux/input-event-codes.h): the absolute axes every pen reports
const ABS_X: u32 = 0x00;
const ABS_Y: u32 = 0x01;

/// What winit 0.30 loads at run time for an X11 window: Xlib, Xlib-xcb, xcb, Xcursor, XInput2 and
/// xkbcommon with its X11 part
const X11_LIBRARIES: &[&str] =
    &["libX11.so.6", "libX11-xcb.so.1", "libxcb.so.1", "libXcursor.so.1", "libXi.so.6", "libxkbcommon.so.0", "libxkbcommon-x11.so.0"];

// Give local Xwayland time to answer while bounding the wait before falling back to Wayland
const X11_SETUP_TIMEOUT: Duration = Duration::from_secs(1);

/// Whether the window opens through Xwayland: a pen is attached on a Wayland desktop and Xwayland
/// has the required libraries and accepts an X11 client setup.
pub fn opens_through_xwayland() -> bool {
    let var = |name: &str| std::env::var(name).ok();
    let pen = || std::fs::read_to_string("/proc/bus/input/devices").is_ok_and(|devices| has_pen(&devices));
    pen_needs_xwayland(var, pen, |display| x11_libraries_available() && x_server_accepts_setup(display))
}

/// [`opens_through_xwayland`]'s decision. winit picks Wayland when `WAYLAND_DISPLAY` or
/// `WAYLAND_SOCKET` is set; Xwayland needs `DISPLAY`. The system probes run only on Wayland.
fn pen_needs_xwayland(var: impl Fn(&str) -> Option<String>, pen: impl FnOnce() -> bool, xwayland_ready: impl FnOnce(&str) -> bool) -> bool {
    let set = |name: &str| var(name).is_some_and(|value| !value.is_empty());
    let wayland = set("WAYLAND_DISPLAY") || set("WAYLAND_SOCKET");
    let keeps_wayland = var(NATIVE_WAYLAND_VARIABLE).is_some_and(|value| !value.is_empty() && value != "0");
    wayland && !keeps_wayland && var("DISPLAY").filter(|display| !display.is_empty()).is_some_and(|display| pen() && xwayland_ready(&display))
}

/// winit commits to its forced backend, so require an accepted X11 setup before forcing X11.
fn x_server_accepts_setup(display: &str) -> bool {
    let Some((display_number, screen)) = local_display(display) else {
        return false;
    };

    let Some(socket) = connect_local_x_server(display_number) else {
        return false;
    };

    let hostname = rustix::system::uname();
    let auth_hostname = std::env::var_os("XAUTHLOCALHOSTNAME");
    let auth_address = auth_hostname.as_ref().map_or(hostname.nodename().to_bytes(), |name| name.as_encoded_bytes());
    let (auth_name, auth_data) = get_auth(Family::LOCAL, auth_address, display_number).ok().flatten().unwrap_or_default();
    let (setup, request) = Connect::with_authorization(auth_name, auth_data);
    exchange_x11_setup(socket, setup, &request, screen)
}

/// Parse the display and screen of a local X server such as `:0`, `:0.0` or `unix:0`.
fn local_display(display: &str) -> Option<(u16, usize)> {
    let local = display.strip_prefix(':').or_else(|| display.strip_prefix("unix:"))?;
    let (display_number, screen) = local.split_once('.').unwrap_or((local, "0"));
    Some((display_number.parse().ok()?, screen.parse().ok()?))
}

fn connect_local_x_server(display_number: u16) -> Option<UnixStream> {
    use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType, connect, socket_with};

    let path = format!("/tmp/.X11-unix/X{display_number}");

    // Try the abstract socket first, as Xlib does on Linux
    [SocketAddrUnix::new_abstract_name(path.as_bytes()), SocketAddrUnix::new(&path)].into_iter().find_map(|address| {
        let address = address.ok()?;

        // A full Unix socket backlog must not block app startup
        let socket = UnixStream::from(socket_with(AddressFamily::UNIX, SocketType::STREAM, SocketFlags::NONBLOCK | SocketFlags::CLOEXEC, None).ok()?);
        connect(&socket, &address).ok()?;
        socket.set_nonblocking(false).ok()?;
        Some(socket)
    })
}

fn exchange_x11_setup(mut socket: UnixStream, mut setup: Connect, request: &[u8], screen: usize) -> bool {
    let start = Instant::now();
    if socket.set_write_timeout(Some(X11_SETUP_TIMEOUT)).is_err() || socket.write_all(request).is_err() {
        return false;
    }

    while !setup.buffer().is_empty() {
        let remaining = X11_SETUP_TIMEOUT.saturating_sub(start.elapsed());
        if remaining.is_zero() || socket.set_read_timeout(Some(remaining)).is_err() {
            return false;
        }

        let Ok(received) = socket.read(setup.buffer()) else {
            return false;
        };

        if received == 0 {
            return false;
        }

        if setup.advance(received) {
            break;
        }
    }

    setup.into_setup().is_ok_and(|reply| screen < reply.roots.len())
}

/// Is a pen among the input devices `/proc/bus/input/devices` lists? A pen's `B: KEY=` bitmap has
/// `BTN_TOOL_PEN` and its `B: ABS=` bitmap has `ABS_X` and `ABS_Y`, as libinput asks of a tablet
/// tool. The key alone isn't enough: a virtual keyboard made with uinput claims every key code.
fn has_pen(devices: &str) -> bool {
    devices.split("\n\n").any(|device| {
        let bitmap = |name: &str| device.lines().find_map(|line| line.strip_prefix(name));
        bitmap("B: KEY=").is_some_and(|keys| bit_set(keys, BTN_TOOL_PEN)) && bitmap("B: ABS=").is_some_and(|axes| bit_set(axes, ABS_X) && bit_set(axes, ABS_Y))
    })
}

/// Bit `bit` of a `/proc/bus/input/devices` bitmap: hex words of the kernel's `long`, most
/// significant first. Short or malformed bitmaps have no bits set.
fn bit_set(bitmap: &str, bit: u32) -> bool {
    let word_bits = usize::BITS;
    let word = usize::try_from(bit / word_bits).unwrap_or(usize::MAX);
    bitmap.split_whitespace().rev().nth(word).and_then(|hex| u64::from_str_radix(hex, 16).ok()).is_some_and(|value| (value >> (bit % word_bits)) & 1 == 1)
}

/// Are the libraries an X11 window needs installed? The linker cache (`ldconfig -p`) or the usual
/// library directories must list each; one that can't be found counts as missing.
fn x11_libraries_available() -> bool {
    let cache = linker_cache();
    let dirs = library_dirs(std::env::var("LD_LIBRARY_PATH").ok().as_deref());
    X11_LIBRARIES.iter().all(|soname| cache.as_deref().is_some_and(|cache| ldconfig_has(cache, soname)) || dirs.iter().any(|dir| dir.join(soname).exists()))
}

/// `ldconfig -p` in English (it is parsed), from where distributions put it
fn linker_cache() -> Option<String> {
    ["/sbin/ldconfig", "/usr/sbin/ldconfig", "ldconfig"].into_iter().find_map(|command| {
        let output = std::process::Command::new(command).arg("-p").env("LC_ALL", "C").output().ok()?;
        output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    })
}

/// Does an `ldconfig -p` listing have `soname` for this build's architecture? Lines look like
/// `\tlibXi.so.6 (libc6,x86-64) => /lib/x86_64-linux-gnu/libXi.so.6`.
fn ldconfig_has(cache: &str, soname: &str) -> bool {
    let architecture = if cfg!(target_arch = "x86_64") {
        Some("x86-64")
    } else if cfg!(target_arch = "aarch64") {
        Some("AArch64")
    } else {
        None
    };

    cache.lines().any(|line| {
        let Some((name, rest)) = line.trim_start().split_once(' ') else {
            return false;
        };

        let flags = rest.split_once('(').and_then(|(_, after)| after.split_once(')')).map_or("", |(flags, _)| flags);
        name == soname && architecture.is_none_or(|tag| flags.split(',').any(|flag| flag.trim() == tag))
    })
}

/// `LD_LIBRARY_PATH`, then the usual system and Flatpak library directories
fn library_dirs(ld_library_path: Option<&str>) -> Vec<PathBuf> {
    let triplet = if cfg!(target_arch = "aarch64") { "aarch64-linux-gnu" } else { "x86_64-linux-gnu" };
    let mut dirs: Vec<PathBuf> = ld_library_path.into_iter().flat_map(|value| value.split(':')).filter(|dir| !dir.is_empty()).map(PathBuf::from).collect();
    dirs.extend(["/app/lib", "/usr/local/lib", "/usr/lib64", "/lib64", "/usr/lib", "/lib"].map(PathBuf::from));
    dirs.extend([Path::new("/usr/lib").join(triplet), Path::new("/lib").join(triplet)]);
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    // Capabilities from /proc/bus/input/devices for a touchpad and a Wacom pen
    const TOUCHPAD: &str = "N: Name=\"SYNA2B33:00 06CB:CE26 Touchpad\"\nB: KEY=e520 10000 0 0 0 0\nB: ABS=2e0800000000003\n";
    const PEN: &str = "N: Name=\"Wacom Intuos Pro M Pen\"\nB: KEY=1c03 0 0 0 0 0\nB: ABS=1000f000003\n";

    // The uinput keyboard claims the pen key but reports no axes
    const VIRTUAL_KEYBOARD: &str = "N: Name=\"RustDesk UInput Keyboard\"\nB: EV=20013\nB: KEY=ffffffffff 0 7ffffff000301ff 1000f7fffffff 7fe001fffff000f 7ffffffffffffff ffffffff0003fdff 7fff8fff00ff03ff 1ffffffffffff07 ffffffffffffffff ffffffffffefffff fffffffffffffffe\n";

    #[test]
    fn a_pen_is_a_device_with_the_pen_key_and_absolute_axes() {
        assert!(has_pen(&format!("{TOUCHPAD}\n{PEN}")));
        assert!(!has_pen(TOUCHPAD), "a touchpad is no pen");
        assert!(!has_pen(&format!("{VIRTUAL_KEYBOARD}\n{TOUCHPAD}")), "a keyboard claiming the pen key has no pen axes");
        assert!(!has_pen("B: KEY=zz 0 0 0 0 0\nB: ABS=3\n"), "a malformed bitmap is no pen");
    }

    #[test]
    fn a_pen_on_wayland_opens_the_window_through_xwayland() {
        fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
            move |name: &str| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| value.to_string())
        }
        const WAYLAND: &[(&str, &str)] = &[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")];

        assert!(pen_needs_xwayland(env(WAYLAND), || true, |display| display == ":0"));
        assert!(!pen_needs_xwayland(env(WAYLAND), || false, |_| true), "no pen");
        assert!(!pen_needs_xwayland(env(WAYLAND), || true, |_| false), "Xwayland can't run the window");
        assert!(!pen_needs_xwayland(env(&[("WAYLAND_DISPLAY", "wayland-0")]), || true, |_| true), "no Xwayland");
        assert!(!pen_needs_xwayland(env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0"), (NATIVE_WAYLAND_VARIABLE, "1")]), || true, |_| true), "kept");
        assert!(pen_needs_xwayland(env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0"), (NATIVE_WAYLAND_VARIABLE, "0")]), || true, |_| true));
        assert!(!pen_needs_xwayland(env(&[("DISPLAY", ":0")]), || panic!("probed off Wayland"), |_| panic!("probed off Wayland")), "X11 already has the pen");
        assert_eq!([":1.2", "unix:0", "remote:0", ":0.bad"].map(local_display), [Some((1, 2)), Some((0, 0)), None, None]);
    }

    #[test]
    fn a_pen_keeps_wayland_when_the_x_server_rejects_setup() -> std::io::Result<()> {
        use x11rb_protocol::protocol::xproto::{Screen, Setup, SetupFailed};
        use x11rb_protocol::x11_utils::Serialize as _;

        const REPLY_HEADER_BYTES: usize = 8;
        const BYTES_PER_X11_WORD: usize = 4;
        let reason = b"Setup denied".to_vec();
        let rejected = SetupFailed { protocol_major_version: 11, length: (reason.len() / BYTES_PER_X11_WORD) as u16, reason, ..Default::default() };
        let mut accepted = Setup { status: 1, protocol_major_version: 11, roots: vec![Screen::default()], ..Default::default() };
        accepted.length = ((accepted.serialize().len() - REPLY_HEADER_BYTES) / BYTES_PER_X11_WORD) as u16;

        for (reply, screen, selects_xwayland) in [(rejected.serialize(), 0, false), (accepted.serialize(), 1, false), (accepted.serialize(), 0, true)] {
            let (client, mut server) = UnixStream::pair()?;
            server.write_all(&reply)?;
            let (setup, request) = Connect::with_authorization(Vec::new(), Vec::new());
            let var = |name: &str| match name {
                "WAYLAND_DISPLAY" => Some("wayland-0".into()),
                "DISPLAY" => Some(format!(":0.{screen}")),
                _ => None,
            };

            let xwayland = pen_needs_xwayland(
                var,
                || true,
                |display| local_display(display).is_some_and(|(_, screen)| exchange_x11_setup(client, setup, &request, screen)),
            );

            assert_eq!(xwayland, selects_xwayland);
        }

        Ok(())
    }
}
