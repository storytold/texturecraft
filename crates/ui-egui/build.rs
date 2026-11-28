//! Embeds the optional craft-fonts UI fonts (storytold/craft-fonts) when `CRAFT_FONTS_DIR` points
//! at a checkout: Inter for the interface (from its Latin catalog) and the CJK and Arabic
//! faces; without it `CRAFT_FONTS` is empty and the UI uses egui's and the system's fonts. The
//! recipe follows craft-fonts' `docs/integration.md`; no font file is committed here. The Latin
//! faces are embedded without their Private Use Area ([`without_private_use`]), and a web build
//! embeds its CJK faces cut to the characters the shipped catalogs use ([`cut_to_catalogs`]):
//! the whole Japanese face alone is 4.5 MB of the wasm download.

mod i18n_build;

use skera::{Plan, SubsetFlags, subset_font};
use skrifa::raw::collections::IntSet;
use skrifa::raw::types::NameId;
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};
use std::fmt::Write as _;
use std::path::PathBuf;

/// The UI faces TextureCraft embeds (family, style) per manifest: desktop, then web (the web build
/// has a download budget: of the CJK faces only the Japanese one, cut to the catalogs' characters
/// by [`cut_to_catalogs`]).
const MANIFESTS: &[(&str, Faces, Faces)] = &[
    ("fonts/manifest.txt", &[("BIZ UDPGothic", "Regular"), ("Noto Sans CJK SC", "Regular"), ARABIC], &[("BIZ UDPGothic", "Regular"), ARABIC]),
    // The interface font has regular and semibold faces
    (LATIN_MANIFEST, LATIN, LATIN),
];
/// The Arabic face (0.8 MB, so the web build has it too).
const ARABIC: (&str, &str) = ("Noto Sans Arabic", "Regular");
/// Newer than the core catalog: a checkout without it keeps the default interface font.
const LATIN_MANIFEST: &str = "fonts/latin-manifest.txt";
const LATIN: Faces = &[("Inter", "Regular"), ("Inter", "SemiBold")];
/// (family, style) pairs of a manifest.
type Faces = &'static [(&'static str, &'static str)];

fn main() {
    let catalogs = i18n_build::embed();
    deflate_assets();
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    let mut src = String::from("pub static CRAFT_FONTS: &[CraftFont] = &[\n");
    if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR").map(PathBuf::from) {
        match MANIFESTS.iter().map(|(rel, desktop, web)| craft_fonts(&dir, rel, desktop, web, &catalogs)).collect::<Result<String, String>>() {
            Ok(entries) => src.push_str(&entries),
            Err(e) if std::env::var_os("CRAFT_FONTS_REQUIRED").is_some() => {
                println!("cargo::error=CRAFT_FONTS_DIR={}: {e}", dir.display());
            }
            Err(e) => println!("cargo::warning=building without craft-fonts: CRAFT_FONTS_DIR={}: {e}", dir.display()),
        }
    }

    src.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_fonts.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// One `CraftFont { .. }` initialiser per wanted line of manifest `rel`.
///
/// `catalogs` is the characters the embedded catalogs use ([`i18n_build::embed`]): a web build
/// cuts its CJK faces to them.
fn craft_fonts(dir: &std::path::Path, rel: &str, desktop: &[(&str, &str)], web: &[(&str, &str)], catalogs: &[char]) -> Result<String, String> {
    let manifest = dir.join(rel);
    println!("cargo::rerun-if-changed={}", manifest.display());
    if rel == LATIN_MANIFEST && !manifest.exists() {
        return Ok(String::new());
    }

    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let wanted = if wasm { web } else { desktop };
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let fields: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, ..] = fields.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };

        if !wanted.contains(&(*family, *style)) {
            continue;
        }

        let mut path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
        println!("cargo::rerun-if-changed={}", path.display());
        let tags: Vec<&str> = scripts.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();

        // Leave the Private Use Area to the icon font
        // Desktop CJK fonts also cover user file names outside the catalogs
        let cut = rel == LATIN_MANIFEST || (wasm && !catalogs.is_empty() && tags.iter().any(|tag| CJK_SCRIPTS.contains(tag)));
        if cut {
            let bytes = std::fs::read(&path).map_err(|e| format!("{file}: {e}"))?;
            let cut =
                if rel == LATIN_MANIFEST { without_private_use(&bytes) } else { cut_to_catalogs(&bytes, catalogs) }.map_err(|e| format!("{file}: {e}"))?;
            let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join(out_name(file, ""));
            std::fs::write(&out, cut).map_err(|e| format!("{}: {e}", out.display()))?;
            path = out;
        }

        // The web build embeds its faces deflated: the wasm is under a download budget and fonts halve.
        if wasm {
            let bytes = std::fs::read(&path).map_err(|e| format!("{file}: {e}"))?;
            let deflated = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join(out_name(file, ".deflate"));
            std::fs::write(&deflated, i18n_build::deflate(&bytes)?).map_err(|e| format!("{}: {e}", deflated.display()))?;
            path = deflated;
        }

        let scripts: Vec<String> = tags.iter().map(|s| format!("{s:?}")).collect();
        let _ = writeln!(
            out,
            "    CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], data: include_bytes!({:?}), deflated: {wasm} }},",
            scripts.join(", "),
            path.display().to_string(),
        );
    }

    Ok(out)
}

/// The scripts whose faces a web build cuts to the catalogs' characters: the CJK faces are most of
/// the web build's font download, and the whole Japanese face alone is 4.5 MB.
const CJK_SCRIPTS: [&str; 4] = ["Hans", "Hant", "Jpan", "Kore"];

/// `font` without the characters of the Private Use Area (U+E000–U+F8FF): the icon font (Phosphor)
/// draws there, and Inter, ahead of it in the fallback list, has alternates of its own there that
/// would show instead of the icons. Everything else (layout features, hinting) is kept, as
/// craft-libs' `craft-fonts-build` subsets.
fn without_private_use(font: &[u8]) -> Result<Vec<u8>, String> {
    let font = FontRef::new(font).map_err(|e| e.to_string())?;
    let mut unicodes = IntSet::<u32>::empty();
    unicodes.extend(font.charmap().mappings().map(|(c, _)| c).filter(|c| !(0xE000..=0xF8FF).contains(c)));
    subset(&font, &unicodes)
}

/// `font` with only the glyphs for the characters the shipped catalogs use, plus ASCII, Latin-1,
/// general punctuation (numbers, names and punctuation in the shipped strings), CJK punctuation
/// (U+3000–303F) and the kana blocks (U+3040–30FF), as craft-libs' `craft-fonts-build` keeps when
/// WordCraft cuts its interface face. The web build has one face per script. Shipped UI strings
/// and user-typed ASCII, Latin-1, punctuation and kana still draw; Han ideographs that are not in
/// the catalogs show as missing glyphs (the desktop build keeps the CJK faces whole, including a
/// user's Japanese file name). The whole Japanese face would otherwise be 4.5 MB of a download
/// that has to stay under the 24 MiB web size gate.
fn cut_to_catalogs(font: &[u8], catalogs: &[char]) -> Result<Vec<u8>, String> {
    let font = FontRef::new(font).map_err(|e| e.to_string())?;
    let mut unicodes = IntSet::<u32>::empty();
    unicodes.extend((0x20..=0xFF).chain(0x2000..=0x206F).chain(0x3000..=0x303F).chain(0x3040..=0x30FF).chain(catalogs.iter().map(|c| *c as u32)));
    subset(&font, &unicodes)
}

/// `font` with the glyphs for `unicodes` only; layout features, hinting and the face's names are
/// kept, as craft-libs' `craft-fonts-build` subsets.
fn subset(font: &FontRef, unicodes: &IntSet<u32>) -> Result<Vec<u8>, String> {
    let mut name_ids = IntSet::<NameId>::empty();
    name_ids.insert_range(NameId::new(0)..=NameId::new(6));
    let mut languages = IntSet::<u16>::empty();
    languages.insert(0x0409);
    let plan = Plan::new(
        &IntSet::<GlyphId>::empty(),
        unicodes,
        font,
        SubsetFlags::default(),
        &IntSet::<Tag>::empty(),
        &IntSet::<Tag>::all(),
        &IntSet::<Tag>::all(),
        &name_ids,
        &languages,
    );
    subset_font(font, &plan).map_err(|e| e.to_string())
}

/// A face's file name under `OUT_DIR`: its path in the craft-fonts checkout with every separator flattened, so faces
/// that share a file name (`jp/Regular.ttf`, `ar/Regular.ttf`) stay apart.
fn out_name(file: &str, suffix: &str) -> String {
    let flat: String = file.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    format!("craft-font-{flat}{suffix}")
}

/// The files a web build embeds deflated (the wasm is under a download budget): the icon font and the studio
/// environment, from this crate's directory to the name they have under `OUT_DIR` (`name.deflate`). The other builds
/// include the files themselves.
const DEFLATED_ASSETS: &[(&str, &str)] =
    &[("../../vendor/egui-phosphor/res/Phosphor.ttf", "Phosphor.ttf"), ("../../assets/hdr/studio_small_09_512.hdr", "studio_small_09_512.hdr")];

/// Writes [`DEFLATED_ASSETS`] under `OUT_DIR` for the web build.
fn deflate_assets() {
    if !std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32") {
        return;
    }

    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    for (source, name) in DEFLATED_ASSETS {
        let source = manifest.join(source);
        println!("cargo::rerun-if-changed={}", source.display());
        let target = out.join(format!("{name}.deflate"));
        let written = std::fs::read(&source)
            .map_err(|e| e.to_string())
            .and_then(|bytes| i18n_build::deflate(&bytes))
            .and_then(|d| std::fs::write(&target, d).map_err(|e| e.to_string()));
        if let Err(e) = written {
            println!("cargo::error={}: {e}", source.display());
        }
    }
}
