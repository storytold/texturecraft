//! The shelf's side panes left of the grid, toggled from the toolbar: saved searches and the folder tree

use craft_ui::layout::{ResizeHandle, ResizeHandleStyle, SplitAxis, SplitLayout, SplitSize, split_rect};
use egui::{Rect, RichText, Ui, UiBuilder};
use egui_phosphor::regular as ic;
use serde::{Deserialize, Serialize};

use crate::TextureCraftApp;
use crate::i18n::{n_, tr};
use crate::saved_search::SavedSearch;
use crate::shelf::TABS;
use crate::theme::Tokens;
use crate::unavailable::NOT_AVAILABLE;
use crate::widgets::icon_button;

const COLUMN_FRACTION: f32 = 0.45;
const COLUMN_MIN_WIDTH_POINTS: f32 = 110.0;
const SPLITTER_HEIGHT_POINTS: f32 = 6.0;
const PANE_MIN_HEIGHT_POINTS: f32 = 40.0;
/// The reference's built-in saved searches after "All", shown greyed until they work
const BUILT_IN_SEARCHES: [&str; 3] = [n_("Favorites"), n_("3D Assets"), n_("Flattened layers")];
/// The reference's font libraries after the asset libraries, shown greyed until they work
const FONT_LIBRARIES: [&str; 2] = [n_("System fonts"), n_("User fonts")];
/// The pane header's caption size in points
const HEADER_FONT_SIZE_POINTS: f32 = 10.0;

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
    (panes.saved_searches || panes.filter_by_path).then(|| (width * COLUMN_FRACTION).max(COLUMN_MIN_WIDTH_POINTS).min(width))
}

/// Draws the open panes and a divider when both are visible.
pub(crate) fn column(app: &mut TextureCraftApp, ui: &mut Ui, paths: &[Vec<String>], applicable: &[Vec<String>]) {
    // Keep long names on one line in the narrow column
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);

    let rect = ui.available_rect_before_wrap();
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }

    let show_saved_searches = app.ui.shelf_panes.saved_searches;
    let show_path_filter = app.ui.shelf_panes.filter_by_path;
    let (search_rect, path_rect) = if show_saved_searches && show_path_filter {
        let Some(layout) = splitter(app, ui, rect) else { return };
        (layout.first, layout.second)
    } else {
        (rect, rect)
    };

    if show_saved_searches && search_rect.height() > 0.0 {
        let mut pane = ui.new_child(UiBuilder::new().id_salt("saved_searches").max_rect(search_rect));
        pane.set_clip_rect(search_rect.intersect(ui.clip_rect()));
        saved_searches(app, &mut pane);
    }
    if show_path_filter && path_rect.height() > 0.0 {
        let mut pane = ui.new_child(UiBuilder::new().id_salt("path_filter").max_rect(path_rect));
        pane.set_clip_rect(path_rect.intersect(ui.clip_rect()));
        filter_by_path(app, &mut pane, paths, applicable);
    }

    ui.allocate_rect(rect, egui::Sense::hover());
}

/// Resize the two panes with the shared pointer, keyboard and accessibility control.
fn splitter(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect) -> Option<SplitLayout> {
    // Callers can set the public share directly without loading settings
    if !app.ui.shelf_panes_split.is_finite() {
        app.ui.shelf_panes_split = crate::state::default_shelf_panes_split();
    }

    // Leave room for a header and a complete row when both minima fit
    let spacing = ui.spacing();
    let minimum_points = PANE_MIN_HEIGHT_POINTS.max(2.0 * spacing.interact_size.y + spacing.item_spacing.y);
    let axis = SplitAxis::Vertical;
    let size = SplitSize::Ratio(app.ui.shelf_panes_split);
    let mut layout = split_rect(rect, axis, size, SPLITTER_HEIGHT_POINTS, minimum_points)?;
    let available_points = layout.available();

    let visuals = ui.visuals();
    let line_stroke = visuals.widgets.noninteractive.bg_stroke;
    if available_points > 2.0 * minimum_points {
        let colors =
            ResizeHandleStyle { hovered: visuals.widgets.hovered.bg_fill, dragged: visuals.widgets.active.bg_fill, focused: visuals.selection.bg_fill };
        let resize = ResizeHandle::new(ui.scope_id().with("shelf_panes_splitter"), axis, layout.divider, tr("Resize Assets filter panes"), colors)
            .hit_margin(0.0)
            .value(layout.first_size(), minimum_points..=available_points - minimum_points)
            .show(ui);
        if resize.delta != 0.0
            && let Some(SplitSize::Ratio(share)) = layout.resized(size, resize.delta, minimum_points)
        {
            app.ui.shelf_panes_split = share;
            layout = split_rect(rect, axis, SplitSize::Ratio(share), SPLITTER_HEIGHT_POINTS, minimum_points)?;
        }
    }

    ui.painter().hline(layout.divider.x_range(), layout.divider.center().y, line_stroke);
    Some(layout)
}

/// "All libraries", then each library's folders as a tree; a click shows that folder
fn filter_by_path(app: &mut TextureCraftApp, ui: &mut Ui, paths: &[Vec<String>], applicable: &[Vec<String>]) {
    let size = ui.spacing().interact_size.y;
    header(ui, tr("Filter by path"), |ui| {
        let hiding = app.ui.shelf_panes.hide_non_applicable;
        if icon_button(ui, ic::EYE_SLASH, tr("Hide non-applicable folders"), hiding, size).clicked() {
            app.ui.shelf_panes.hide_non_applicable = !hiding;
        }
    });

    let hiding = app.ui.shelf_panes.hide_non_applicable;
    let shown_paths = if hiding { applicable } else { paths };
    let mut picked = None;

    // Use the real pane height so the last row can scroll clear of the clip edge
    egui::ScrollArea::vertical().id_salt("shelf_path_tree").min_scrolled_height(0.0).auto_shrink([false, true]).show(ui, |ui| {
        if ui.selectable_label(app.ui.shelf_path.is_empty(), tr("All libraries")).clicked() {
            picked = Some(Vec::new());
        }
        folders(ui, shown_paths, &[], &app.ui.shelf_path, &mut picked);

        // Font libraries contain no matching assets while they are unavailable
        if !hiding {
            for library in FONT_LIBRARIES {
                ui.add_enabled(false, egui::Button::selectable(false, tr(library))).on_disabled_hover_text(tr(NOT_AVAILABLE));
            }
        }
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
        let has_children = paths.iter().any(|candidate| candidate.len() > path.len() && candidate.starts_with(&path));

        if !has_children {
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
        ui.label(RichText::new(title.to_uppercase()).size(HEADER_FONT_SIZE_POINTS).color(tokens.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), button);
    });
}

/// "All", then the saved queries; the header's button saves the current one, a right-click deletes one
fn saved_searches(app: &mut TextureCraftApp, ui: &mut Ui) {
    let size = ui.spacing().interact_size.y;
    header(ui, tr("Saved searches"), |ui| {
        if icon_button(ui, ic::FLOPPY_DISK, tr("Save the current search"), false, size).clicked() {
            let query = current_query(app);
            if !app.ui.shelf_saved_searches.contains(&query) {
                app.ui.shelf_saved_searches.push(query);
            }
        }
    });

    let all_selected = app.ui.shelf_filter.is_empty() && app.ui.shelf_path.is_empty() && app.ui.shelf_tab == crate::shelf_tabs::NO_TYPE;
    let first_saved_row = 1 + BUILT_IN_SEARCHES.len();
    let row_height = ui.spacing().interact_size.y;
    let total_rows = first_saved_row + app.ui.shelf_saved_searches.len();
    let mut deleted_index = None;

    // The fixed entries and saved searches share one scroll area
    egui::ScrollArea::vertical().id_salt("shelf_saved_searches").min_scrolled_height(0.0).auto_shrink([false, true]).show_rows(
        ui,
        row_height,
        total_rows,
        |ui, visible_rows| {
            for row in visible_rows {
                if row == 0 {
                    if ui.selectable_label(all_selected, tr("All")).clicked() {
                        crate::shelf_toolbar::reset_query(app);
                    }
                    continue;
                }

                if let Some(name) = BUILT_IN_SEARCHES.get(row - 1) {
                    ui.add_enabled(false, egui::Button::selectable(false, tr(name))).on_disabled_hover_text(tr(NOT_AVAILABLE));
                    continue;
                }

                let saved_index = row - first_saved_row;
                let Some(saved) = app.ui.shelf_saved_searches.get(saved_index) else { continue };

                let selected = saved.filter == app.ui.shelf_filter && saved.tab == app.ui.shelf_tab && saved.path == app.ui.shelf_path;
                let response = ui.selectable_label(selected, &saved.name);
                if response.clicked() {
                    (app.ui.shelf_filter, app.ui.shelf_tab, app.ui.shelf_path) = (saved.filter.clone(), saved.tab, saved.path.clone());
                }

                response.context_menu(|ui| {
                    if ui.button(tr("Delete")).clicked() {
                        deleted_index = Some(saved_index);
                        ui.close();
                    }
                });
            }
        },
    );

    if let Some(index) = deleted_index {
        app.ui.shelf_saved_searches.remove(index);
    }
}

/// The shelf's query, named after its search text, else its folder, else its tab
fn current_query(app: &TextureCraftApp) -> SavedSearch {
    let filter = &app.ui.shelf_filter;
    let tab = app.ui.shelf_tab;
    let path = &app.ui.shelf_path;
    let name = if !filter.is_empty() {
        filter.as_str()
    } else if let Some(folder) = path.last() {
        crate::shelf_folders::name(path.len() - 1, folder)
    } else {
        TABS.get(tab).map_or_else(|| tr("All"), |(_, _, label)| tr(label))
    };

    SavedSearch { name: name.to_owned(), filter: filter.clone(), tab, path: path.clone() }
}
