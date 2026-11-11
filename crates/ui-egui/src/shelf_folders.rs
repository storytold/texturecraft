//! The shelf's library folders ([`ShelfEntry::path`]) for the breadcrumb above the search and the Filter by path tree

use egui::{RichText, Ui};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::shelf_entry::{LIBRARIES, ShelfEntry};
use crate::theme::Tokens;
use crate::widgets::icon_button;

/// The breadcrumb arrows' size relative to an icon button
const ARROW_SCALE: f32 = 0.8;

/// The folder paths of `entries`, each a library id then folder names
pub(crate) fn paths<'a>(entries: impl IntoIterator<Item = &'a ShelfEntry>) -> Vec<Vec<String>> {
    let mut paths: Vec<Vec<String>> = entries.into_iter().map(|entry| entry.path().into_iter().map(str::to_string).collect()).collect();
    paths.sort();
    paths.dedup();
    paths
}

/// `paths` and every library, even one with nothing in it (the picker always lists them all)
pub(crate) fn with_every_library(mut paths: Vec<Vec<String>>) -> Vec<Vec<String>> {
    paths.extend(LIBRARIES.iter().map(|(id, _)| vec![id.to_string()]));
    paths.sort();
    paths.dedup();
    paths
}

/// The folders right below `prefix` (libraries in their usual order, folders by name)
pub(crate) fn children(paths: &[Vec<String>], prefix: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in paths.iter().filter(|path| path.len() > prefix.len() && path.starts_with(prefix)) {
        if let Some(next) = path.get(prefix.len())
            && !out.contains(next)
        {
            out.push(next.clone());
        }
    }
    if prefix.is_empty() {
        out.sort_by_key(|library| LIBRARIES.iter().position(|(id, _)| id == library));
    }
    out
}

/// A folder's shown name: libraries by their name, folders (type and category names) translated
pub(crate) fn name(level: usize, segment: &str) -> &str {
    match LIBRARIES.iter().find(|(id, _)| *id == segment) {
        Some((_, library)) if level == 0 => tr(library),
        _ => tr(segment),
    }
}

/// "› All libraries › …": each arrow lists the folders at its level, the names are plain text, a last arrow lists the folders inside
pub(crate) fn breadcrumb(app: &mut TextureCraftApp, ui: &mut Ui, paths: &[Vec<String>]) {
    let tokens = Tokens::get(ui.ctx());
    let size = ui.spacing().interact_size.y * ARROW_SCALE;
    let mut picked: Option<Vec<String>> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = ui.spacing().item_spacing.x * 0.5;
        let path = app.ui.shelf_path.clone();
        for level in 0..=path.len() {
            let options = children(paths, &path[..level]);
            if level > 0 && options.is_empty() {
                break;
            }

            let current = path.get(level);
            let response = icon_button(ui, egui_phosphor::regular::CARET_RIGHT, tr("Browse folders"), false, size);
            egui::Popup::menu(&response).show(|ui| {
                if level == 0 && ui.selectable_label(current.is_none(), tr("All libraries")).clicked() {
                    picked = Some(Vec::new());
                }
                for option in &options {
                    if ui.selectable_label(current == Some(option), name(level, option)).clicked() {
                        picked = Some(path[..level].iter().cloned().chain([option.clone()]).collect());
                    }
                }
            });

            let shown = match current {
                Some(segment) => name(level, segment),
                None if level == 0 => tr("All libraries"),
                None => continue,
            };
            ui.label(RichText::new(shown).strong().color(tokens.text_strong));
        }
    });
    if let Some(path) = picked {
        app.ui.shelf_path = path;
    }
}
