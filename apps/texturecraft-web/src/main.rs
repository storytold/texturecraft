//! TextureCraft in the browser.
//!
//! Runs the same `TextureCraftApp` as the desktop app through eframe's web
//! runner (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release`
//! from this directory (`packaging/web/package.sh` zips the result).
//!
//! Differences from the desktop app: no control server (browsers can't listen on sockets), and
//! file I/O goes through `texturecraft_ui_egui::web_files` (file picker in, downloads out).
//!
//! URL query flag: `?webgl` forces the WebGL2 backend instead of WebGPU.

#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("texturecraft-web only runs in the browser: build it with `trunk build --release` in apps/texturecraft-web");
}

#[cfg(test)]
mod tests {
    /// A WebAssembly module that never downloads runs no Rust, so the page itself replaces the
    /// endless "Loading" with a way to retry, without a hand-written script.
    #[test]
    fn loading_page_offers_a_reload_when_startup_stalls() {
        let html = include_str!("../index.html");
        let (_, hint) = html.split_once("<p id=\"texturecraft_stalled\">").expect("stalled-startup hint in the loading element");
        assert!(hint.contains("failed to") && hint.contains("<a href=\"\">Reload</a>"), "{hint}");
        assert!(html.contains("animation: texturecraft-stalled"), "the hint appears after a delay");
        assert!(html.contains("id=\"texturecraft_canvas\"") && html.contains("data-cargo-profile=\"wasm-release\""));
        assert!(!html.contains("<script"), "trunk injects the generated loader; the page has no script of its own");
    }
}
