//! Optional shared translation catalogs, embedded at build time.
use std::fmt::Write as _;
use std::path::Path;

/// The catalogs' Rust includes and the distinct characters they use, sorted: the web build cuts
/// its CJK interface face to those characters.
#[allow(dead_code)] // used by tests/i18n_build.rs, not by build.rs
pub fn collect(dir: Option<&Path>, required: bool) -> Result<(String, Vec<char>), String> {
    collect_into(dir, required, None)
}

/// `bytes` deflated (raw deflate, as `flate2::read::DeflateDecoder` reads it).
pub fn deflate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Write as _;
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(bytes).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

/// What the generated source does with the catalogs' text. A web build downloads the wasm, so it embeds each catalog
/// deflated (written under `out`) and inflates all of them on first use; other builds embed the text itself.
const INFLATE_FN: &str = r#"
fn inflate_text(deflated: &[u8]) -> &'static str {
    use std::io::Read as _;
    let mut text = String::new();
    // A catalog that does not inflate leaves its language without translations, never a panic.
    if flate2::read::DeflateDecoder::new(deflated).take(64 << 20).read_to_string(&mut text).is_err() {
        text.clear();
    }
    Box::leak(text.into_boxed_str())
}

pub fn craft_i18n_catalogs() -> &'static craft_i18n::Embedded {
    static CATALOGS: std::sync::OnceLock<&'static craft_i18n::Embedded> = std::sync::OnceLock::new();
    CATALOGS.get_or_init(|| {
        let catalogs: Vec<(&'static str, &'static str, &'static str)> =
            CRAFT_I18N_DEFLATED.iter().map(|&(locale, common, app)| (locale, inflate_text(common), inflate_text(app))).collect();
        Box::leak(catalogs.into_boxed_slice())
    })
}
"#;

/// [`collect`], embedding the catalogs deflated under the directory `deflated_in` when it is given.
pub fn collect_into(dir: Option<&Path>, required: bool, deflated_in: Option<&Path>) -> Result<(String, Vec<char>), String> {
    let mut source = match deflated_in {
        Some(_) => String::from("static CRAFT_I18N_DEFLATED: &[(&str, &[u8], &[u8])] = &[\n"),
        None => String::from("pub static CRAFT_I18N: &craft_i18n::Embedded = &[\n"),
    };
    let close = |mut source: String| {
        source.push_str("];\n");
        source.push_str(if deflated_in.is_some() {
            INFLATE_FN
        } else {
            "\npub fn craft_i18n_catalogs() -> &'static craft_i18n::Embedded {\n    CRAFT_I18N\n}\n"
        });
        source
    };
    let Some(dir) = dir else {
        if required {
            return Err("CRAFT_I18N_REQUIRED=1 requires CRAFT_I18N_DIR".into());
        }
        return Ok((close(source), Vec::new()));
    };

    if !dir.is_absolute() {
        return Err("CRAFT_I18N_DIR must be an absolute path".into());
    }

    let locales = dir.join("locales");
    let entries = std::fs::read_dir(&locales).map_err(|e| format!("{}: {e}", locales.display()))?;
    println!("cargo::rerun-if-changed={}", locales.display());
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            paths.push(entry.path());
        }
    }

    paths.sort();
    let mut count = 0;
    let mut catalogs = Vec::new();
    let mut characters = Vec::new();
    for path in paths {
        // Watch directories so adding or removing an app catalog rebuilds the embedding
        println!("cargo::rerun-if-changed={}", path.display());
        let app = path.join("texturecraft.tsv");
        if !app.is_file() {
            continue;
        }

        let locale = path.file_name().and_then(|n| n.to_str()).ok_or("non-UTF-8 locale")?;
        let common = path.join("common.tsv");
        let embed = |file: &Path| -> Result<(String, &'static str), String> {
            let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
            let name = file.to_str().ok_or("non-UTF-8 catalog path")?;
            println!("cargo::rerun-if-changed={name}");

            let include = match deflated_in {
                Some(out) => {
                    let copy = out.join(format!("{locale}.{}.deflate", file.file_stem().and_then(|n| n.to_str()).unwrap_or("catalog")));
                    std::fs::write(&copy, deflate(text.as_bytes())?).map_err(|e| format!("{}: {e}", copy.display()))?;
                    format!("include_bytes!({:?})", copy.display().to_string())
                }
                None => format!("include_str!({name:?})"),
            };
            // The shared runtime needs static inputs for this short-lived build process
            Ok((include, Box::leak(text.into_boxed_str())))
        };
        let empty = if deflated_in.is_some() { "&[]" } else { "\"\"" };
        let (shared, shared_text) = if common.is_file() { embed(&common)? } else { (empty.into(), "") };
        let (app, app_text) = embed(&app)?;
        characters.extend(shared_text.chars().chain(app_text.chars()));
        let code: &'static str = Box::leak(locale.to_owned().into_boxed_str());
        catalogs.push((code, shared_text, app_text));
        let _ = writeln!(source, "    ({locale:?}, {shared}, {app}),");
        count += 1;
    }
    if required && count == 0 {
        return Err("no TextureCraft translation catalogs found".into());
    }

    // Reject rows the runtime would skip before required builds can ship English fallbacks
    let (runtime, mut problems) = craft_i18n::I18n::new(Box::leak(catalogs.into_boxed_slice()), &[]);
    problems.extend(runtime.check_all());
    if !problems.is_empty() {
        return Err(format!("invalid translation catalogs in {}: {}", locales.display(), problems.join("; ")));
    }

    characters.sort_unstable();
    characters.dedup();
    Ok((close(source), characters))
}

/// Write the embedding for both desktop and wasm, reporting optional failures as warnings.
/// Required input errors stop Cargo instead of silently shipping an English-only release.
///
/// Returns the distinct characters the embedded catalogs use: the web build cuts its CJK interface
/// face to them, since every character kept costs glyph data in the wasm download.
pub fn embed() -> Vec<char> {
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_REQUIRED");
    let dir = std::env::var_os("CRAFT_I18N_DIR").map(std::path::PathBuf::from);
    let required = std::env::var("CRAFT_I18N_REQUIRED").is_ok_and(|v| v == "1");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let (source, characters) = match collect_into(dir.as_deref(), required, wasm.then_some(out_dir.as_path())) {
        Ok(embedded) => embedded,
        Err(error) => {
            if required {
                println!("cargo::error={error}");
            } else {
                println!("cargo::warning=building without craft-i18n: {error}");
            }
            (empty_embedding(wasm), Vec::new())
        }
    };

    let out = out_dir.join("craft_i18n.rs");
    if let Err(error) = std::fs::write(&out, source) {
        println!("cargo::error=writing {}: {error}", out.display());
    }
    characters
}

/// The embedding of a build without catalogs.
fn empty_embedding(deflated: bool) -> String {
    if deflated {
        format!("static CRAFT_I18N_DEFLATED: &[(&str, &[u8], &[u8])] = &[];\n{INFLATE_FN}")
    } else {
        "pub static CRAFT_I18N: &craft_i18n::Embedded = &[];\n\npub fn craft_i18n_catalogs() -> &'static craft_i18n::Embedded {\n    CRAFT_I18N\n}\n".into()
    }
}
