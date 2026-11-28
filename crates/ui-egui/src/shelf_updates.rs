//! The shelf's refresh popup: read the Assets panel and the project's images again every few minutes, or now by hand

use egui::{Context, Id, Ui, vec2};
use egui_phosphor::regular as ic;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::TextureCraftApp;
use crate::i18n::{n_, tr};
use crate::property_sheet::{NumberField, PropertySheet, SIDE_PADDING_POINTS, SectionScope, in_scope};
use crate::widgets::icon_dropdown_button;

/// Least and most minutes between automatic updates
const MIN_MINUTES: u32 = 1;
const MAX_MINUTES: u32 = 24 * 60;

/// Air between the popup and the toolbar under it
const POPUP_GAP: f32 = 1.0;

/// Air between the two update buttons and under the last one
const BUTTON_GAP: f32 = 6.0;

/// Room right of a row's switch for its i
const INFO_COLUMN: f32 = 28.0;

/// How soon a run that waited for a stroke tries again
const STROKE_RETRY: std::time::Duration = std::time::Duration::from_millis(250);

/// A drag of 20 points moves the minutes by one
const MINUTES_PER_DRAG_POINT: f32 = 0.05;

/// How much shorter a manual update button is than a sheet row, so neighbouring buttons don't touch
const BUTTON_INSET: f32 = 4.0;

const SKIP_MISMATCH: &str = n_("Skip assets when their parameters mismatch");

/// Which automatic updates are on and how often they run
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShelfUpdates {
    /// Read the user's saved assets again
    pub assets: bool,
    /// Read the project's images again from the files they were imported from when a file changed
    pub resources: bool,
    pub minutes: u32,
}

impl Default for ShelfUpdates {
    fn default() -> Self {
        ShelfUpdates { assets: false, resources: false, minutes: MIN_MINUTES }
    }
}

/// When the automatic updates last ran, on egui's clock
fn last_run_id() -> Id {
    Id::unique("shelf_updates_last_run")
}

/// Runs the automatic updates once their minutes have passed since the last run, and wakes the app for the next one
pub(crate) fn tick(app: &mut TextureCraftApp, ctx: &Context) {
    let updates = app.ui.shelf_updates;
    if !updates.assets && !updates.resources {
        ctx.data_mut(|data| data.remove::<f64>(last_run_id()));
        return;
    }

    // An update during a stroke would land in the stroke's undo step: it waits for the stroke to end
    if app.session.stroke_active() {
        ctx.request_repaint_after(STROKE_RETRY);
        return;
    }

    let now = ctx.input(|input| input.time);
    let every = f64::from(updates.minutes.clamp(MIN_MINUTES, MAX_MINUTES)) * 60.0;
    let last = ctx.data_mut(|data| *data.get_temp_mut_or(last_run_id(), now));
    if now - last < every {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(every - (now - last)));
        return;
    }

    ctx.data_mut(|data| data.insert_temp(last_run_id(), now));
    if updates.assets {
        refresh_assets(app);
    }
    if updates.resources {
        update_resources(app, ctx);
    }
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(every));
}

/// Read the user's saved assets from disk again and redraw their thumbnails
pub(crate) fn refresh_assets(app: &mut TextureCraftApp) {
    app.library = None;

    #[cfg(not(target_arch = "wasm32"))]
    for warning in app.session.load_brush_presets(crate::user_dir("brushes")) {
        app.toast(warning, true);
    }

    app.thumbs.forget_saved_files();
}

/// Reads the project's changed image files again; a file that is gone or can't be read says why once, until it reads again
fn update_resources(app: &mut TextureCraftApp, ctx: &Context) {
    let Some(status) = app.run("resource.status", json!({})) else { return };
    let rows = status["resources"].as_array().cloned().unwrap_or_default();
    let mut failures: Vec<(String, String)> =
        rows.iter().filter(|row| row["status"] == "missing").map(|row| (text(&row["name"]), text(&row["error"]))).collect();
    if status["outdated"].as_u64().is_some_and(|outdated| outdated > 0)
        && let Some(result) = app.run("resource.update", json!({}))
    {
        failures.extend(result["failed"].as_array().into_iter().flatten().map(|failure| (text(&failure["name"]), text(&failure["error"]))));
    }

    let reported_id = reported_failures_id(app.session.project_id);
    let mut reported: Vec<String> = ctx.data(|data| data.get_temp(reported_id)).unwrap_or_default();
    reported.retain(|name| failures.iter().any(|(failed, _)| failed == name));
    for (name, error) in failures {
        if !reported.contains(&name) {
            app.toast(format!("{name}: {error}"), true);
            reported.push(name);
        }
    }
    ctx.data_mut(|data| data.insert_temp(reported_id, reported));
}

/// Reads the files of `names` (every changed image when None) again through the Resources Updater, which says what happened
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn update_from_files(app: &mut TextureCraftApp, names: Option<Vec<String>>) {
    crate::resources_updater::update(app, names);
}

/// The browser reads no files: the command's refusal says so
#[cfg(target_arch = "wasm32")]
pub(crate) fn update_from_files(app: &mut TextureCraftApp, names: Option<Vec<String>>) {
    app.run("resource.update", names.map_or_else(|| json!({}), |names| json!({ "names": names })));
}

/// A string field of a command's result, empty when it has none
fn text(value: &serde_json::Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// The files of project `project_id` whose failure to read was already shown (ctx temp data)
fn reported_failures_id(project_id: u64) -> Id {
    Id::unique(("shelf_updates_reported_failures", project_id))
}

pub(super) fn forget_reported_failures(ctx: &Context, project_id: u64) {
    ctx.data_mut(|data| data.remove::<Vec<String>>(reported_failures_id(project_id)));
}

/// The refresh button with its popup of update settings, opening upward from the toolbar `panel` with its left edge on the panel's
pub(crate) fn button(app: &mut TextureCraftApp, ui: &mut Ui, size: f32, panel: egui::Rect) {
    let label = tr("Refresh");
    let response = icon_dropdown_button(ui, ic::ARROWS_CLOCKWISE, label, false, size).on_hover_text(label);
    let width = popup_width(ui, panel.width());
    egui::Popup::from_toggle_button_response(&response)
        .anchor(panel)
        .align(egui::RectAlign::TOP_START)
        .gap(POPUP_GAP)
        .width(width)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(width);
            popup(app, ui);
        });
}

/// The panel's width, or wider to fit the longest row
fn popup_width(ui: &Ui, panel_width: f32) -> f32 {
    let font = egui::FontId::proportional(crate::property_sheet::TEXT_SIZE);
    let longest = ui.painter().layout_no_wrap(tr(SKIP_MISMATCH).to_owned(), font, egui::Color32::WHITE).size().x;
    panel_width.max(longest + crate::property_sheet::TOGGLE_ROOM + INFO_COLUMN + 2.0 * SIDE_PADDING_POINTS)
}

fn popup(app: &mut TextureCraftApp, ui: &mut Ui) {
    let updates = &mut app.ui.shelf_updates;
    let full = ui.available_rect_before_wrap();
    let rows = full.with_max_x(full.right() - INFO_COLUMN);
    in_scope(ui, SectionScope::Shelf, |ui| {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rows), |ui| {
            PropertySheet::show(ui, "shelf_updates", |sheet| {
                sheet.section(tr("Automatic updates"), false);
                sheet.toggle(tr("Assets panel"), &mut updates.assets);
                sheet.beside_info(INFO_COLUMN, tr("Reads your saved assets from disk again every few minutes, so new and changed ones show here."));
                sheet.toggle(tr("Resources used in project"), &mut updates.resources);
                sheet.beside_info(
                    INFO_COLUMN,
                    tr("Every few minutes, reads again the images whose files changed since this project imported them. Each update is one undo step."),
                );

                let mut minutes = [NumberField {
                    tag: "",
                    tag_color: None,
                    value: updates.minutes as f32,
                    speed: MINUTES_PER_DRAG_POINT,
                    decimals: 0,
                    suffix: tr(" min"),
                }];
                if sheet.number_fields(tr("Update every"), &mut minutes, None, false).is_some() {
                    updates.minutes = (minutes[0].value.round() as u32).clamp(MIN_MINUTES, MAX_MINUTES);
                }
                sheet.beside_info(INFO_COLUMN, tr("How often the automatic updates above run."));

                sheet.section(tr("Advanced settings"), false);
                sheet.unavailable_toggle(tr(SKIP_MISMATCH));
                sheet.beside_info(
                    INFO_COLUMN,
                    tr("For assets with parameters: an update whose parameters were renamed or removed would change your texturing, so it is skipped. TextureCraft doesn't update assets with parameters from their files yet, so there is nothing to skip."),
                );

                sheet.section(tr("Manual"), false);
            });
        });
    });

    let button_size = vec2(full.width() - 2.0 * SIDE_PADDING_POINTS, crate::property_sheet::ROW_HEIGHT - BUTTON_INSET);
    let add_button = |ui: &mut Ui, label: &str| {
        let clicked = ui
            .horizontal(|ui| {
                ui.add_space(SIDE_PADDING_POINTS);
                ui.add_sized(button_size, egui::Button::new(label)).clicked()
            })
            .inner;
        ui.add_space(BUTTON_GAP);
        clicked
    };
    if add_button(ui, tr("Update Assets panel")) {
        refresh_assets(app);
    }
    if add_button(ui, tr("Update resources used in project")) {
        update_from_files(app, None);
    }
}
