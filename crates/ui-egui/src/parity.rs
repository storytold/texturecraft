//! Menu parity with the target menu tree: which catalog items are live. `cargo test -p texturecraft-ui-egui
//! parity` checks the floor; with `TC_UPDATE_PARITY=1` it rewrites `docs/parity.md`.

use crate::menus::{CATALOG, implemented};

/// Live menu items must never drop below this (raise it when parity rises).
pub const FLOOR: usize = 51;

/// RFC-0052 hand marks for catalog ids that aren't engine commands: `(id, headless, reach, why)`.
///
/// `headless` is `no` or `partial`; `reach` is `window` (needs a desktop window, dialog or pointer),
/// `os` (needs an OS or online service the engine doesn't wrap) or `engine-gap` (could be an engine
/// command, isn't one yet). `yes` is never written here: an item is `yes` only when its id is an
/// engine command (`texturecraft_engine::find`), the registry `texturecraft-cli` and the MCP server
/// load. A `ui.*` id is live in the desktop app only. An unmarked id is `unknown`.
pub const HEADLESS: &[(&str, &str, &str, &str)] = &[
    ("ui.newProject", "partial", "window", "the dialog is desktop-only; `project.new` creates the project"),
    ("ui.saveAsTemplate", "partial", "window", "the dialog is desktop-only; `project.template.save` saves it"),
    ("ui.export", "partial", "window", "the dialog is desktop-only; `file.exportTextures` exports"),
    ("ui.saveReduced", "no", "engine-gap", "a smaller save could be a command"),
    ("ui.projectConfig", "partial", "window", "the dialog is desktop-only; `project.rename` sets the project name"),
    ("ui.recent.clear", "no", "window", "the desktop recent-projects list"),
    ("ui.quit", "no", "window", "quits the desktop app"),
    ("ui.settings", "no", "window", "the desktop settings window"),
    ("ui.mode.bake", "no", "window", "switches the desktop workspace; `bake.run` bakes"),
    ("ui.mode.paint", "no", "window", "switches the desktop workspace"),
    ("ui.mode.render", "no", "window", "switches the desktop workspace"),
    ("ui.layout.save.painting", "no", "window", "desktop window layout"),
    ("ui.layout.save.rendering", "no", "window", "desktop window layout"),
    ("ui.layout.save.baking", "no", "window", "desktop window layout"),
    ("ui.layout.load", "no", "window", "desktop window layout"),
    ("ui.hideUi", "no", "window", "desktop window layout"),
    ("ui.resetUi", "no", "window", "desktop window layout"),
    ("ui.help.releaseNotes", "no", "os", "opens a web page in the browser"),
    ("ui.help.docs", "no", "os", "opens a web page in the browser"),
    ("ui.help.discord", "no", "os", "opens a web page in the browser"),
    ("ui.help.bug", "no", "os", "opens a web page in the browser"),
    ("ui.help.checkForUpdates", "no", "os", "an online update check"),
    ("ui.help.shortcuts", "no", "window", "a desktop window"),
    ("ui.help.showLog", "no", "window", "a desktop window"),
    ("ui.help.exportLog", "no", "window", "saves the desktop app's log"),
    ("ui.about", "no", "window", "a desktop window"),
];

/// Prefixes whose every id opens or arranges a desktop panel (`no`, `window`).
const WINDOW_PREFIXES: [&str; 2] = ["ui.view.", "ui.toolbar."];

/// An item's RFC-0052 `(headless, reach, why)`: `yes` for an engine command, else its mark, else
/// `unknown`.
pub fn headless_of(id: &str, in_engine: bool) -> (&'static str, Option<&'static str>, &'static str) {
    if in_engine {
        return ("yes", None, "");
    }
    if let Some(m) = HEADLESS.iter().find(|m| m.0 == id) {
        return (m.1, Some(m.2), m.3);
    }
    if WINDOW_PREFIXES.iter().any(|p| id.starts_with(p)) {
        return ("no", Some("window"), "a desktop panel or toolbar");
    }
    ("unknown", None, "")
}

/// Problems with `marks` against `ids` (`(id, is an engine command)`): bad values, repeats, ids
/// not in the catalog, and marks on engine commands (they would hide a `yes`).
pub fn headless_errors(marks: &[(&str, &str, &str, &str)], ids: &[(&str, bool)]) -> Vec<String> {
    let mut errs = Vec::new();
    for (i, (id, value, reach, _)) in marks.iter().enumerate() {
        if !matches!(*value, "no" | "partial") {
            errs.push(format!("headless mark `{id}`: `{value}` is not `no` or `partial` (`yes` comes from the registry)"));
        }
        if !matches!(*reach, "window" | "os" | "engine-gap") {
            errs.push(format!("headless mark `{id}`: reach `{reach}` is not window, os or engine-gap"));
        }
        if marks[..i].iter().any(|m| m.0 == *id) {
            errs.push(format!("headless mark `{id}` is written twice"));
        }
        match ids.iter().find(|x| x.0 == *id) {
            None => errs.push(format!("headless mark `{id}` is not a catalog id")),
            Some((_, true)) => errs.push(format!("headless mark `{id}` is stale: it is an engine command, so it is `yes`")),
            Some(_) => {}
        }
    }
    errs
}

/// (live, total) over the catalog, separators excluded.
pub fn counts() -> (usize, usize) {
    let items: Vec<_> = CATALOG.iter().filter(|i| i.label != "-").collect();
    (items.iter().filter(|i| implemented(i.id)).count(), items.len())
}

/// The Markdown report.
pub fn report() -> String {
    let (live, total) = counts();
    let mut out = String::new();
    out.push_str("# Menu parity\n\nGenerated by `TC_UPDATE_PARITY=1 cargo test -p texturecraft-ui-egui parity`. ");
    out.push_str("Each row is a menu item of the target menu layout with TextureCraft's command id; ");
    out.push_str("**live** means the id is implemented. Feature parity beyond menus is tracked in [`ROADMAP.md`](../ROADMAP.md).\n\n");
    out.push_str(&format!("**{live} of {total} menu items live ({:.0}%).**\n\n", live as f32 * 100.0 / total.max(1) as f32));
    let values: Vec<_> = CATALOG.iter().filter(|i| i.label != "-").map(|i| headless_of(i.id, texturecraft_engine::find(i.id).is_some())).collect();
    let count = |v: &str| values.iter().filter(|h| h.0 == v).count();
    let no = |r: &str| values.iter().filter(|h| h.0 == "no" && h.1 == Some(r)).count();
    out.push_str(&format!(
        "**Headless ([RFC-0052](https://github.com/storytold/craft-contrib/blob/main/rfcs/0052-headless-reach.md)): {} yes, {} no (window {}, os {}, engine-gap {}), {} partial, {} unknown.** ",
        count("yes"),
        count("no"),
        no("window"),
        no("os"),
        no("engine-gap"),
        count("partial"),
        count("unknown")
    ));
    out.push_str("An item is `headless: yes` when its id is an engine command, so `texturecraft-cli` and the MCP server reach it. ");
    out.push_str("A `ui.*` id is live in the desktop app only; its value is the hand mark in `HEADLESS` (`crates/ui-egui/src/parity.rs`).\n\n");
    for top in crate::menus::TOP {
        out.push_str(&format!("## {top}\n\n| Item | Id | Shortcut | Status | Headless |\n|---|---|---|---|---|\n"));
        for i in CATALOG.iter().filter(|i| i.menu == top && i.label != "-") {
            let label = match i.sub {
                Some(s) => format!("{s} ▸ {}", i.label),
                None => i.label.to_string(),
            };
            let status = if implemented(i.id) { "live" } else { "missing" };
            let (value, reach, why) = headless_of(i.id, texturecraft_engine::find(i.id).is_some());
            let headless = reach.map_or_else(|| value.to_string(), |r| format!("{value}, reach: {r} ({why})"));
            out.push_str(&format!("| {label} | `{}` | {} | {status} | {headless} |\n", i.id, i.shortcut.unwrap_or("")));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_marks_are_current() {
        let ids: Vec<(&str, bool)> = CATALOG.iter().filter(|i| i.label != "-").map(|i| (i.id, texturecraft_engine::find(i.id).is_some())).collect();
        let errs = headless_errors(HEADLESS, &ids);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn a_desktop_only_id_is_live_but_not_headless() {
        assert!(implemented("ui.export"), "live in the desktop app");
        assert_eq!(headless_of("ui.export", false).0, "partial");
        assert_eq!(headless_of("ui.view.layers", false), ("no", Some("window"), "a desktop panel or toolbar"));
        assert_eq!(headless_of("file.save", true).0, "yes");
        assert_eq!(headless_of("ui.unheardOf", false).0, "unknown");
        let r = report();
        assert!(r.contains("| `file.save` | Ctrl+S | live | yes |"), "{r}");
        let (live, _) = counts();
        let yes = CATALOG.iter().filter(|i| i.label != "-" && texturecraft_engine::find(i.id).is_some()).count();
        assert!(yes < live, "the ui.* ids are live but not headless");
        assert!(r.contains(&format!("{yes} yes,")), "{r}");
    }

    #[test]
    fn bad_headless_marks_are_refused() {
        let ids = [("a.b", true), ("c.d", false)];
        let marks = [("c.d", "yes", "os", ""), ("c.d", "no", "desk", ""), ("a.b", "no", "os", ""), ("z.z", "no", "os", "")];
        let errs = headless_errors(&marks, &ids);
        for want in ["is not `no` or `partial`", "reach `desk`", "written twice", "`a.b` is stale", "`z.z` is not a catalog id"] {
            assert!(errs.iter().any(|e| e.contains(want)), "{want}: {errs:?}");
        }
    }

    #[test]
    fn parity_floor_and_report() {
        let (live, total) = counts();
        assert!(live >= FLOOR, "menu parity dropped: {live} < {FLOOR}");
        assert!(total >= live);
        if std::env::var("TC_UPDATE_PARITY").is_ok() {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/parity.md");
            std::fs::write(path, report()).unwrap();
        }
    }
}
