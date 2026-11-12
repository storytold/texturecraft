//! CJK, Arabic and Thai fallback fonts for the UI. egui's default fonts have no Japanese, Chinese,
//! Korean, Arabic or Thai glyphs, so translated UIs (and such layer or file names) need more fonts.
//! They come from:
//!
//! 1. craft-fonts, embedded at build time when `CRAFT_FONTS_DIR` points at a checkout (optional;
//!    see `build.rs` and storytold/craft-fonts' `docs/integration.md`);
//! 2. on desktop, the operating system's fonts (Windows, macOS, Linux and BSD paths).
//!
//! Fonts load lazily: one script when the UI language needs it, all of them while the language
//! picker shows every language's own name. Thai is no interface language, but its face loads in
//! every language, so Thai layer and file names and typed text draw instead of boxes. They are
//! appended after the app's fonts (`theme::font_definitions`), so Latin text and row heights keep the interface font; the current language's script goes first, so
//! shared Han characters take that language's glyph forms. egui shapes Arabic and orders
//! right-to-left runs itself. In the browser only craft-fonts can supply them (without it, the
//! language shows its English name in the picker).

use std::sync::Arc;

use egui::{FontData, FontFamily};

/// A font from the optional craft-fonts build input (empty unless built with `CRAFT_FONTS_DIR`).
pub struct CraftFont {
    pub family: &'static str,
    pub style: &'static str,
    /// ISO 15924 scripts the font is for, e.g. `"Jpan"`.
    pub scripts: &'static [&'static str],
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

/// Skip absurdly large files (a corrupt or non-font path must not eat memory).
#[cfg(not(target_arch = "wasm32"))]
const MAX_FONT_BYTES: u64 = 128 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Japanese,
    Chinese,
    Korean,
    Arabic,
    /// Not an interface language: installed in every language so Thai names and typed text draw.
    Thai,
}

impl Script {
    pub const ALL: [Script; 5] = [Script::Japanese, Script::Chinese, Script::Korean, Script::Arabic, Script::Thai];

    /// The script a UI language needs beyond Latin.
    pub fn of_language(code: &str) -> Option<Script> {
        match code {
            "ja" => Some(Script::Japanese),
            "zh-Hans" => Some(Script::Chinese),
            "ko" => Some(Script::Korean),
            "ar" => Some(Script::Arabic),
            _ => None,
        }
    }

    /// ISO 15924 code (craft-fonts' manifest).
    fn iso(self) -> &'static str {
        match self {
            Script::Japanese => "Jpan",
            Script::Chinese => "Hans",
            Script::Korean => "Kore",
            Script::Arabic => "Arab",
            Script::Thai => "Thai",
        }
    }

    /// Candidate system font files: (path, face index in a collection).
    #[cfg(not(target_arch = "wasm32"))]
    fn system_files(self) -> Vec<(std::path::PathBuf, u32)> {
        use std::path::PathBuf;
        let mut v: Vec<(PathBuf, u32)> = Vec::new();
        if cfg!(windows) {
            let fonts = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
            let files: &[&str] = match self {
                Script::Japanese => &["YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"],
                Script::Chinese => &["msyh.ttc", "simsun.ttc", "simhei.ttf"],
                Script::Korean => &["malgun.ttf", "gulim.ttc"],
                Script::Arabic => &["segoeui.ttf", "tahoma.ttf", "arial.ttf"],
                Script::Thai => &["LeelawUI.ttf", "leelawad.ttf", "tahoma.ttf"],
            };
            v.extend(files.iter().map(|f| (fonts.join(f), 0)));
        } else if cfg!(target_os = "macos") {
            let files: &[&str] = match self {
                Script::Japanese => &["/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "/System/Library/Fonts/Hiragino Sans GB.ttc"],
                Script::Chinese => {
                    &["/System/Library/Fonts/Hiragino Sans GB.ttc", "/System/Library/Fonts/STHeiti Light.ttc", "/System/Library/Fonts/PingFang.ttc"]
                }
                Script::Korean => &["/System/Library/Fonts/AppleSDGothicNeo.ttc"],
                Script::Arabic => &["/System/Library/Fonts/SFArabic.ttf", "/System/Library/Fonts/Supplemental/GeezaPro.ttc"],
                Script::Thai => &["/System/Library/Fonts/Supplemental/Thonburi.ttc", "/System/Library/Fonts/Thonburi.ttc"],
            };
            v.extend(files.iter().map(|f| (PathBuf::from(f), 0)));
            v.push((PathBuf::from("/System/Library/Fonts/Supplemental/Arial Unicode.ttf"), 0));
        } else if self == Script::Thai {
            for f in [
                "/usr/share/fonts/truetype/noto/NotoSansThai-Regular.ttf",
                "/usr/share/fonts/noto/NotoSansThai-Regular.ttf",
                "/usr/share/fonts/google-noto/NotoSansThai-Regular.ttf",
                "/usr/share/fonts/truetype/tlwg/Loma.ttf",
            ] {
                v.push((PathBuf::from(f), 0));
            }
        } else if self == Script::Arabic {
            for f in [
                "/usr/share/fonts/truetype/noto/NotoSansArabic-Regular.ttf",
                "/usr/share/fonts/noto/NotoSansArabic-Regular.ttf",
                "/usr/share/fonts/google-noto/NotoSansArabic-Regular.ttf",
                "/usr/local/share/fonts/noto/NotoSansArabic-Regular.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/dejavu/DejaVuSans.ttf",
                "/usr/local/share/fonts/dejavu/DejaVuSans.ttf",
            ] {
                v.push((PathBuf::from(f), 0));
            }
        } else {
            // Noto Sans CJK collections hold JP, KR, SC, TC and HK faces, in that order.
            let face = match self {
                Script::Japanese | Script::Arabic | Script::Thai => 0,
                Script::Korean => 1,
                Script::Chinese => 2,
            };
            for dir in ["/usr/share/fonts/opentype/noto", "/usr/share/fonts/noto-cjk", "/usr/share/fonts/google-noto-cjk", "/usr/local/share/fonts/noto"] {
                v.push((PathBuf::from(dir).join("NotoSansCJK-Regular.ttc"), face));
            }
            if self == Script::Korean {
                v.push((PathBuf::from("/usr/share/fonts/truetype/nanum/NanumGothic.ttf"), 0));
            }
            for f in [
                "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
                "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
                "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
            ] {
                v.push((PathBuf::from(f), 0));
            }
        }
        v
    }
}

/// The fallback fonts installed in the egui context, and the ones loaded so far.
#[derive(Default)]
pub struct UiFonts {
    /// Scripts installed, in priority order.
    installed: Vec<Script>,
    /// Font data per script (`None`: nothing found), loaded once.
    loaded: Vec<(Script, Option<Arc<FontData>>)>,
    /// System font files read so far, by (path, face index): scripts that resolve to the same
    /// file (Tahoma, Arial Unicode) share one copy.
    files: Vec<(std::path::PathBuf, u32, Arc<FontData>)>,
}

impl UiFonts {
    /// Install the fonts `language` needs, or every script's when `all` (the language picker is
    /// showing), and the Thai face in every language. Fonts stay once loaded; the language's
    /// script moves to the front.
    pub fn update(&mut self, ctx: &egui::Context, language: &str, all: bool) {
        let first = Script::of_language(language);
        let thai_installed = self.installed.contains(&Script::Thai);
        if thai_installed && first.is_none_or(|f| self.installed.first() == Some(&f)) && (!all || self.installed.len() == Script::ALL.len()) {
            return;
        }
        let mut want: Vec<Script> = first.into_iter().collect();
        for s in Script::ALL {
            if (all || s == Script::Thai || self.installed.contains(&s)) && !want.contains(&s) {
                want.push(s);
            }
        }
        if want == self.installed {
            return;
        }
        ctx.set_fonts(self.definitions(&want));
        self.installed = want;
    }

    /// The app's fonts with the faces for `scripts` appended to every family, in that order.
    fn definitions(&mut self, scripts: &[Script]) -> egui::FontDefinitions {
        let mut defs = crate::theme::font_definitions();
        let mut added: Vec<Arc<FontData>> = Vec::new();
        for s in scripts {
            let Some(data) = self.data(*s) else { continue };
            // The same face, already added for an earlier script, draws these glyphs too.
            if added.iter().any(|d| Arc::ptr_eq(d, &data)) {
                continue;
            }
            let name = format!("script-{}", s.iso());
            defs.font_data.insert(name.clone(), data.clone());
            added.push(data);
            for family in [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name(crate::theme::SEMIBOLD.into())] {
                defs.families.entry(family).or_default().push(name.clone());
            }
        }
        defs
    }

    /// Whether some installed font draws `script` (the picker falls back to English names).
    pub fn covers(&self, script: Script) -> bool {
        self.installed.contains(&script) && self.loaded.iter().any(|(s, d)| *s == script && d.is_some())
    }

    fn data(&mut self, script: Script) -> Option<Arc<FontData>> {
        if let Some((_, d)) = self.loaded.iter().find(|(s, _)| *s == script) {
            return d.clone();
        }
        let d = load(script, &mut self.files);
        match &d {
            Some(_) => log::info!("UI font fallback for {}", script.iso()),
            None => log::warn!("no font found for {} text; it shows as boxes", script.iso()),
        }
        self.loaded.push((script, d.clone()));
        d
    }
}

/// The first available font for `script`: craft-fonts' (embedded), then the system's, reusing a
/// system file already in `files`.
fn load(script: Script, files: &mut Vec<(std::path::PathBuf, u32, Arc<FontData>)>) -> Option<Arc<FontData>> {
    if let Some(f) = CRAFT_FONTS.iter().find(|f| f.scripts.contains(&script.iso()) && f.style == "Regular" && !f.bytes.is_empty()) {
        return Some(Arc::new(aligned(FontData::from_static(f.bytes))));
    }
    #[cfg(not(target_arch = "wasm32"))]
    for (path, index) in script.system_files() {
        if let Some(data) = read_font(&path, index, files) {
            return Some(data);
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = files;
    None
}

/// Face `index` of the font file at `path`, from `files` when already read, else read once and
/// added there. `None` when it is missing, empty or too large.
#[cfg(not(target_arch = "wasm32"))]
fn read_font(path: &std::path::Path, index: u32, files: &mut Vec<(std::path::PathBuf, u32, Arc<FontData>)>) -> Option<Arc<FontData>> {
    if let Some((_, _, data)) = files.iter().find(|(p, i, _)| p == path && *i == index) {
        return Some(data.clone());
    }
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_FONT_BYTES {
        return None;
    }
    let mut data = FontData::from_owned(std::fs::read(path).ok()?);
    data.index = index;
    let data = Arc::new(aligned(data));
    files.push((path.to_path_buf(), index, data.clone()));
    Some(data)
}

/// `data` shifted so its baseline meets the UI font's. egui centers a fallback face's line box
/// in the primary font's, so faces with taller boxes (most CJK fonts) would sit high.
fn aligned(mut data: FontData) -> FontData {
    let defs = crate::theme::font_definitions();
    let primary = defs.families.get(&FontFamily::Proportional).and_then(|f| f.first()).and_then(|n| defs.font_data.get(n));
    if let (Some((pa, pd, pg)), Some((ca, cd, cg))) =
        (primary.and_then(|p| line_metrics(p.font.as_ref().as_ref(), p.index)), line_metrics(data.font.as_ref().as_ref(), data.index))
    {
        // Baseline of a fallback glyph: its ascent plus half the difference of the line boxes.
        let drift = ca + 0.5 * ((pa - pd + pg) - (ca - cd + cg)) - pa;
        if drift.is_finite() && drift.abs() < 1.0 {
            data.tweak.y_offset_factor = -drift;
        }
    }
    data
}

/// (ascent, descent, line gap) of face `index` in em units, chosen as egui's font reader does:
/// OS/2 typographic metrics when the font asks for them, else `hhea` (then OS/2 when `hhea` is
/// zero). `descent` is negative.
fn line_metrics(font: &[u8], index: u32) -> Option<(f32, f32, f32)> {
    let u16_at = |o: usize| font.get(o..o.checked_add(2)?).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32_at = |o: usize| font.get(o..o.checked_add(4)?).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let i16_at = |o: usize| u16_at(o).map(|v| v as i16 as f32);
    let dir = if font.get(..4)? == b"ttcf" { u32_at(12usize.checked_add(4usize.checked_mul(index as usize)?)?)? as usize } else { 0 };
    let tables = u16_at(dir.checked_add(4)?)? as usize;
    let table = |tag: &[u8; 4]| {
        (0..tables).find_map(|i| {
            let rec = dir.checked_add(12)?.checked_add(i.checked_mul(16)?)?;
            (font.get(rec..rec.checked_add(4)?)? == tag).then(|| u32_at(rec + 8).map(|o| o as usize)).flatten()
        })
    };
    let upem = f32::from(u16_at(table(b"head")?.checked_add(18)?)?.max(1));
    let os2 = table(b"OS/2");
    let typo = |o: usize| Some((i16_at(o + 68)?, i16_at(o + 70)?, i16_at(o + 72)?));
    let use_typo = os2.and_then(|o| u16_at(o + 62)).is_some_and(|sel| sel & (1 << 7) != 0);
    let (a, d, g) = match (use_typo, os2) {
        (true, Some(o)) => typo(o)?,
        _ => {
            let h = table(b"hhea")?;
            let m = (i16_at(h + 4)?, i16_at(h + 6)?, i16_at(h + 8)?);
            if m.0 == 0.0 && m.1 == 0.0 { os2.and_then(typo).unwrap_or(m) } else { m }
        }
    };
    Some((a / upem, d / upem, g / upem))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_metrics_read_the_default_font_and_reject_garbage() {
        let defs = egui::FontDefinitions::default();
        let ubuntu = defs.font_data.get("Ubuntu-Light").unwrap();
        let (a, d, g) = line_metrics(ubuntu.font.as_ref().as_ref(), 0).unwrap();
        assert!(a > 0.5 && a < 1.5 && d < 0.0 && d > -0.6 && g >= 0.0, "{a} {d} {g}");
        // The primary font (Inter with craft-fonts, else egui's) needs no shift against itself.
        let ui = crate::theme::font_definitions();
        let primary = ui.families.get(&FontFamily::Proportional).and_then(|f| f.first()).and_then(|n| ui.font_data.get(n)).unwrap();
        assert_eq!(aligned(FontData::from_owned(primary.font.as_ref().as_ref().to_vec())).tweak.y_offset_factor, 0.0);
        for junk in [&b""[..], b"ttcf", b"\0\x01\0\0\0\x09", &[0xff; 64]] {
            assert!(line_metrics(junk, 0).is_none());
            assert!(line_metrics(junk, 7).is_none());
        }
    }

    #[test]
    fn languages_map_to_scripts() {
        assert_eq!(Script::of_language("ja"), Some(Script::Japanese));
        assert_eq!(Script::of_language("zh-Hans"), Some(Script::Chinese));
        assert_eq!(Script::of_language("ko"), Some(Script::Korean));
        assert_eq!(Script::of_language("ar"), Some(Script::Arabic));
        assert_eq!(Script::of_language("de"), None);
        for l in &crate::i18n::LANGUAGES {
            // Past the Latin blocks and their diacritics.
            let other_script = l.name.chars().any(|c| c > '\u{036F}');
            assert_eq!(other_script, Script::of_language(l.code).is_some(), "{}", l.code);
        }
    }

    #[test]
    fn fonts_install_once_and_keep_the_language_script_first() {
        let ctx = egui::Context::default();
        let mut f = UiFonts::default();
        f.update(&ctx, "en", false);
        assert_eq!(f.installed, [Script::Thai], "English loads only the Thai face");
        f.update(&ctx, "ko", false);
        assert_eq!(f.installed, [Script::Korean, Script::Thai]);
        f.update(&ctx, "ja", true);
        assert_eq!(f.installed, Script::ALL);
        // Closing the picker keeps what is loaded; each script was read at most once.
        f.update(&ctx, "ja", false);
        assert_eq!(f.installed.len(), Script::ALL.len());
        assert_eq!(f.loaded.len(), Script::ALL.len());
    }

    /// A stand-in face (egui's bundled Ubuntu-Light), for tests that must not depend on installed
    /// fonts.
    fn stand_in() -> Arc<FontData> {
        let defs = egui::FontDefinitions::default();
        Arc::new((**defs.font_data.get("Ubuntu-Light").unwrap()).clone())
    }

    /// The Thai face is the last entry of every family in every language, after the language's
    /// own script, whatever fonts the machine has: stand-in faces are injected as already loaded.
    #[test]
    fn thai_is_the_last_fallback_in_every_family() {
        for (language, scripts) in [("en", &[Script::Thai][..]), ("ko", &[Script::Korean, Script::Thai][..])] {
            let ctx = egui::Context::default();
            let mut f = UiFonts { loaded: vec![(Script::Korean, Some(stand_in())), (Script::Thai, Some(stand_in()))], ..Default::default() };
            f.update(&ctx, language, false);
            assert_eq!(f.installed, scripts, "{language}");
            assert!(f.covers(Script::Thai));
            let defs = f.definitions(scripts);
            assert!(defs.font_data.contains_key("script-Thai"));
            let tail: Vec<String> = scripts.iter().map(|s| format!("script-{}", s.iso())).collect();
            for family in [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name(crate::theme::SEMIBOLD.into())] {
                let names = defs.families.get(&family).cloned().unwrap_or_default();
                assert!(names.ends_with(&tail), "{language}: {family:?} {names:?}");
            }
            // Nothing beyond the injected faces was probed.
            assert_eq!(f.loaded.len(), 2);
        }
    }

    /// A missing Thai font is remembered: later updates do not probe for it again.
    #[test]
    fn missing_thai_font_is_probed_once() {
        let ctx = egui::Context::default();
        // As left by the first probe on a machine without a Thai font.
        let mut f = UiFonts { loaded: vec![(Script::Thai, None)], ..Default::default() };
        for _ in 0..3 {
            f.update(&ctx, "en", false);
            assert_eq!(f.installed, [Script::Thai]);
            assert_eq!(f.loaded.len(), 1, "Thai was probed again");
            assert!(!f.covers(Script::Thai));
        }
    }

    /// Scripts that resolve to the same font file share one copy, added to the families once.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_shared_font_file_is_read_and_installed_once() {
        let path = std::env::temp_dir().join(format!("texturecraft-shared-font-{}.ttf", std::process::id()));
        std::fs::write(&path, stand_in().font.as_ref()).unwrap();
        let mut files = Vec::new();
        let first = read_font(&path, 0, &mut files);
        let second = read_font(&path, 0, &mut files);
        std::fs::remove_file(&path).unwrap();
        let (first, second) = (first.unwrap(), second.unwrap());
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(files.len(), 1);

        let mut f = UiFonts { loaded: vec![(Script::Arabic, Some(first)), (Script::Thai, Some(second))], ..Default::default() };
        let defs = f.definitions(&[Script::Arabic, Script::Thai]);
        assert!(defs.font_data.contains_key("script-Arab"));
        assert!(!defs.font_data.contains_key("script-Thai"));
        let names = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
        assert_eq!(names.iter().filter(|n| n.starts_with("script-")).count(), 1, "{names:?}");
    }

    /// Thai layer and file names draw in every family whatever the interface language (#362);
    /// skipped when there is no Thai font, as font files are not committed.
    #[test]
    fn thai_draws_in_every_family_in_every_language() {
        if load(Script::Thai, &mut Vec::new()).is_none() {
            eprintln!("skipping: no Thai font");
            return;
        }

        for language in ["en", "ja", "ar"] {
            let ctx = egui::Context::default();
            let mut f = UiFonts::default();
            f.update(&ctx, language, false);
            let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
            output.textures_delta.clear();
            for family in [FontFamily::Proportional, FontFamily::Monospace, FontFamily::Name(crate::theme::SEMIBOLD.into())] {
                let font = egui::FontId::new(13.0, family.clone());
                assert!(ctx.fonts_mut(|v| v.has_glyphs(&font, "สวัสดีเลเยอร์")), "{language}: {family:?} has no Thai");
            }
        }
    }
}
