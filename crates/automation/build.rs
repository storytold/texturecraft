//! Test executables of this crate on Windows (MSVC) reserve 8 MiB for each thread that does not
//! ask for a stack size of its own, as texturecraft-render's do: the tests render through
//! texturecraft-render on Mesa's lavapipe on the Windows CI gate, whose shader threads get the
//! executable's default of 1 MiB there and overflow it on the larger programs (Linux gives them
//! 8 MiB). This library crate links no other executable.

/// Bytes each thread reserves for its stack when it asks for none.
const THREAD_STACK_RESERVE: u32 = 8 * 1024 * 1024;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        println!("cargo:rustc-link-arg=/STACK:{THREAD_STACK_RESERVE}");
    }
}
