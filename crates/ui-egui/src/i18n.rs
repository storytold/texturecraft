//! UI text lookup over the optional shared craft-i18n catalogs.
//! Missing translations and builds without catalogs use the English source text.

use std::cell::Cell;
use std::fmt::Display;
use std::sync::{LazyLock, OnceLock};

include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));

static I18N: LazyLock<craft_i18n::I18n> = LazyLock::new(|| {
    let offered: Vec<_> = LANGUAGES.iter().map(|l| l.code).collect();
    let (i18n, problems) = craft_i18n::I18n::new(CRAFT_I18N, &offered);
    for problem in problems {
        log::warn!("translation catalog: {problem}");
    }
    i18n
});

static SHARED_LANGUAGES: OnceLock<[craft_i18n::Lang; COUNT]> = OnceLock::new();

/// Resolve the fixed picker codes once; hot text lookups only load the cached language handle.
fn shared_languages() -> &'static [craft_i18n::Lang; COUNT] {
    SHARED_LANGUAGES.get_or_init(|| LANGUAGES.each_ref().map(|l| I18N.from_code(l.code).unwrap_or(craft_i18n::Lang::EN)))
}

/// Whether this build embeds a catalog for a supported language.
pub fn available(code: &str) -> bool {
    I18N.from_code(code).is_some()
}

pub struct Language {
    /// BCP 47 code ("en", "pt-BR", "zh-Hans").
    pub code: &'static str,
    /// The language's own name, for the picker.
    pub name: &'static str,
    /// Its English name (the picker's fallback when no font can draw `name`).
    pub english: &'static str,
}

const COUNT: usize = 10;

pub static LANGUAGES: [Language; COUNT] = [
    Language { code: "en", name: "English", english: "English" },
    Language { code: "fr", name: "Français", english: "French" },
    Language { code: "de", name: "Deutsch", english: "German" },
    Language { code: "es", name: "Español", english: "Spanish" },
    Language { code: "pt-BR", name: "Português (Brasil)", english: "Portuguese (Brazil)" },
    Language { code: "it", name: "Italiano", english: "Italian" },
    Language { code: "ja", name: "日本語", english: "Japanese" },
    Language { code: "zh-Hans", name: "简体中文", english: "Chinese (Simplified)" },
    Language { code: "ko", name: "한국어", english: "Korean" },
    Language { code: "ar", name: "العربية", english: "Arabic" },
];

// Rendering and preferences use the UI thread, including independent test harnesses.
thread_local! {
    static CURRENT: Cell<usize> = const { Cell::new(0) };
}

/// How a fill reads its images, as the Filtering combo names it
pub(crate) fn filtering_label(filtering: texturecraft_engine::core::Filtering) -> &'static str {
    use texturecraft_engine::core::Filtering;
    match filtering {
        Filtering::BilinearHq => n_("Bilinear | HQ"),
        Filtering::BilinearSharp => n_("Bilinear | Sharp"),
        Filtering::Nearest => n_("Nearest"),
    }
}

/// Whether a fill repeats past its tile, as the UV Wrap combo names it
pub(crate) fn wrap_label(wrap: texturecraft_engine::core::UvWrap) -> &'static str {
    use texturecraft_engine::core::UvWrap;
    match wrap {
        UvWrap::None => n_("None"),
        UvWrap::RepeatHorizontally => n_("Repeat horizontally"),
        UvWrap::RepeatVertically => n_("Repeat vertically"),
        UvWrap::Repeat => n_("Repeat"),
    }
}

/// Display labels belong to the UI; the model exposes only map identities
pub(crate) fn baked_map_label(map: texturecraft_engine::core::BakedMap) -> &'static str {
    use texturecraft_engine::core::BakedMap;

    match map {
        BakedMap::Normal => n_("Normal"),
        BakedMap::WorldNormal => n_("World Space Normal"),
        BakedMap::Id => n_("ID"),
        BakedMap::AmbientOcclusion => n_("Ambient Occlusion"),
        BakedMap::Curvature => n_("Curvature"),
        BakedMap::Position => n_("Position"),
        BakedMap::Thickness => n_("Thickness"),
        BakedMap::Height => n_("Height"),
        BakedMap::BentNormal => n_("Bent Normals"),
        BakedMap::Opacity => n_("Opacity"),
    }
}

/// The saved language that follows the operating system's (Settings > Language > Default (System Language)).
pub const SYSTEM_LANGUAGE: &str = "";

/// Switch the calling UI thread's language. [`SYSTEM_LANGUAGE`] follows the system.
pub fn set_language(code: &str) {
    let code = if code == SYSTEM_LANGUAGE { system_language() } else { code };
    CURRENT.set(LANGUAGES.iter().position(|l| l.code.eq_ignore_ascii_case(code) && available(l.code)).unwrap_or(0));
}

/// The current UI language.
pub fn current() -> &'static Language {
    LANGUAGES.get(CURRENT.get()).unwrap_or(&LANGUAGES[0])
}

/// `text` in the current language (unchanged when it has no translation).
pub fn tr(text: &str) -> &str {
    let lang = shared_languages().get(CURRENT.get()).copied().unwrap_or(craft_i18n::Lang::EN);
    I18N.t(lang, text)
}

/// Marks a static string for translation where it is defined (gettext's `N_`); it is translated
/// with [`tr`] where it is shown. The key test collects both.
pub const fn n_(text: &'static str) -> &'static str {
    text
}

/// `template` translated, with each `{name}` replaced by its value in `args`.
pub fn trf(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let values: Vec<_> = args.iter().map(|(name, value)| (*name, value.to_string())).collect();
    let args: Vec<_> = values.iter().map(|(name, value)| (*name, value.as_str())).collect();
    craft_i18n::fmt(tr(template), &args)
}

/// An undo step's label in the current language. A label with a layer name, path number or
/// smart mask name in it ("Paint on Rust") is matched to its engine template, which is translated
/// and filled in again; any other label is translated as it is.
pub fn tr_history(label: &str) -> String {
    tr_history_with(label, |text| tr(text).to_string())
}

/// [`tr_history`] with its own lookup `translate`
fn tr_history_with(label: &str, translate: impl Fn(&str) -> String) -> String {
    match history_template(label) {
        Some((template, name, value)) => craft_i18n::fmt(&translate(template), &[(name, value)]),
        None => translate(label),
    }
}

/// The engine template a history label was made from, its placeholder name and the value in it.
/// The template with the longest fixed start wins.
fn history_template(label: &str) -> Option<(&'static str, &'static str, &str)> {
    texturecraft_engine::HISTORY_TEMPLATES
        .iter()
        .filter_map(|template| {
            let (open, close) = (template.find('{')?, template.find('}')?);
            let (prefix, suffix) = (&template[..open], &template[close + 1..]);
            let value = label.strip_prefix(prefix)?.strip_suffix(suffix)?;
            Some((prefix.len(), *template, &template[open + 1..close], value))
        })
        .max_by_key(|(prefix_length, ..)| *prefix_length)
        .map(|(_, template, name, value)| (template, name, value))
}

/// The supported language for a locale such as "de-AT", "pt_BR.UTF-8" or "zh-Hant-TW" ("en" when
/// none matches). Chinese locales get Simplified Chinese; Portuguese ones get Brazilian Portuguese.
pub fn language_for_locale(locale: &str) -> &'static str {
    let lang = locale.split(['-', '_', '.', '@']).next().unwrap_or("").to_ascii_lowercase();
    match lang.as_str() {
        "pt" => "pt-BR",
        "zh" => "zh-Hans",
        l => LANGUAGES.iter().find(|x| x.code == l).map_or("en", |x| x.code),
    }
}

/// The operating system's (or browser's) UI language, for the first run.
pub fn os_language() -> &'static str {
    #[cfg(not(target_arch = "wasm32"))]
    let locale = sys_locale::get_locale();
    #[cfg(target_arch = "wasm32")]
    let locale = web_sys::window().and_then(|w| w.navigator().language());
    locale.map_or("en", |l| language_for_locale(&l))
}

/// [`os_language`] read once: the language is applied every frame.
fn system_language() -> &'static str {
    static SYSTEM: OnceLock<&'static str> = OnceLock::new();
    SYSTEM.get_or_init(os_language)
}

/// Shared guard used by language fixture tests.
#[cfg(test)]
pub(crate) static TEST_LANGUAGE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::TEST_LANGUAGE as LANGUAGE;
    use super::*;
    use std::collections::{BTreeSet, HashMap};

    #[test]
    fn a_parallel_ui_language_context_keeps_its_own_language() {
        set_language("en");
        let french = LANGUAGES.iter().position(|language| language.code == "fr").unwrap();
        let barrier = std::sync::Barrier::new(2);
        let (first, second) = std::thread::scope(|scope| {
            let other = scope.spawn(|| {
                // Exercise the resolved language selection even when the French catalog is absent.
                CURRENT.set(french);
                barrier.wait();
                barrier.wait();
                current().code
            });
            barrier.wait();
            let first = current().code;
            barrier.wait();
            (first, other.join().unwrap())
        });
        assert_eq!((first, second), ("en", "fr"));
    }

    /// Every key the UI looks up: `tr("…")` / `trf("…"` literals in the sources, plus the
    /// static catalogs whose labels are translated where they are shown.
    fn keys() -> Vec<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut keys = Vec::new();
        let mut directories = vec![dir];
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() && path.file_name().is_some_and(|name| name != "i18n") {
                    directories.push(path);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    literals(&std::fs::read_to_string(path).unwrap(), &mut keys);
                }
            }
        }
        keys.extend(catalog_keys().into_iter().map(str::to_owned));
        keys.sort();
        keys.dedup();
        keys
    }

    /// The labels of the menu catalog, panels, shortcuts, shelf tabs and engine enums that are
    /// translated where they are shown.
    fn catalog_keys() -> Vec<&'static str> {
        use texturecraft_engine::core::{
            BlendMode, Channel, EffectKind, GeneratorKind, ProceduralKind, TileBlending, TileInputDistribution, TileShape, TileSizeMode, TileSymmetry,
        };
        use texturecraft_engine::filter_lib::{FILTERS, build, scalar_params};
        let mut keys: Vec<&'static str> = Vec::new();
        keys.extend(crate::menus::TOP);
        for item in crate::menus::CATALOG.iter().filter(|item| item.label != "-") {
            keys.push(item.label);
            keys.extend(item.sub);
        }
        keys.extend(crate::docking::VIEWS.iter().map(|(_, panel)| panel.name()));
        for binding in crate::keymap::bindings() {
            keys.push(binding.label);
            keys.push(binding.group);
        }
        keys.extend(crate::shelf::TABS.iter().map(|tab| tab.2));
        keys.extend(Channel::ALL.iter().flat_map(|channel| [channel.label(), channel.short()]));
        keys.extend(BlendMode::all().map(|mode| mode.label()));
        keys.extend(texturecraft_engine::formats::export::VirtualMap::ALL.iter().map(|map| map.label()));
        keys.extend(texturecraft_engine::Tool::ALL.iter().map(|tool| tool.label()));
        keys.extend(texturecraft_engine::HISTORY_TEMPLATES);
        keys.extend(ProceduralKind::ALL.iter().map(|kind| kind.label()));
        keys.extend(GeneratorKind::ALL.iter().map(|kind| kind.label()));
        keys.extend(TileShape::ALL.iter().map(|shape| shape.label()));
        keys.extend(TileSizeMode::ALL.iter().map(|mode| mode.label()));
        keys.extend(TileInputDistribution::ALL.iter().map(|distribution| distribution.label()));
        keys.extend(TileSymmetry::ALL.iter().map(|symmetry| symmetry.label()));
        keys.extend(TileBlending::ALL.iter().map(|blending| blending.label()));
        keys.extend(texturecraft_engine::paint::BrushAlpha::ALL.iter().map(|alpha| alpha.label()));
        keys.extend(texturecraft_engine::geom::primitives::BUILTINS.iter().map(|(_, label)| *label));
        for filter in FILTERS {
            keys.extend([filter.label, filter.group]);
            if let Some(about) = filter.about() {
                keys.push(about.description);
                keys.extend(about.tags);
            }
            if let Ok(EffectKind::Filter(effect)) = build(filter.id, &serde_json::Value::Null) {
                keys.push(effect.label());
                keys.extend(scalar_params(&effect).into_iter().map(|parameter| parameter.1));
            }
        }

        // Include effect, view and camera names supplied by the engine
        keys.extend([
            "Paint",
            "Path",
            "Fill",
            "Generator",
            "Levels",
            "Color selection",
            "Compare mask",
            "Anchor point",
            "Layer mask",
            texturecraft_engine::camera_cmds::DEFAULT_CAMERA,
            crate::dock::SOLO_HINT,
        ]);
        keys.retain(|key| !key.is_empty() && !UNTRANSLATED.contains(key));
        keys
    }

    /// String literals passed straight to `tr(` or `trf(` in `src`.
    fn literals(src: &str, out: &mut Vec<String>) {
        let src = src
            .split(
                "#[cfg(test)]
mod tests",
            )
            .next()
            .unwrap_or(src);
        for pat in ["tr(", "trf(", "n_("] {
            let mut rest = src;
            while let Some(i) = rest.find(pat) {
                let before = rest[..i].chars().next_back();
                rest = rest[i + pat.len()..].trim_start();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let Some(after) = rest.strip_prefix('"') else { continue };
                rest = after;
                let mut s = String::new();
                let mut chars = rest.chars();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some(c) => s.push(c),
                            None => break,
                        },
                        c => s.push(c),
                    }
                }
                out.push(s);
            }
        }
    }

    #[test]
    fn the_system_language_setting_speaks_the_operating_system_language() {
        let _language = LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_language(SYSTEM_LANGUAGE);
        let shown = current().code;
        set_language("en");

        let expected = if available(os_language()) { os_language() } else { "en" };
        assert_eq!(shown, expected);
    }

    #[test]
    fn embedded_catalogs_are_valid() {
        assert!(I18N.check_all().is_empty(), "{:?}", I18N.check_all());
    }

    /// Translation work never blocks a feature: report missing/stale keys without failing.
    #[test]
    fn translation_coverage_report() {
        let keys = keys();
        assert!(keys.len() > 300, "found only {} keys", keys.len());
        let used: BTreeSet<_> = keys.iter().map(String::as_str).collect();
        for &(locale, common, app) in CRAFT_I18N {
            let Some(lang) = I18N.from_code(locale) else { continue };
            let missing: Vec<_> = keys.iter().filter(|key| I18N.get(lang, "", key).is_none()).collect();

            // Decode TSV escapes to compare with literal source strings
            let mut rows = HashMap::new();
            for text in [common, app] {
                for line in text.lines().filter(|l| !l.starts_with('#')) {
                    let columns: Vec<_> = line.split('\t').collect();
                    if let ["", source, _] = columns.as_slice() {
                        let mut decoded = String::new();
                        let mut chars = source.chars();
                        while let Some(c) = chars.next() {
                            decoded.push(if c == '\\' {
                                match chars.next() {
                                    Some('n') => '\n',
                                    Some('t') => '\t',
                                    Some(c) => c,
                                    None => '\\',
                                }
                            } else {
                                c
                            });
                        }
                        rows.insert(decoded, ());
                    }
                }
            }
            let stale: Vec<_> = rows.keys().filter(|key| !used.contains(key.as_str())).collect();
            eprintln!("{locale}: missing {} {missing:?}; unused/shared {} {stale:?}", missing.len(), stale.len());
        }
    }

    #[test]
    fn history_labels_are_matched_to_their_templates() {
        assert_eq!(history_template("Paint on Rust"), Some(("Paint on {layer}", "layer", "Rust")));
        assert_eq!(history_template("Paint on Paint mask of x"), Some(("Paint on {layer}", "layer", "Paint mask of x")));
        assert_eq!(history_template("Paint mask of Paint on x"), Some(("Paint mask of {layer}", "layer", "Paint on x")));
        assert_eq!(history_template("Erase mask of Rust"), Some(("Erase mask of {layer}", "layer", "Rust")));
        assert_eq!(history_template("Edit path 2"), Some(("Edit path {n}", "n", "2")));
        assert_eq!(history_template("Apply smart mask Dirt"), Some(("Apply smart mask {name}", "name", "Dirt")));
        assert_eq!(history_template("Rename layer"), None);
    }

    /// The template is translated and the layer name filled in; other labels are translated whole
    #[test]
    fn history_labels_with_a_layer_name_are_translated_through_their_template() {
        let translate = |text: &str| {
            match text {
                "Paint on {layer}" => "Auf {layer} malen",
                "Paint mask of {layer}" => "Maske von {layer} malen",
                "Remove layer" => "Ebene entfernen",
                other => other,
            }
            .to_string()
        };
        assert_eq!(tr_history_with("Paint on Rust", translate), "Auf Rust malen");
        assert_eq!(tr_history_with("Paint mask of Paint on x", translate), "Maske von Paint on x malen");
        assert_eq!(tr_history_with("Remove layer", translate), "Ebene entfernen");
        assert_eq!(tr_history_with("Clone on Rust", translate), "Clone on Rust");
        assert_eq!(tr_history_with("Paint on {layer}", translate), "Auf {layer} malen");
    }

    const UNTRANSLATED: &[&str] = &["RGB", "RGBA"];

    #[test]
    fn lookup_formatting_and_fallbacks() {
        let _lock = LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        set_language("xx");
        assert_eq!(current().code, "en");
        assert_eq!(tr("Layers"), "Layers");
        assert_eq!(trf("Saved {path}", &[("path", &"a.tcraft")]), "Saved a.tcraft");
        assert_eq!(trf("{a} of {b} {c}", &[("a", &1), ("b", &2)]), "1 of 2 {c}");
        assert_eq!(trf("{unclosed", &[]), "{unclosed");
        for (locale, code) in
            [("de-AT", "de"), ("pt_PT.UTF-8", "pt-BR"), ("zh-Hant-TW", "zh-Hans"), ("ja", "ja"), ("ko_KR", "ko"), ("ar-EG", "ar"), ("nl-NL", "en"), ("", "en")]
        {
            assert_eq!(language_for_locale(locale), code, "{locale}");
        }
        set_language("de");
        assert_eq!(current().code, if available("de") { "de" } else { "en" });
        assert_eq!(tr("not a key"), "not a key");
        set_language("en");
    }

    #[test]
    fn saved_language_codes_and_spanish_lookup() {
        let _lock = LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for language in &LANGUAGES {
            set_language(language.code);
            assert_eq!(current().code, if available(language.code) { language.code } else { "en" });
        }
        const MISSING: &str = "missing test label";
        set_language("es");
        let spanish = tr("Layers");
        let missing = tr(MISSING);
        set_language("en");
        let english = tr("Layers");
        set_language("es");
        let spanish_again = tr("Layers");
        set_language("en");

        assert_eq!(spanish, if available("es") { "Capas" } else { "Layers" });
        assert_eq!(missing, MISSING);
        assert_eq!(english, "Layers");
        assert_eq!(spanish_again, spanish);
    }
}
