//! The shelf's side panes left of the grid, toggled from the toolbar: saved searches and the folder tree

use egui::{RichText, Ui};
use egui_phosphor::regular as ic;
use serde::{Deserialize, Serialize};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::saved_search::SavedSearch;
use crate::shelf::TABS;
use crate::theme::Tokens;
use crate::widgets::icon_button;

/// The panes' column width as a fraction of the panel, and its least width in points
const COLUMN_FRACTION: f32 = 0.45;
const COLUMN_MIN_WIDTH: f32 = 110.0;
/// The pane header's caption size in points
const HEADER_SIZE: f32 = 10.0;

/// Which side panes are open
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShelfPanes {
    pub saved_searches: bool,
    pub filter_by_path: bool,
    /// Filter by path lists only the folders with assets matching the search and tab
    pub hide_non_applicable: bool,
}

/// The toolbar's buttons that open and close the panes
pub(crate) fn toggles(app: &mut TextureCraftApp, ui: &mut Ui, size: f32) {
    let panes = &mut app.ui.shelf_panes;
    if icon_button(ui, ic::BOOKMARKS_SIMPLE, tr("Saved searches"), panes.saved_searches, size).clicked() {
        panes.saved_searches = !panes.saved_searches;
    }
    if icon_button(ui, ic::TREE_STRUCTURE, tr("Filter by path"), panes.filter_by_path, size).clicked() {
        panes.filter_by_path = !panes.filter_by_path;
    }
}

/// The width of the panes' column in a panel `width` wide, or None while no pane is open
pub(crate) fn column_width(app: &TextureCraftApp, width: f32) -> Option<f32> {
    let panes = &app.ui.shelf_panes;
    (panes.saved_searches || panes.filter_by_path).then(|| (width * COLUMN_FRACTION).max(COLUMN_MIN_WIDTH).min(width))
}

/// The open panes, top to bottom; the tree shows `paths`, or `applicable` while it hides folders with nothing matching
pub(crate) fn column(app: &mut TextureCraftApp, ui: &mut Ui, paths: &[Vec<String>], applicable: &[Vec<String>]) {
    // Names past the narrow column end in "…" rather than breaking mid-word
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
    if app.ui.shelf_panes.saved_searches {
        saved_searches(app, ui);
    }
    if app.ui.shelf_panes.filter_by_path {
        if app.ui.shelf_panes.saved_searches {
            ui.separator();
        }
        let shown = if app.ui.shelf_panes.hide_non_applicable { applicable } else { paths };
        filter_by_path(app, ui, shown);
    }
}

/// "All libraries", then each library's folders as a tree; a click shows that folder
fn filter_by_path(app: &mut TextureCraftApp, ui: &mut Ui, paths: &[Vec<String>]) {
    let size = ui.spacing().interact_size.y;
    header(ui, tr("Filter by path"), |ui| {
        let hiding = app.ui.shelf_panes.hide_non_applicable;
        if icon_button(ui, ic::EYE_SLASH, tr("Hide non-applicable folders"), hiding, size).clicked() {
            app.ui.shelf_panes.hide_non_applicable = !hiding;
        }
    });

    let mut picked = None;
    egui::ScrollArea::vertical().id_salt("shelf_path_tree").auto_shrink([false, true]).show(ui, |ui| {
        if ui.selectable_label(app.ui.shelf_path.is_empty(), tr("All libraries")).clicked() {
            picked = Some(Vec::new());
        }
        folders(ui, paths, &[], &app.ui.shelf_path, &mut picked);
    });
    if let Some(path) = picked {
        app.ui.shelf_path = path;
    }
}

/// The folders below `prefix`: one with folders inside opens and closes, a click on a name picks it
fn folders(ui: &mut Ui, paths: &[Vec<String>], prefix: &[String], current: &[String], picked: &mut Option<Vec<String>>) {
    for child in crate::shelf_folders::children(paths, prefix) {
        let path: Vec<String> = prefix.iter().cloned().chain([child.clone()]).collect();
        let name = crate::shelf_folders::name(prefix.len(), &child);
        let selected = current == path.as_slice();
        if crate::shelf_folders::children(paths, &path).is_empty() {
            ui.indent(("shelf_path_leaf", &path), |ui| {
                if ui.selectable_label(selected, name).clicked() {
                    *picked = Some(path.clone());
                }
            });
            continue;
        }

        let id = ui.make_persistent_id(("shelf_path", &path));
        let open_by_default = current.starts_with(&path);
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open_by_default)
            .show_header(ui, |ui| {
                if ui.selectable_label(selected, name).clicked() {
                    *picked = Some(path.clone());
                }
            })
            .body(|ui| folders(ui, paths, &path, current, picked));
    }
}

/// A pane's caption with a button at its right
fn header(ui: &mut Ui, title: &str, button: impl FnOnce(&mut Ui)) {
    let tokens = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(RichText::new(title.to_uppercase()).size(HEADER_SIZE).color(tokens.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), button);
    });
}

/// "All", then the saved queries; the header's button saves the current one, a right-click deletes one
fn saved_searches(app: &mut TextureCraftApp, ui: &mut Ui) {
    let size = ui.spacing().interact_size.y;
    let current = current_query(app);
    header(ui, tr("Saved searches"), |ui| {
        if icon_button(ui, ic::FLOPPY_DISK, tr("Save the current search"), false, size).clicked() && !app.ui.shelf_saved_searches.contains(&current) {
            app.ui.shelf_saved_searches.push(current.clone());
        }
    });

    let everything = current.filter.is_empty() && current.path.is_empty() && current.tab == crate::shelf_tabs::NO_TYPE;
    if ui.selectable_label(everything, tr("All")).clicked() {
        crate::shelf_toolbar::reset_query(app);
    }

    let mut deleted = None;
    for (i, saved) in app.ui.shelf_saved_searches.iter().enumerate() {
        let same = saved.filter == current.filter && saved.tab == current.tab && saved.path == current.path;
        let response = ui.selectable_label(same, &saved.name);
        if response.clicked() {
            (app.ui.shelf_filter, app.ui.shelf_tab, app.ui.shelf_path) = (saved.filter.clone(), saved.tab, saved.path.clone());
        }
        response.context_menu(|ui| {
            if ui.button(tr("Delete")).clicked() {
                deleted = Some(i);
                ui.close();
            }
        });
    }
    if let Some(i) = deleted {
        app.ui.shelf_saved_searches.remove(i);
    }
}

/// The shelf's query, named after its search text, else its folder, else its tab
fn current_query(app: &TextureCraftApp) -> SavedSearch {
    let (filter, tab, path) = (app.ui.shelf_filter.clone(), app.ui.shelf_tab, app.ui.shelf_path.clone());
    let name = if !filter.is_empty() {
        filter.clone()
    } else if let Some(folder) = path.last() {
        crate::shelf_folders::name(path.len() - 1, folder).to_string()
    } else {
        TABS.get(tab).map_or(tr("All"), |(_, _, label)| tr(label)).to_string()
    };
    SavedSearch { name, filter, tab, path }
}
