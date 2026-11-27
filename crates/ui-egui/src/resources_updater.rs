//! The Resources Updater: the project's image resources and whether the files they were imported
//! from changed on disk. Update reads a changed file again, Select new resource points a resource
//! at another file, and a notice offers to update when the app gets the focus back after files
//! changed. Everything runs through `resource.status`, `resource.update` and `resource.relink`.

mod row_request;

use std::collections::BTreeSet;

use serde_json::json;
use texturecraft_engine::{ResourceState, ResourceStatus};

use crate::TextureCraftApp;
use crate::file_dialogs::{Bound, FileDialog};
use crate::i18n::{tr, trf};
use crate::state::ResourcesUpdater;
use crate::theme::Tokens;
use crate::toast_action::ToastAction;
use crate::web_files::IMAGE_EXTS;
use row_request::RowRequest;

/// Width of the window
const WINDOW_WIDTH_POINTS: f32 = 640.0;
/// Height of the resource list
const LIST_HEIGHT_POINTS: f32 = 360.0;
/// Initial space for the header, tabs, filters and footer
const WINDOW_CHROME_HEIGHT_POINTS: f32 = 160.0;
/// Side of a row's thumbnail
const THUMBNAIL_POINTS: f32 = 40.0;
/// Width of the name filter field
const NAME_FILTER_WIDTH_POINTS: f32 = 160.0;
/// Width of the status dropdown
const STATUS_WIDTH_POINTS: f32 = 120.0;
/// Space inside a row's frame
const ROW_MARGIN: egui::Margin = egui::Margin::same(4);
/// Strength of the danger tint behind an outdated row (the status text says it too)
const OUTDATED_TINT: f32 = 0.18;
/// How long the "changed on disk" notice stays, in seconds: long enough to reach its button
const NOTICE_SECONDS: f64 = 12.0;
/// Time between two checks while the window is open, in seconds (each check looks at every file; an unchanged one is not read)
const RECHECK_SECONDS: f64 = 0.5;

/// Opens the window, or closes it when it is open (the shelf button)
pub(crate) fn toggle(app: &mut TextureCraftApp) {
    if app.ui.windows.resources_updater.take().is_none() {
        open(app);
    }
}

/// Opens the window with the resources checked now (an open window is checked again)
pub(crate) fn open(app: &mut TextureCraftApp) {
    let window = app.ui.windows.resources_updater.take().unwrap_or_default();
    app.ui.windows.resources_updater =
        Some(ResourcesUpdater { rows: app.session.resource_states(), project: app.session.project_id, checked_at: app.last_time, ..window });
}

/// Check once when another project is open (files may have changed while it was closed), and
/// forget what was announced for the last one
pub(crate) fn follow_project(app: &mut TextureCraftApp) {
    if !app.session.project_open || app.resources_project == Some(app.session.project_id) {
        return;
    }

    app.resources_project = Some(app.session.project_id);
    app.resources_notified.clear();
    check_files(app);
}

/// Check the files again (the app has the focus back, or a project opened) and, when other
/// resources than last time are outdated, show a notice whose button updates them
pub(crate) fn check_files(app: &mut TextureCraftApp) {
    if !app.session.project_open {
        return;
    }

    let rows = app.session.resource_states();
    let outdated: BTreeSet<String> = rows.iter().filter(|row| row.status == ResourceStatus::Outdated).map(|row| row.name.clone()).collect();
    if let Some(window) = app.ui.windows.resources_updater.as_mut() {
        window.rows = rows;
        window.checked_at = app.last_time;
    }

    if !outdated.is_empty() && outdated != app.resources_notified {
        let text = if outdated.len() == 1 {
            tr("1 resource changed on disk").to_string()
        } else {
            trf("{count} resources changed on disk", &[("count", &outdated.len())])
        };
        app.toast(text, false);
        if let Some(toast) = app.toasts.last_mut() {
            toast.until = app.last_time + NOTICE_SECONDS;
            toast.action = Some(ToastAction::UpdateResources { project: app.session.project_id });
        }
    }
    app.resources_notified = outdated;
}

/// Read the files of `names` again (every outdated resource when None), one undo step, and say
/// what happened
pub(crate) fn update(app: &mut TextureCraftApp, names: Option<Vec<String>>) {
    let params = match names {
        Some(names) => json!({ "names": names }),
        None => json!({}),
    };
    let Some(result) = app.run("resource.update", params) else {
        app.announce_latest_toast();
        return;
    };

    let updated = result["updated"].as_array().map_or(0, Vec::len);
    let failed: Vec<String> = result["failed"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|failure| format!("{}: {}", failure["name"].as_str().unwrap_or_default(), failure["error"].as_str().unwrap_or_default()))
        .collect();
    if failed.is_empty() {
        app.toast(trf("Updated {count} resources", &[("count", &updated)]), false);
    } else {
        app.toast(trf("Updated {count} resources; could not update {failed}", &[("count", &updated), ("failed", &failed.join("; "))]), true);
    }
    app.announce_latest_toast();

    // What was announced is handled; a later change is announced again
    app.resources_notified.clear();
}

/// Ask for another file for resource `name`; the resource takes its pixels and follows it
fn select_new_resource(app: &mut TextureCraftApp, name: String) {
    let dialog = FileDialog::open().filter(tr("Images"), IMAGE_EXTS);
    app.ask_file(dialog, Bound::Project, move |app, paths| {
        let Some(path) = paths.first() else { return };
        let Some(path) = path.to_str() else {
            app.toast(trf("Could not use a path that is not UTF-8: {path}", &[("path", &path.display())]), true);
            app.announce_latest_toast();
            return;
        };

        if app.run("resource.relink", json!({ "name": name, "path": path })).is_some() {
            app.toast(trf("{name} now reads {path}", &[("name", &name), ("path", &path)]), false);
        }
        app.announce_latest_toast();
    });
}

/// Show the folder holding `path` in the system's file manager
fn show_in_folder(app: &mut TextureCraftApp, path: &str) {
    let folder = std::path::Path::new(path).parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    if let Err(error) = app.file_dialogs.show_folder(&folder) {
        app.toast(trf("Could not show {folder}: {error}", &[("folder", &folder.display()), ("error", &error)]), true);
        app.announce_latest_toast();
    }
}

/// Whether `row` passes the window's status and name filters
fn shown(window: &ResourcesUpdater, row: &ResourceState) -> bool {
    let name_filter = window.name_filter.trim().to_lowercase();
    window.status_filter.is_none_or(|status| row.status == status) && (name_filter.is_empty() || row.name.to_lowercase().contains(&name_filter))
}

/// "(N resources, M outdated)"
fn summary(rows: &[ResourceState]) -> String {
    let outdated = rows.iter().filter(|row| row.status == ResourceStatus::Outdated).count();
    trf("({count} resources, {outdated} outdated)", &[("count", &rows.len()), ("outdated", &outdated)])
}

pub(crate) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(mut window) = app.ui.windows.resources_updater.take() else { return };

    // Drop the window state when its project closes or changes
    if window.project != app.session.project_id || !app.session.project_open {
        return;
    }

    // Files changed on disk and document changes (undo, a removed resource) show while the window stays open
    let wait = window.checked_at + RECHECK_SECONDS - app.last_time;
    if wait <= 0.0 {
        window.rows = app.session.resource_states();
        window.checked_at = app.last_time;
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(RECHECK_SECONDS));
    } else {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
    }

    // Only close on Escape when the window owned keyboard focus
    // Read raw input because egui consumes Escape when it clears focus
    let escape = ctx.input(|input| input.raw.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));
    if window.had_keyboard && escape && !egui::Popup::is_any_open(ctx) {
        return;
    }

    let tokens = Tokens::get(ctx);
    let mut window_open = true;
    let (mut refresh, mut update_all) = (false, false);
    let mut request = None;

    let shown = crate::windows::dialog(ctx, "resources_updater", tr("Resources Updater"), &mut window_open)
        .default_size(egui::vec2(WINDOW_WIDTH_POINTS, LIST_HEIGHT_POINTS + WINDOW_CHROME_HEIGHT_POINTS))
        .show(ctx, |ui| {
            ui.label(format!("{} {}", tr("Project :"), app.session.doc.project.name));

            let tab = ui.add(egui::Button::selectable(true, tr("Resources")));
            tab.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, true, tr("Resources")));
            ui.separator();

            ui.horizontal(|ui| {
                let status_label = ui.label(tr("Status:"));
                let selected = window.status_filter.map_or(tr("All"), |status| tr(status.label()));
                egui::ComboBox::from_id_salt("resources_updater_status")
                    .width(STATUS_WIDTH_POINTS)
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut window.status_filter, None, tr("All"));
                        for status in ResourceStatus::ALL {
                            ui.selectable_value(&mut window.status_filter, Some(status), tr(status.label()));
                        }
                    })
                    .response
                    .labelled_by(status_label.id);

                let name_label = ui.label(tr("Name filter:"));
                ui.add(egui::TextEdit::singleline(&mut window.name_filter).desired_width(NAME_FILTER_WIDTH_POINTS)).labelled_by(name_label.id);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(tr("Bottom")).clicked() {
                        window.jump = Some(egui::Align::Max);
                    }
                    if ui.button(tr("Top")).clicked() {
                        window.jump = Some(egui::Align::Min);
                    }
                });
            });
            ui.add_space(4.0);

            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    let refresh_button = ui.button(tr("Refresh"));
                    refresh_button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Refresh resources")));
                    refresh = refresh_button.clicked();
                    let count = ui.label(summary(&window.rows));
                    ui.ctx().accesskit_node_builder(count.id, |node| node.set_live(egui::accesskit::Live::Polite));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let outdated = window.rows.iter().any(|row| row.status == ResourceStatus::Outdated);
                        let label = egui::RichText::new(tr("Update All")).color(tokens.text_strong);
                        let action = egui::Button::new(label).fill(tokens.accent_dim).stroke(egui::Stroke::new(1.0, tokens.text_dim));
                        let button = ui.add_enabled(outdated, action);
                        update_all = button.on_disabled_hover_text(tr("No resource is outdated")).clicked();
                    });
                });
                ui.separator();

                let list_height = ui.available_height().max(0.0);
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    egui::ScrollArea::vertical().max_height(list_height).auto_shrink([false, false]).show(ui, |ui| {
                        let jump = window.jump.take();
                        if jump == Some(egui::Align::Min) {
                            ui.scroll_to_cursor(Some(egui::Align::Min));
                        }

                        let visible: Vec<&ResourceState> = window.rows.iter().filter(|row| shown(&window, row)).collect();
                        if visible.is_empty() {
                            ui.label(tr("No resources match the filters"));
                        }
                        for (index, row) in visible.into_iter().enumerate() {
                            // Keep keyboard focus with the resource when rows move
                            let row_scope = egui::UiBuilder::new().scope_id(ui.scope_id().with(("resources_updater_row", &row.name)));
                            let asked = ui.scope_builder(row_scope, |ui| row_ui(app, ui, &tokens, index + 1, row)).inner;
                            if let Some(asked) = asked {
                                request = Some(asked);
                            }
                        }

                        if jump == Some(egui::Align::Max) {
                            ui.scroll_to_cursor(Some(egui::Align::Max));
                        }
                    });
                });
            });
        });

    let focused = ctx.memory(|memory| memory.focused()).and_then(|id| ctx.read_response(id));
    window.had_keyboard = shown.as_ref().zip(focused.as_ref()).is_some_and(|(shown, focused)| focused.layer_id == shown.response.layer_id);
    if !window_open {
        return;
    }

    app.ui.windows.resources_updater = Some(window);

    // Its own changes and Refresh show at once
    let check_now = refresh || update_all || matches!(request, Some(RowRequest::Update(_)));
    if update_all {
        update(app, None);
    }
    match request {
        Some(RowRequest::Update(name)) => update(app, Some(vec![name])),
        Some(RowRequest::SelectNew(name)) => select_new_resource(app, name),
        Some(RowRequest::ShowFolder(path)) => show_in_folder(app, &path),
        None => {}
    }

    if check_now {
        open(app);
    }
}

/// One resource: its thumbnail, number, name, file and status, and its buttons. Outdated rows are
/// tinted, and their status says so in words too.
fn row_ui(app: &mut TextureCraftApp, ui: &mut egui::Ui, tokens: &Tokens, number: usize, row: &ResourceState) -> Option<RowRequest> {
    let outdated = row.status == ResourceStatus::Outdated;
    let (fill, stroke) = if outdated {
        (tokens.danger.gamma_multiply(OUTDATED_TINT), egui::Stroke::new(1.0, tokens.danger))
    } else {
        (egui::Color32::TRANSPARENT, egui::Stroke::NONE)
    };
    let mut request = None;

    egui::Frame::NONE.fill(fill).stroke(stroke).inner_margin(ROW_MARGIN).corner_radius(crate::theme::CARD_RADIUS).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            match app.session.images.get(&row.name) {
                Some(image) => {
                    let (rect, preview) = ui.allocate_exact_size(egui::Vec2::splat(THUMBNAIL_POINTS), egui::Sense::hover());
                    preview.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &row.name));
                    if ui.is_rect_visible(rect) {
                        let texture = app.thumbs.image(ui.ctx(), &row.name, image);
                        egui::Image::new(&texture).paint_at(ui, rect);
                    }
                }

                // Reserve thumbnail space for images that failed to decode
                None => ui.add_space(THUMBNAIL_POINTS),
            }
            ui.label(number.to_string());

            // Reserve space for the buttons before truncating the labels
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let update = ui.add_enabled(outdated, egui::Button::new(tr("Update")));
                let update_name = format!("{} {}", tr("Update"), row.name);
                update.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, outdated, &update_name));
                if update.clicked() {
                    request = Some(RowRequest::Update(row.name.clone()));
                }

                // Only images can be relinked to disk
                let material = row.status == ResourceStatus::Material;
                if !material {
                    let select = ui.button(tr("Select new resource"));
                    let select_name = format!("{} {}", tr("Select new resource"), row.name);
                    select.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &select_name));
                    if select.clicked() {
                        request = Some(RowRequest::SelectNew(row.name.clone()));
                    }
                }

                if let Some(path) = &row.path {
                    let folder = ui.button(tr("Show in folder"));
                    let folder_name = format!("{} {}", tr("Show in folder"), row.name);
                    folder.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &folder_name));
                    if folder.clicked() {
                        request = Some(RowRequest::ShowFolder(path.clone()));
                    }
                }

                ui.vertical(|ui| {
                    ui.add(egui::Label::new(format!("{} {}", tr("Name :"), row.name)).truncate());
                    ui.label(format!("{} {}", tr("Shelf :"), tr("project")));
                    let file = row.path.as_deref().unwrap_or(tr("No source file"));
                    ui.add(egui::Label::new(format!("{} {}", tr("File :"), file)).truncate()).on_hover_text(file);
                    let status = ui.label(tr(row.status.label()));
                    let detail = row
                        .error
                        .as_deref()
                        .or_else(|| material.then(|| tr("This MaterialCraft package is stored in the project. Its cooked maps are separate image resources.")));
                    if let Some(detail) = detail {
                        ui.ctx().accesskit_node_builder(status.id, |node| node.set_description(detail));
                        status.on_hover_text(detail);
                    }
                });
            });
        });
    });

    request
}

#[cfg(test)]
mod tests {
    use std::sync::PoisonError;

    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use texturecraft_engine::Session;

    use super::*;

    fn write_png(path: &std::path::Path, size: u32) {
        image::RgbaImage::from_pixel(size, size, image::Rgba([10, 200, 30, 255])).save(path).unwrap();
    }

    fn notices(app: &TextureCraftApp) -> Vec<ToastAction> {
        app.toasts.iter().filter_map(|toast| toast.action).collect()
    }

    #[test]
    fn a_changed_file_is_announced_once_and_the_notice_updates_it() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notice.png");
        write_png(&path, 4);
        let session = Session::new_project("Test", texturecraft_engine::geom::primitives::cube(1.0), "builtin:cube", None, 128).unwrap();
        let mut harness = Harness::builder().with_context_setup(crate::framework::install_headless).build_eframe(|cc| TextureCraftApp::new(cc, session));
        harness.state_mut().ui.prefs.language = "en".into();
        harness.run_steps(2);
        let app = harness.state_mut();
        let name = app.run("resource.import", json!({ "path": path.to_str().unwrap() })).unwrap()["name"].as_str().unwrap().to_string();

        check_files(app);
        assert!(notices(app).is_empty(), "nothing changed yet");

        write_png(&path, 8);
        check_files(app);
        check_files(app);
        let announced = notices(app);
        assert_eq!(announced, [ToastAction::UpdateResources { project: app.session.project_id }], "the same change is announced once");

        // A notice about another project does nothing
        ToastAction::UpdateResources { project: app.session.project_id + 1 }.run(app);
        assert_eq!(app.session.images[&name].width, 4);

        harness.run_steps(2);
        let message = harness.get_by_label("1 resource changed on disk");
        assert_eq!(message.accesskit_node().live(), egui::accesskit::Live::Polite, "the notice is announced away from focus");
        harness.get_by_role_and_label(egui::Role::Button, "Update resources").focus();
        harness.run_steps(2);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);

        let app = harness.state();
        assert_eq!(app.session.images[&name].width, 8, "the notice's button updated the resource");
        assert_eq!(app.session.history_labels().last().map(String::as_str), Some("Update resources"));
    }
}
