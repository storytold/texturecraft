// Provenance for the headless CLI: the same commit and date variables as the app, and (Windows
// only) its own icon and VERSIONINFO into `texturecraft-cli.exe`.
//
// This is deliberately not an `include!` of the app's build script: that one resolves a `#[path]`
// submodule and reads the Nightly channel, neither of which a CLI build has any use for, and a
// module path inside an included script is resolved from the included file, not from here.

use std::process::Command;

const ICON: &str = "../../assets/app-icon/texturecraft.ico";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");
    for var in ["TEXTURECRAFT_BUILD_SHA", "TEXTURECRAFT_BUILD_DATE", "TEXTURECRAFT_REQUIRE_WINRES", "SOURCE_DATE_EPOCH"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rustc-env=TEXTURECRAFT_BUILD_SHA={}", env_or("TEXTURECRAFT_BUILD_SHA", git_sha));
    println!("cargo:rustc-env=TEXTURECRAFT_BUILD_DATE={}", env_or("TEXTURECRAFT_BUILD_DATE", today));

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

fn env_or(var: &str, fallback: fn() -> Option<String>) -> String {
    std::env::var(var).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).or_else(fallback).unwrap_or_default()
}

fn git_sha() -> Option<String> {
    let out = Command::new("git").args(["rev-parse", "HEAD"]).output().ok().filter(|o| o.status.success())?;
    let sha = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (sha.len() >= 7 && sha.bytes().all(|b| b.is_ascii_hexdigit())).then_some(sha)
}

/// UTC date of `SOURCE_DATE_EPOCH`, or of now.
fn today() -> Option<String> {
    let secs = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(v) => v.trim().parse::<u64>().ok()?,
        Err(_) => std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs(),
    };
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm, proleptic Gregorian).
    let z = i64::try_from(secs / 86_400).ok()? + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

fn windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ICON)
        .set("ProductName", "TextureCraft")
        .set("FileDescription", "TextureCraft command-line tool")
        .set("CompanyName", "Learning Machines LLC")
        .set("LegalCopyright", "Copyright (c) the TextureCraft contributors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "texturecraft-cli.exe")
        .set("InternalName", "texturecraft-cli");
    if let Err(e) = res.compile() {
        if std::env::var_os("TEXTURECRAFT_REQUIRE_WINRES").is_some() {
            println!("cargo::error=embedding Windows resources into texturecraft-cli.exe failed: {e}");
        } else {
            println!("cargo:warning=texturecraft-cli.exe built without icon/version resources: {e}");
        }
    }
}
