// Build provenance for every target, plus the Windows icon and VERSIONINFO.
//
// Shared by `texturecraft` and `texturecraft-cli` (whose build.rs `include!`s this file).
//
// - Sets `TEXTURECRAFT_VERSION` for `env!`: the prepared Nightly feed identity when
//   `TEXTURECRAFT_BUILD_CHANNEL=nightly-v1`, else the workspace package version. A Nightly build
//   without a well-formed `TEXTURECRAFT_VERSION` fails rather than falling back to the Cargo
//   version, so an archive can never carry the wrong identity.
// - Always sets `TEXTURECRAFT_BUILD_SHA` (full git commit) and `TEXTURECRAFT_BUILD_DATE`
//   (`YYYY-MM-DD`, UTC) for `env!`. They come from the same-named environment variables (CI and
//   packaging/env.sh set them), else from `git` and the clock (`SOURCE_DATE_EPOCH` wins for
//   reproducible builds); either is an empty string when unknown, so source tarballs, wasm and
//   offline builds still compile. Without the env vars the commit is read when this script runs,
//   so a local build can show a stale commit; release builds always set the variable.
// - Windows targets only: embeds the flavor's `assets/app-icon/*.ico` and VERSIONINFO into the exe.
//   A missing resource compiler is a warning, so a cross-compile still links, unless
//   `TEXTURECRAFT_REQUIRE_WINRES=1` (set by packaging/windows/package.ps1) makes it an error.

#[path = "src/build_channel.rs"]
mod build_channel;

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/build_channel.rs");
    for icon in [build_channel::windows_icon_path(false), build_channel::windows_icon_path(true)] {
        println!("cargo:rerun-if-changed={icon}");
    }
    for var in [
        "TEXTURECRAFT_BUILD_SHA",
        "TEXTURECRAFT_BUILD_DATE",
        "TEXTURECRAFT_BUILD_CHANNEL",
        "TEXTURECRAFT_VERSION",
        "TEXTURECRAFT_OVERRIDE_VERSION",
        "TEXTURECRAFT_REQUIRE_WINRES",
        "SOURCE_DATE_EPOCH",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo:rustc-check-cfg=cfg(texturecraft_nightly_v1)");
    let nightly = match std::env::var("TEXTURECRAFT_BUILD_CHANNEL").as_deref() {
        Ok("nightly-v1") => {
            println!("cargo:rustc-cfg=texturecraft_nightly_v1");
            true
        }
        // An empty value (an unset CI input) means the default stable build.
        Ok("stable" | "") | Err(std::env::VarError::NotPresent) => false,
        _ => {
            eprintln!("unsupported TEXTURECRAFT_BUILD_CHANNEL (expected stable or nightly-v1)");
            std::process::exit(1);
        }
    };
    // `TEXTURECRAFT_VERSION` is the feed identity the workflow exports; the packaging scripts
    // (packaging/env.sh, nightly-common.sh, linux/package.sh) spell the same value
    // `TEXTURECRAFT_OVERRIDE_VERSION`, so both name the version the binary reports.
    let requested = ["TEXTURECRAFT_VERSION", "TEXTURECRAFT_OVERRIDE_VERSION"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().map(|value| value.trim().to_owned()).filter(|value| !value.is_empty()));
    let built_version = if nightly {
        match requested {
            Some(version) if build_channel::is_nightly_version(&version) => version,
            _ => {
                eprintln!("nightly-v1 requires an explicit prepared feed identity in TEXTURECRAFT_VERSION");
                std::process::exit(1);
            }
        }
    } else {
        std::env::var("CARGO_PKG_VERSION").unwrap_or_default()
    };
    println!("cargo:rustc-env=TEXTURECRAFT_VERSION={built_version}");
    println!("cargo:rustc-env=TEXTURECRAFT_BUILD_SHA={}", env_or("TEXTURECRAFT_BUILD_SHA", git_sha));
    println!("cargo:rustc-env=TEXTURECRAFT_BUILD_DATE={}", env_or("TEXTURECRAFT_BUILD_DATE", today));

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources(nightly, &built_version);
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

fn windows_resources(nightly: bool, built_version: &str) {
    let name = std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "texturecraft".into());
    let description = if name.ends_with("-cli") {
        if nightly { "TextureCraft Nightly command-line tool" } else { "TextureCraft command-line tool" }
    } else if nightly {
        "TextureCraft Nightly 3D texture painter"
    } else {
        "TextureCraft 3D texture painter"
    };
    let mut res = winresource::WindowsResource::new();
    res.set_icon(build_channel::windows_icon_path(nightly))
        .set("ProductName", if nightly { "TextureCraft Nightly" } else { "TextureCraft" })
        .set("FileDescription", description)
        .set("CompanyName", "Learning Machines LLC")
        .set("LegalCopyright", "Copyright (c) the TextureCraft contributors. MIT OR Apache-2.0.")
        .set("OriginalFilename", &format!("{name}.exe"))
        .set("InternalName", &name);
    if nightly {
        // The Launcher checks the PE ProductVersion against the exact feed identity, not FileVersion.
        res.set("ProductVersion", built_version);
    }
    if let Err(e) = res.compile() {
        if std::env::var_os("TEXTURECRAFT_REQUIRE_WINRES").is_some() {
            println!("cargo::error=embedding Windows resources into {name}.exe failed: {e}");
        } else {
            println!("cargo:warning={name}.exe built without icon/version resources: {e}");
        }
    }
}
