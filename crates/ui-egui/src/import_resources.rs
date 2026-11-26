//! File > Import resources...: a window to collect files, set what each is for, and import them
//! together. Images become project resources; materials load as smart materials and MaterialX
//! documents as fill layers.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Key, Modifiers};
use serde_json::json;

use crate::TextureCraftApp;
use crate::file_dialogs::{Bound, FileDialog};
use crate::i18n::{tr, trf};
use crate::state::{ImportResources, ImportRow, ResourceUsage};
use crate::web_files::{IMAGE_EXTS, MATERIAL_EXTS};
use crate::widgets::primary_button;
use crate::windows::MATERIALX_EXT;

/// Width of the window
const WINDOW_WIDTH_POINTS: f32 = 640.0;
/// Height of the file list
const LIST_HEIGHT_POINTS: f32 = 300.0;
// Initial space for the title, file controls, destination and footer
const WINDOW_CHROME_HEIGHT_POINTS: f32 = 200.0;
/// Width of a row's prefix field and usage dropdown
const PREFIX_WIDTH_POINTS: f32 = 160.0;
const USAGE_WIDTH_POINTS: f32 = 110.0;
/// Width of the destination dropdown
const TARGET_WIDTH_POINTS: f32 = 190.0;
/// Keep space for the file name in a narrow row
const MIN_NAME_WIDTH_POINTS: f32 = 40.0;
const CONTROL_GAP_POINTS: f32 = 4.0;
const SECTION_GAP_POINTS: f32 = 8.0;

/// Opens the window with an empty list (an open window stays as it is)
pub(crate) fn open(app: &mut TextureCraftApp) {
    static NEXT_OPENING_ID: AtomicU64 = AtomicU64::new(1);

    app.ui.windows.project_config = None;
    if app.ui.windows.import_resources.is_none() {
        app.ui.windows.import_resources = Some(ImportResources {
            project: app.session.project_id,
            opening_id: NEXT_OPENING_ID.fetch_add(1, Ordering::Relaxed),
            ..ImportResources::default()
        });
    }
}

/// The keys of the open window: Escape closes it, Enter imports unless a text field or button had
/// the keyboard. An open dropdown takes both first.
pub(crate) fn keys_in(app: &mut TextureCraftApp, input: &mut egui::InputState, nothing_focused: bool, popup_open: bool) {
    if popup_open {
        return;
    }

    let escape = input.consume_key(Modifiers::NONE, Key::Escape);
    let enter = input.consume_key(Modifiers::NONE, Key::Enter);
    if escape {
        app.ui.windows.import_resources = None;
    } else if enter
        && nothing_focused
        && let Some(window) = app.ui.windows.import_resources.as_mut()
    {
        window.confirm = true;
    }
}

fn extension_of(row: &ImportRow) -> Option<&str> {
    row.path.extension().and_then(|extension| extension.to_str())
}

fn is_smart_material_extension(extension: &str) -> bool {
    MATERIAL_EXTS.iter().any(|material_extension| extension.eq_ignore_ascii_case(material_extension))
}

/// Whether the file is a material (a smart material or a MaterialX document, so it has no usage or prefix)
fn is_material(row: &ImportRow) -> bool {
    extension_of(row).is_some_and(|extension| extension.eq_ignore_ascii_case(MATERIALX_EXT) || is_smart_material_extension(extension))
}

fn file_name(row: &ImportRow) -> String {
    row.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| row.path.display().to_string())
}

/// A click on row `id` of the `visible` rows: plain selects only it, Ctrl toggles it, Shift
/// selects from the anchor to it
fn click_row(window: &mut ImportResources, visible: &[u64], id: u64, modifiers: Modifiers) {
    let ctrl = modifiers.command || modifiers.ctrl;
    let from = window.anchor.and_then(|anchor| visible.iter().position(|v| *v == anchor));
    let to = visible.iter().position(|v| *v == id);

    match (modifiers.shift, from, to) {
        (true, Some(from), Some(to)) => {
            let range: BTreeSet<u64> = visible[from.min(to)..=from.max(to)].iter().copied().collect();
            if ctrl {
                window.selected.extend(range);
            } else {
                window.selected = range;
            }
        }
        _ if ctrl => {
            if !window.selected.remove(&id) {
                window.selected.insert(id);
            }
            window.anchor = Some(id);
        }
        _ => {
            window.selected = BTreeSet::from([id]);
            window.anchor = Some(id);
        }
    }
}

pub(crate) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(mut window) = app.ui.windows.import_resources.take() else { return };

    // Files chosen for another project cannot be imported here
    if window.project != app.session.project_id {
        return;
    }

    // A filter whose rows are gone shows everything again
    if window.filter.is_some_and(|usage| !window.rows.iter().any(|row| row.usage == usage)) {
        window.filter = None;
    }

    let mut open = true;
    let (mut import, mut cancel, mut add) = (std::mem::take(&mut window.confirm), false, false);
    let mut remove = false;

    crate::windows::dialog(ctx, "import_resources", tr("Import Resources"), &mut open)
        .default_size(egui::vec2(WINDOW_WIDTH_POINTS, LIST_HEIGHT_POINTS + WINDOW_CHROME_HEIGHT_POINTS))
        .show(ctx, |ui| {
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        cancel = ui.button(tr("Cancel")).clicked();
                        import |= ui.add_enabled(app.session.project_open && !window.rows.is_empty(), primary_button(ui, tr("Import"))).clicked();
                    });
                });
                ui.add_space(CONTROL_GAP_POINTS);
                if !app.session.project_open {
                    ui.label(tr("Open a project to import resources. Importing into the asset library is not available yet."));
                }

                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::ComboBox::from_id_salt("import_target").width(TARGET_WIDTH_POINTS).selected_text(tr("Open project")).show_ui(ui, |ui| {
                            if ui.selectable_label(true, tr("Open project")).clicked() {
                                ui.close();
                            }

                            ui.add_enabled(false, egui::Button::selectable(false, tr("Asset library"))).on_disabled_hover_text(tr("Not available yet"));
                        });
                        ui.label(tr("Destination"));
                    });
                });
                ui.add_space(SECTION_GAP_POINTS);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(tr("Choose files and assign their roles"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_enabled_ui(!window.selected.is_empty(), |ui| {
                                remove = ui.button(tr("Remove selected resources")).clicked();
                            });
                            add = ui.button(tr("Add resources")).clicked();
                        });
                    });
                    ui.add_space(CONTROL_GAP_POINTS);

                    let all_label = trf("All files ({count})", &[("count", &window.rows.len())]);
                    let filter_text = match window.filter {
                        None => all_label.clone(),
                        Some(usage) => {
                            trf("{usage} ({count})", &[("usage", &usage.name()), ("count", &window.rows.iter().filter(|row| row.usage == usage).count())])
                        }
                    };
                    egui::ComboBox::from_id_salt("import_filter").width(ui.available_width()).selected_text(filter_text).show_ui(ui, |ui| {
                        if ui.selectable_label(window.filter.is_none(), &all_label).clicked() {
                            window.filter = None;
                        }

                        // Only usages some row has, so the list is never empty by filter
                        for usage in ResourceUsage::ALL {
                            let count = window.rows.iter().filter(|row| row.usage == usage).count();
                            if count > 0 && ui.selectable_label(window.filter == Some(usage), format!("{} ({count})", usage.name())).clicked() {
                                window.filter = Some(usage);
                            }
                        }
                    });
                    ui.add_space(CONTROL_GAP_POINTS);

                    let filter = window.filter;
                    let visible = |row: &ImportRow| filter.is_none_or(|usage| row.usage == usage);
                    let modifiers = ui.input(|i| i.modifiers);
                    let mut clicked_row = None;
                    let mut selected_usage = None;

                    let list_height = ui.available_height().max(0.0);
                    egui::ScrollArea::both().min_scrolled_width(0.0).min_scrolled_height(0.0).max_height(list_height).auto_shrink([false, false]).show(
                        ui,
                        |ui| {
                            for row in window.rows.iter_mut().filter(|row| visible(row)) {
                                let material = is_material(row);
                                let selected = window.selected.contains(&row.id);

                                ui.horizontal(|ui| {
                                    let name_width = (ui.available_width() - PREFIX_WIDTH_POINTS - USAGE_WIDTH_POINTS - 2.0 * ui.spacing().item_spacing.x)
                                        .max(MIN_NAME_WIDTH_POINTS);
                                    let name =
                                        ui.add_sized([name_width, ui.spacing().interact_size.y], egui::Button::selectable(selected, file_name(row)).truncate());
                                    if name.on_hover_text(row.path.display().to_string()).clicked() {
                                        clicked_row = Some(row.id);
                                    }

                                    let prefix = ui.add_enabled(
                                        !material,
                                        egui::TextEdit::singleline(&mut row.prefix).hint_text(tr("Optional prefix/path...")).desired_width(PREFIX_WIDTH_POINTS),
                                    );
                                    if material {
                                        prefix.on_disabled_hover_text(tr("Materials keep their own name"));
                                    } else {
                                        prefix
                                            .on_hover_text(tr("Put in front of the resource's name. Characters other than letters, digits and _ . - become _"));
                                    }

                                    ui.add_enabled_ui(!material, |ui| {
                                        egui::ComboBox::from_id_salt(("import_usage", row.id))
                                            .width(USAGE_WIDTH_POINTS)
                                            .selected_text(row.usage.name())
                                            .show_ui(ui, |ui| {
                                                for usage in ResourceUsage::ALL {
                                                    let available = usage.available();
                                                    let item = ui.add_enabled(available, egui::Button::selectable(row.usage == usage, usage.name()));
                                                    if !available {
                                                        item.on_disabled_hover_text(tr("Not available yet"));
                                                    } else if item.clicked() {
                                                        if selected && window.selected.len() > 1 {
                                                            selected_usage = Some(usage);
                                                        } else {
                                                            row.usage = usage;
                                                        }

                                                        ui.close();
                                                    }
                                                }
                                            });
                                    })
                                    .response
                                    .on_disabled_hover_text(tr("Materials need no usage"));
                                });
                            }
                        },
                    );

                    // Include selected rows hidden by the filter
                    if let Some(usage) = selected_usage {
                        for row in &mut window.rows {
                            if window.selected.contains(&row.id) && !is_material(row) {
                                row.usage = usage;
                            }
                        }
                    }

                    if let Some(id) = clicked_row {
                        let visible: Vec<u64> =
                            if modifiers.shift { window.rows.iter().filter(|row| visible(row)).map(|row| row.id).collect() } else { Vec::new() };
                        click_row(&mut window, &visible, id, modifiers);
                    }
                });
            });
        });

    if cancel || !open {
        return;
    }

    if remove {
        window.rows.retain(|row| !window.selected.contains(&row.id));
        window.selected.clear();
        window.anchor = None;
    }

    if add {
        let mut extensions: Vec<&str> = IMAGE_EXTS.to_vec();
        extensions.extend_from_slice(MATERIAL_EXTS);
        extensions.push(MATERIALX_EXT);
        let opening_id = window.opening_id;
        app.ask_file(FileDialog::open_many().filter(tr("Resources"), &extensions), Bound::Project, move |app, paths| {
            let Some(stored) = app.ui.windows.import_resources.as_mut().filter(|stored| stored.opening_id == opening_id) else { return };

            for path in paths {
                if stored.rows.iter().any(|row| row.path == path) {
                    continue;
                }

                stored.rows.push(ImportRow { id: stored.next_row, path, prefix: String::new(), usage: ResourceUsage::Undefined });
                stored.next_row += 1;
            }
        });
    }

    if import && app.session.project_open && !window.rows.is_empty() {
        window.rows.retain(|row| !import_row(app, row));
        if window.rows.is_empty() {
            app.ui.dock.focus(crate::docking::Panel::Assets);
            return;
        }

        window.selected.clear();
        window.anchor = None;
    }

    // File picker answers run between frames after the dialog state is restored
    app.ui.windows.import_resources = Some(window);
}

/// Returns true once the resource has been imported or a linked material cook has been queued.
fn import_row(app: &mut TextureCraftApp, row: &ImportRow) -> bool {
    let Some(path) = row.path.to_str() else {
        app.toast(trf("Could not import a path that is not UTF-8: {path}", &[("path", &row.path.display())]), true);
        return false;
    };

    if let Some(extension) = extension_of(row) {
        if extension.eq_ignore_ascii_case(MATERIALX_EXT) {
            return crate::windows::import_materialx(app, path);
        }

        if is_smart_material_extension(extension) {
            return app.run("material.load", json!({ "path": path })).is_some();
        }
    }

    let Some(result) = app.run("resource.import", json!({ "path": path, "prefix": row.prefix })) else { return false };

    let name = result["name"].as_str().unwrap_or_default();
    app.toast(trf("Imported {name}", &[("name", &name)]), false);

    match row.usage {
        ResourceUsage::Environment => crate::web_files::use_environment(app, name),
        ResourceUsage::Alpha => {
            if app.run("tool.brush", json!({ "alpha": { "image": name } })).is_some() {
                app.toast(trf("Brush alpha: {name}", &[("name", &name)]), false);
            }
        }
        ResourceUsage::Undefined | ResourceUsage::Texture | ResourceUsage::ColorLut | ResourceUsage::PostFx => {}
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: Modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    const SHIFT: Modifiers = Modifiers { shift: true, ..Modifiers::NONE };

    fn selected(window: &ImportResources) -> Vec<u64> {
        window.selected.iter().copied().collect()
    }

    #[test]
    fn a_plain_click_selects_one_row() {
        let mut window = ImportResources::default();
        click_row(&mut window, &[1, 2, 3], 1, Modifiers::NONE);
        click_row(&mut window, &[1, 2, 3], 3, Modifiers::NONE);

        assert_eq!(selected(&window), [3]);
    }

    #[test]
    fn ctrl_toggles_rows() {
        let mut window = ImportResources::default();
        click_row(&mut window, &[1, 2, 3], 1, CTRL);
        click_row(&mut window, &[1, 2, 3], 3, CTRL);
        assert_eq!(selected(&window), [1, 3]);

        click_row(&mut window, &[1, 2, 3], 1, CTRL);
        assert_eq!(selected(&window), [3]);
    }

    #[test]
    fn shift_selects_from_the_anchor_in_either_direction() {
        let mut window = ImportResources::default();
        click_row(&mut window, &[1, 2, 3, 4], 2, Modifiers::NONE);
        click_row(&mut window, &[1, 2, 3, 4], 4, SHIFT);
        assert_eq!(selected(&window), [2, 3, 4]);

        click_row(&mut window, &[1, 2, 3, 4], 1, SHIFT);
        assert_eq!(selected(&window), [1, 2]);
    }
}
