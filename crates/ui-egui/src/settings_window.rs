//! Edit > Settings: the General, Shortcuts and Libraries pages chosen in the left column. Changes
//! wait in a working copy until OK applies and keeps them; Cancel or closing the window drops them.

use egui::{Color32, Response, RichText, Ui, vec2};
use egui_phosphor::regular as ic;

use crate::TextureCraftApp;
#[cfg(not(target_arch = "wasm32"))]
use crate::file_dialogs::{Bound, FileDialog};
use crate::i18n::{LANGUAGES, SYSTEM_LANGUAGE, n_, tr, trf};
use crate::state::{CAMERA_SPEED_RANGE, LibraryLocation, Prefs};
use crate::theme::{Accent, Brightness, Tokens, caption};
use crate::unavailable::NOT_AVAILABLE;
use crate::widgets::icon_button;

mod settings_draft;
#[cfg(not(target_arch = "wasm32"))]
mod settings_folder;
mod settings_page;

pub use settings_draft::SettingsDraft;
#[cfg(not(target_arch = "wasm32"))]
use settings_folder::SettingsFolder;
pub use settings_page::SettingsPage;

/// The Libraries page's note on when locations can change.
pub const PROJECT_OPEN_NOTE: &str = n_("Library locations can't be added/removed when a project is opened.");
/// The hint of the default library's "-".
pub const DEFAULT_LIBRARY_NOTE: &str = n_("Cannot remove the default library.");
/// The Baking options' note when there is no GPU to trace rays on.
pub const GPU_RAYTRACING_UNAVAILABLE: &str = n_("GPU raytracing not available for your hardware.");
/// The footer's text while two shortcuts share a key combination.
pub const SHORTCUT_ERRORS: &str = n_("One or several errors have been found in Shortcuts settings");

/// The width of the left column of a row and of a page's controls.
const LABEL_WIDTH: f32 = 176.0;
const CONTROL_WIDTH: f32 = 340.0;
const PAGE_WIDTH: f32 = 560.0;
const MAX_PAGE_HEIGHT_POINTS: f32 = 520.0;
// Reserve room for the title bar, window margins, separator and footer buttons
const WINDOW_CHROME_HEIGHT_POINTS: f32 = 112.0;
// Leave room for the column gap, layout spacing and scroll bar
const CONTENT_WIDTH_ALLOWANCE_POINTS: f32 = 32.0;
/// The page buttons of the left column.
const PAGE_BUTTON: egui::Vec2 = vec2(84.0, 80.0);
const PAGE_ICON_SIZE: f32 = 44.0;

/// The Settings window, while `windows.settings` is set.
pub(crate) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
    if !app.ui.windows.settings {
        app.ui.windows.settings_draft = None;
        crate::keymap::stop_settings_capture(ctx);
        return;
    }

    let mut draft = app.ui.windows.settings_draft.take().unwrap_or_else(|| SettingsDraft::of(app));
    let mut open = true;
    let (mut ok, mut cancel) = (false, false);
    #[cfg(not(target_arch = "wasm32"))]
    let mut pick = None;
    let project_open = app.session.project_open;
    let gpu = app.render_state.as_ref().map(|render_state| render_state.adapter.get_info().name);

    // A language whose script no font draws is listed by its English name and disabled
    let drawable = LANGUAGES.each_ref().map(|l| crate::fonts::Script::of_language(l.code).is_none_or(|s| app.fonts.covers(s)));
    #[cfg(not(target_arch = "wasm32"))]
    let resources = settings_resources(ctx);

    let viewport = ctx.content_rect();
    let page_height = (viewport.height() - WINDOW_CHROME_HEIGHT_POINTS).clamp(0.0, MAX_PAGE_HEIGHT_POINTS);
    let frame_width = egui::Frame::window(&ctx.global_style()).total_margin().sum().x;
    let content_width = (viewport.width() - frame_width).clamp(0.0, PAGE_BUTTON.x + PAGE_WIDTH + CONTENT_WIDTH_ALLOWANCE_POINTS);

    // The shortcut checker follows the working copy, so the page icon and the footer know of a shared shortcut
    crate::keymap::follow(&mut app.settings_bindings, &draft.keymap, ctx);
    let shortcut_conflict = crate::keymap::has_conflict(&app.settings_bindings);

    crate::windows::window("settings", tr("Settings"), &mut open).resizable(false).show(ctx, |ui| {
        // Keep the footer inside the viewport when the fixed-width controls need to scroll
        ui.set_max_width(content_width);
        ui.horizontal_top(|ui| {
            egui::ScrollArea::vertical().id_salt("settings_navigation").min_scrolled_height(0.0).max_height(page_height).show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.set_width(PAGE_BUTTON.x);
                    for page in SettingsPage::ALL {
                        let warn = page == SettingsPage::Shortcuts && shortcut_conflict;
                        if page_button(ui, page, draft.page == page, warn).clicked() {
                            draft.page = page;
                        }
                        ui.add_space(6.0);
                    }
                });
            });
            ui.add_space(8.0);
            egui::ScrollArea::both()
                .id_salt(("settings_page", draft.page as u8))
                .auto_shrink([false, false])
                .min_scrolled_width(0.0)
                .min_scrolled_height(page_height)
                .max_height(page_height)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(PAGE_WIDTH);
                        match draft.page {
                            SettingsPage::General => {
                                general(
                                    ui,
                                    &mut draft.prefs,
                                    &drawable,
                                    gpu.as_deref(),
                                    #[cfg(not(target_arch = "wasm32"))]
                                    resources,
                                    #[cfg(not(target_arch = "wasm32"))]
                                    &mut pick,
                                );
                            }
                            SettingsPage::Shortcuts => {
                                crate::keymap::actions_page(ui, &mut draft.keymap, &app.bindings, &mut app.settings_bindings);
                            }
                            SettingsPage::Libraries => {
                                libraries(
                                    ui,
                                    &mut draft,
                                    project_open,
                                    #[cfg(not(target_arch = "wasm32"))]
                                    &mut pick,
                                );
                            }
                        }
                    });
                });
        });
        ui.separator();

        // The page has brought the checker up to date, so OK follows this frame's edits
        let conflict = crate::keymap::has_conflict(&app.settings_bindings);
        ui.horizontal(|ui| {
            if conflict {
                let danger = Tokens::get(ui.ctx()).danger;
                ui.label(RichText::new(format!("{} {}", ic::WARNING, tr(SHORTCUT_ERRORS))).color(danger));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                cancel = ui.add(egui::Button::new(tr("Cancel")).min_size(vec2(100.0, 0.0))).clicked();
                ok = ui.add_enabled(!conflict, egui::Button::new(tr("OK")).min_size(vec2(100.0, 0.0))).clicked();
            });
        });
    });

    app.ui.windows.settings_page = draft.page;
    if draft.page != SettingsPage::Shortcuts || ok || cancel || !open {
        crate::keymap::stop_settings_capture(ctx);
    }
    if ok {
        apply(app, draft);
    } else if !(cancel || !open) {
        app.ui.windows.settings_draft = Some(draft);
    }
    app.ui.windows.settings = !(ok || cancel || !open);

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(pick) = pick {
        ask_folder(app, pick);
    }
}

/// OK: the working copy becomes the settings in use.
fn apply(app: &mut TextureCraftApp, draft: SettingsDraft) {
    let libraries_changed = draft.prefs.libraries != app.ui.prefs.libraries;
    let previous_autosave_minutes = app.ui.prefs.autosave_minutes;

    app.ui.prefs = draft.prefs;
    app.reschedule_autosave_if_changed(previous_autosave_minutes);

    let ids: std::collections::BTreeSet<&String> = draft.keymap.keys().chain(draft.keymap_at_open.keys()).collect();
    for id in ids.into_iter().filter(|id| draft.keymap.get(*id) != draft.keymap_at_open.get(*id)) {
        match draft.keymap.get(id) {
            Some(shortcut) => app.ui.keymap.insert(id.clone(), shortcut.clone()),
            None => app.ui.keymap.remove(id),
        };
    }
    app.ui.prefs.apply_resource_limits();

    // The shelf reads the saved materials of the locations again
    if libraries_changed {
        app.library = None;
    }
}

/// The folder dialog of the data folder or of the Libraries page's Path, answered into the working copy.
#[cfg(not(target_arch = "wasm32"))]
fn ask_folder(app: &mut TextureCraftApp, pick: SettingsFolder) {
    let Some(draft) = &app.ui.windows.settings_draft else { return };

    let bound = Bound::Settings(draft.id);
    match pick {
        SettingsFolder::DataFolder => {
            let start = crate::data_folder(&draft.prefs.data_folder);
            app.ask_file(FileDialog::folder().directory(start), bound, |app, paths| {
                if let (Some(dir), Some(draft)) = (paths.first(), app.ui.windows.settings_draft.as_mut()) {
                    draft.prefs.data_folder = dir.display().to_string();
                }
            });
        }
        SettingsFolder::LibraryPath => {
            let start = Some(std::path::PathBuf::from(draft.library_path.trim())).filter(|path| path.is_dir()).unwrap_or_else(crate::data_dir);
            app.ask_file(FileDialog::folder().directory(start), bound, |app, paths| {
                if let (Some(dir), Some(draft)) = (paths.first(), app.ui.windows.settings_draft.as_mut()) {
                    draft.library_path = dir.display().to_string();
                }
            });
        }
    }
}

/// A page of the left column: a big icon over its caption, the caption highlighted when chosen.
fn page_button(ui: &mut Ui, page: SettingsPage, selected: bool, warn: bool) -> Response {
    let tk = Tokens::get(ui.ctx());
    let label = tr(page.label());
    let (rect, response) = ui.allocate_exact_size(PAGE_BUTTON, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    let icon_color = if selected {
        tk.accent
    } else if response.hovered() {
        tk.text_strong
    } else {
        tk.text
    };
    let painter = ui.painter();
    painter.text(rect.center_top() + vec2(0.0, 4.0), egui::Align2::CENTER_TOP, page.icon(), egui::FontId::proportional(PAGE_ICON_SIZE), icon_color);
    let text_color = if selected { tk.accent } else { tk.text };
    let text = painter.text(rect.center_bottom() - vec2(0.0, 6.0), egui::Align2::CENTER_BOTTOM, label, egui::FontId::proportional(12.5), text_color);
    if warn {
        // The warning overlay on the icon
        painter.text(rect.center_top() + vec2(PAGE_ICON_SIZE * 0.5, 4.0), egui::Align2::CENTER_TOP, ic::WARNING, egui::FontId::proportional(18.0), tk.danger);
    }
    if selected {
        painter.rect_stroke(text.expand2(vec2(3.0, 1.0)), 2.0, egui::Stroke::new(1.0, tk.accent), egui::StrokeKind::Outside);
    }
    response
}

/// `response` of a disabled control says the function is not available yet.
fn not_available(response: Response) -> Response {
    response.on_disabled_hover_text(tr(NOT_AVAILABLE))
}

/// A row of a section: `label` on the left, `control` beside it.
fn row<R>(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui, egui::Id) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let label = ui
            .vertical(|ui| {
                ui.set_width(LABEL_WIDTH);
                ui.add(egui::Label::new(label).wrap())
            })
            .inner;
        control(ui, label.id)
    })
    .inner
}

/// A section caption with the space above it.
fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(caption(tr(title)));
    ui.add_space(2.0);
}

/// A disabled check box in the right column of a row labelled `label`.
fn disabled_check(ui: &mut Ui, label: egui::Id, mut checked: bool) {
    not_available(ui.add_enabled(false, egui::Checkbox::without_text(&mut checked)).labelled_by(label));
}

/// A disabled check box with its text.
fn disabled_text_check(ui: &mut Ui, text: &str, mut checked: bool) {
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        not_available(ui.add_enabled(false, egui::Checkbox::new(&mut checked, tr(text))));
    });
}

/// A slider row: `label` and the value above the track, whole steps shown without decimals.
fn speed_slider(ui: &mut Ui, label: &str, value: &mut f32, width: f32) -> Response {
    ui.vertical(|ui| {
        ui.set_width(width);
        let decimals = if value.fract() == 0.0 { 0 } else { 1 };
        let response = crate::widgets::slider(ui, label, value, CAMERA_SPEED_RANGE, decimals);

        // Tenths: the shown value is the value used
        *value = ((*value * 10.0).round() / 10.0).clamp(*CAMERA_SPEED_RANGE.start(), *CAMERA_SPEED_RANGE.end());
        response
    })
    .inner
}

/// Whether the Baking options say there is no GPU to trace rays on: no adapter, or the browser.
pub fn gpu_note_shown(adapter: Option<&str>) -> bool {
    adapter.is_none() || cfg!(target_arch = "wasm32")
}

/// The General page.
fn general(
    ui: &mut Ui,
    p: &mut Prefs,
    drawable: &[bool],
    gpu: Option<&str>,
    #[cfg(not(target_arch = "wasm32"))] (available_cores, available_ram_bytes): (u32, u64),
    #[cfg(not(target_arch = "wasm32"))] pick: &mut Option<SettingsFolder>,
) {
    let tk = Tokens::get(ui.ctx());
    section(ui, n_("INTERFACE OPTIONS"));
    row(ui, tr("Language"), |ui, label| {
        let selected = if p.language == SYSTEM_LANGUAGE {
            tr("Default (System Language)")
        } else {
            LANGUAGES.iter().find(|l| l.code == p.language).map_or(tr("Default (System Language)"), |l| l.name)
        };
        egui::ComboBox::from_id_salt("settings_language")
            .width(CONTROL_WIDTH)
            .height(320.0)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut p.language, SYSTEM_LANGUAGE.to_owned(), tr("Default (System Language)"));
                for (l, ok) in LANGUAGES.iter().zip(drawable).filter(|(l, _)| crate::i18n::available(l.code)) {
                    let item = egui::Button::selectable(l.code == p.language, if *ok { l.name } else { l.english });
                    if ui.add_enabled(*ok, item).on_disabled_hover_text(tr("No font on this system can show this language")).clicked() {
                        p.language = l.code.to_owned();
                    }
                }
            })
            .response
            .labelled_by(label);
    });
    row(ui, tr("Show keyboard helper"), |ui, label| disabled_check(ui, label, true));
    row(ui, tr("Show world axes"), |ui, label| disabled_check(ui, label, true));
    row(ui, tr("Background color"), |ui, _| {
        for (index, side, name) in [(0, "top", "Top background"), (1, "bottom", "Bottom background")] {
            let id = format!("settings_background_{side}");
            crate::widgets::color_swatch(ui, &id, &mut p.background[index], 38.0);

            // The reference shows no tooltip here; the name is only for screen readers
            let label = trf("Pick the {overlay} Color from the Screen", &[("overlay", &name)]);
            crate::screen_color_pick::eyedropper_without_tooltip(ui, ui.make_persistent_id(&id), &mut p.background[index], &label, 18.0);
            ui.add_space(10.0);
        }
    });
    row(ui, tr("Only display the selected material when painting"), |ui, label| disabled_check(ui, label, false));
    row(ui, tr("Viewport Scaling"), |ui, label| {
        ui.add_enabled_ui(false, |ui| {
            not_available(
                egui::ComboBox::from_id_salt("settings_viewport_scaling").width(CONTROL_WIDTH).selected_text(tr("Auto")).show_ui(ui, |_| {}).response,
            )
            .labelled_by(label);
        });
    });

    section(ui, n_("LAYER STACK OPTIONS"));
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.add_enabled_ui(false, |ui| {
            let mut uv_scale = 1.0;
            ui.vertical(|ui| {
                ui.set_width(PAGE_WIDTH - 24.0);
                not_available(crate::widgets::slider(ui, tr("Default UV scale for materials"), &mut uv_scale, 0.0..=10.0, 0));
            });
        });
    });
    disabled_text_check(ui, n_("Use simplified thumbnails (improves performance)"), true);

    section(ui, n_("CAMERA OPTIONS"));
    let speed_width = CONTROL_WIDTH + 12.0;
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        speed_slider(ui, tr("Rotation speed"), &mut p.rotation_speed, speed_width);
    });
    for (label, speed, inverse) in [(tr("Zoom speed"), &mut p.zoom_speed, &mut p.zoom_inverse), (tr("Wheel speed"), &mut p.wheel_speed, &mut p.wheel_inverse)] {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            speed_slider(ui, label, speed, speed_width);
            ui.add_space(8.0);
            ui.checkbox(inverse, tr("Inverse direction"));
        });
    }

    section(ui, n_("BAKING OPTIONS"));
    disabled_text_check(ui, n_("Save preprocessed scene files"), true);
    disabled_text_check(ui, n_("Enable live preview baking process"), true);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.checkbox(&mut p.gpu_raytracing, tr("Enable GPU raytracing"))
            .on_hover_text(tr("Bakes cast their rays on the graphics card when one is available. Off, they run on the CPU."));
    });
    if gpu_note_shown(gpu) {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new(tr(GPU_RAYTRACING_UNAVAILABLE)).color(tk.danger));
        });
    }

    section(ui, n_("PREVIEW OPTIONS"));
    row(ui, tr("Local cache directory"), |ui, label| {
        ui.add_enabled_ui(false, |ui| {
            let mut path = String::new();
            not_available(ui.add(egui::TextEdit::singleline(&mut path).desired_width(CONTROL_WIDTH - 24.0))).labelled_by(label);
            not_available(ui.add(egui::Button::new("R").frame(false)));
        });
    });
    row(ui, tr("Local cache budget (in MB)"), |ui, label| {
        let mut budget = 4000_u32;
        ui.spacing_mut().item_spacing.x = 2.0;
        not_available(ui.add_enabled(false, egui::Button::new("−").frame(false)));
        not_available(ui.add_enabled(false, egui::Button::new("+").frame(false)));
        not_available(ui.add_enabled(false, egui::DragValue::new(&mut budget))).labelled_by(label);
    });
    row(ui, tr("Material preview shader"), |ui, _| {
        let text = format!("{}\n{}", tr("Select a preview shader"), tr("(default)"));
        not_available(ui.add_enabled(false, egui::Button::new(text).min_size(vec2(CONTROL_WIDTH, 40.0))));
    });

    section(ui, n_("TEMPORARY FILES"));
    row(ui, tr("Cache directory"), |ui, label| {
        #[cfg(not(target_arch = "wasm32"))]
        let mut path = std::env::temp_dir().display().to_string();
        #[cfg(target_arch = "wasm32")]
        let mut path = String::new();
        ui.add_enabled_ui(false, |ui| {
            not_available(ui.add(egui::TextEdit::singleline(&mut path).desired_width(CONTROL_WIDTH - 24.0))).labelled_by(label);
            not_available(ui.add(egui::Button::new("R").frame(false)));
        });
    });

    section(ui, n_("SPARSE VIRTUAL TEXTURE"));
    disabled_text_check(ui, n_("Hardware support acceleration"), true);

    section(ui, n_("IRAY HARDWARE"));
    disabled_text_check(ui, gpu.unwrap_or(n_("no GPU")), true);
    disabled_text_check(ui, n_("CPU"), false);

    // TextureCraft's own settings, after the reference sections
    section(ui, n_("APPEARANCE"));
    row(ui, tr("Interface scale"), |ui, label| {
        ui.add(egui::Slider::new(&mut p.ui_scale, 0.75..=2.0).fixed_decimals(2)).labelled_by(label);
    });
    row(ui, tr("Brightness"), |ui, label| {
        egui::ComboBox::from_id_salt("settings_brightness")
            .width(140.0)
            .selected_text(tr(p.brightness.label()))
            .show_ui(ui, |ui| {
                for b in Brightness::ALL {
                    ui.selectable_value(&mut p.brightness, b, tr(b.label()));
                }
            })
            .response
            .labelled_by(label);
    });
    row(ui, tr("Accent color"), |ui, _| {
        for a in Accent::ALL {
            let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, p.accent == a, tr(a.label())));
            let ring = if p.accent == a {
                tk.text_strong
            } else if resp.hovered() {
                tk.text_dim
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().circle(rect.center(), 6.5, a.colors(tk.dark).0, egui::Stroke::NONE);
            ui.painter().circle_stroke(rect.center(), 9.0, egui::Stroke::new(1.5, ring));
            if resp.on_hover_text(tr(a.label())).clicked() {
                p.accent = a;
            }
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        row(ui, tr("Title bar"), |ui, _| {
            ui.checkbox(&mut p.system_title_bar, tr("System title bar (applies on restart)"))
                .on_hover_text(tr("Use the operating system's window frame and buttons instead of TextureCraft's own title bar"));
        });
    }

    section(ui, n_("VIEWPORT"));
    row(ui, tr("Texture resolution"), |ui, _| {
        ui.horizontal(|ui| {
            for (v, l) in [(0u8, tr("Full")), (1, tr("Half")), (2, tr("Quarter"))] {
                ui.selectable_value(&mut p.viewport_texture, v, l);
            }
        })
        .response
        .on_hover_text(tr(
            "Size of the textures the viewport shows: lower uses less GPU memory with large texture sets. Painting and export keep full resolution.",
        ));
    });

    section(ui, n_("RESOURCES"));
    #[cfg(not(target_arch = "wasm32"))]
    {
        row(ui, tr("Max CPU threads for work"), |ui, label| {
            ui.add(egui::DragValue::new(&mut p.work_threads).range(0..=available_cores))
                .labelled_by(label)
                .on_hover_text(tr("Threads that paint, bake and export. 0 uses every CPU thread. Applies on restart."));
        });
        row(ui, tr("Cores kept for the UI"), |ui, label| {
            ui.add(egui::DragValue::new(&mut p.reserved_cores).range(0..=available_cores)).labelled_by(label);
        });
        row(ui, tr("Max RAM while saving"), |ui, label| {
            ui.add(egui::DragValue::new(&mut p.save_ram_percent).range(1..=100).suffix(" %")).labelled_by(label);
            let budget_bytes = texturecraft_engine::formats::save_ram_budget(available_ram_bytes, p.save_ram_percent);
            ui.label(format!("≈ {:.2} GiB", budget_bytes as f64 / (1_u64 << 30) as f64));
        });
    }
    row(ui, tr("Max project size on open"), |ui, label| {
        ui.add(egui::DragValue::new(&mut p.project_open_limit_gib).range(1..=u32::MAX).suffix(" GiB"))
            .labelled_by(label)
            .on_hover_text(tr("Maximum decompressed project data. Increase this limit to open larger projects."));
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        note(ui, tr("Saving uses the other cores and at most this share of the free RAM. At least one core always works."));
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        section(ui, n_("AUTOSAVE"));
        row(ui, tr("Autosave every"), |ui, label| {
            ui.add(egui::DragValue::new(&mut p.autosave_minutes).range(0..=120).suffix(format!(" {}", tr("min")))).labelled_by(label);
        });
        note(ui, &trf("0 turns autosave off. Copies go to {folder}", &[("folder", &crate::user_dir("autosave").display())]));

        section(ui, n_("FOLDERS"));
        row(ui, tr("Data folder"), |ui, label| {
            let default = crate::data_folder("").display().to_string();
            ui.add(egui::TextEdit::singleline(&mut p.data_folder).hint_text(default).desired_width(CONTROL_WIDTH - 30.0)).labelled_by(label);
            if ui.button("...").on_hover_text(tr("Choose the data folder")).clicked() {
                *pick = Some(SettingsFolder::DataFolder);
            }
        });
        for (label, dir) in [(tr("Smart materials"), crate::library_dir()), (tr("UI layouts"), crate::user_dir("layouts"))] {
            row(ui, label, |ui, _| ui.label(RichText::new(dir.display().to_string()).small().color(tk.text_dim)));
        }
        note(
            ui,
            tr(
                "Autosaves, smart materials, presets, templates and UI layouts go to the data folder. A new folder applies on restart; files already saved stay where they are.",
            ),
        );
    }
}

/// A small dim note under a section's rows.
#[cfg(not(target_arch = "wasm32"))]
fn note(ui: &mut Ui, text: &str) {
    let tk = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.add(egui::Label::new(RichText::new(text).small().color(tk.text_dim)).wrap());
    });
}

/// The built-in library locations as (name, path): the user's assets in the data folder, the
/// assets built into TextureCraft, and the font folders text alphas read.
pub fn built_in_locations() -> Vec<(String, String)> {
    let mut out = Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    out.push(("your_assets".to_owned(), crate::data_dir().display().to_string()));
    out.push(("starter_assets".to_owned(), tr("(built in)").to_owned()));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (system, user) = texturecraft_engine::formats::fonts::font_folders();
        let joined = |dirs: Vec<std::path::PathBuf>| dirs.iter().map(|dir| dir.display().to_string()).collect::<Vec<_>>().join("; ");
        out.push(("system_fonts".to_owned(), joined(system)));
        out.push(("user_fonts".to_owned(), joined(user)));
    }
    out
}

/// `text` cut at its start with "…" to fit `width` points in `font`.
fn elide_start(ui: &Ui, text: &str, font: &egui::FontId, width: f32) -> String {
    let fits = |text: &str| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).rect.width() <= width;
    if fits(text) {
        return text.to_owned();
    }

    let chars: Vec<char> = text.chars().collect();
    let shown = |from: usize| format!("…{}", chars.get(from..).unwrap_or_default().iter().collect::<String>());

    // The longest tail that fits; every shorter tail fits too
    let (mut low, mut high) = (1, chars.len());
    while low < high {
        let mid = low + (high - low) / 2;
        if fits(&shown(mid)) {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    shown(low)
}

/// The Libraries page.
fn libraries(ui: &mut Ui, draft: &mut SettingsDraft, project_open: bool, #[cfg(not(target_arch = "wasm32"))] pick: &mut Option<SettingsFolder>) {
    let tk = Tokens::get(ui.ctx());

    // The browser has no folders to add
    let can_change = !project_open && !cfg!(target_arch = "wasm32");
    section(ui, n_("LIBRARY LOCATIONS"));

    // With a project open the locations cannot change: the add row is gone and the note says why
    if project_open {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(tr(PROJECT_OPEN_NOTE));
        });
    } else {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                row(ui, tr("Name"), |ui, label| {
                    ui.add_enabled(can_change, egui::TextEdit::singleline(&mut draft.library_name).desired_width(CONTROL_WIDTH)).labelled_by(label);
                });
                row(ui, tr("Path"), |ui, label| {
                    ui.add_enabled(can_change, egui::TextEdit::singleline(&mut draft.library_path).desired_width(CONTROL_WIDTH - 24.0)).labelled_by(label);
                    let browse = ui.add_enabled(can_change, egui::Button::new("...").frame(false)).on_hover_text(tr("Choose the folder"));
                    #[cfg(not(target_arch = "wasm32"))]
                    if browse.clicked() {
                        *pick = Some(SettingsFolder::LibraryPath);
                    }
                    #[cfg(target_arch = "wasm32")]
                    not_available(browse);
                });
            });
            let add = ui.add_enabled_ui(can_change && draft.can_add_library(), |ui| icon_button(ui, ic::PLUS, tr("Add the location"), false, 24.0)).inner;
            let add = if cfg!(target_arch = "wasm32") { not_available(add) } else { add };
            if add.clicked() {
                draft.prefs.libraries.push(LibraryLocation { name: draft.library_name.trim().to_owned(), path: draft.library_path.trim().to_owned() });
                draft.library_name.clear();
                draft.library_path.clear();
            }
        });
    }
    ui.add_space(6.0);

    let mut remove = None;
    egui::Frame::new().fill(tk.field).inner_margin(6.0).corner_radius(3.0).show(ui, |ui| {
        ui.set_width(PAGE_WIDTH - 12.0);
        ui.label(tr("Locations:"));
        let font = egui::TextStyle::Body.resolve(ui.style());
        let built_in = built_in_locations();
        let added = draft.prefs.libraries.iter().map(|location| (location.name.clone(), location.path.clone()));
        for (i, (name, path)) in built_in.iter().cloned().chain(added).enumerate() {
            let is_built_in = i < built_in.len();
            ui.horizontal(|ui| {
                let left = egui::Layout::left_to_right(egui::Align::Center);
                ui.allocate_ui_with_layout(vec2(120.0, 24.0), left, |ui| {
                    ui.set_min_width(120.0);
                    ui.add(egui::Label::new(RichText::new(&name).strong()).truncate());
                });
                let shown = elide_start(ui, &path, &font, 260.0);
                ui.allocate_ui_with_layout(vec2(270.0, 24.0), left, |ui| {
                    ui.set_min_width(270.0);
                    ui.label(RichText::new(shown).italics()).on_hover_text(&path);
                });

                // New assets are saved to the user's assets; another default needs the asset model to choose
                let is_default = i == 0 && !cfg!(target_arch = "wasm32");
                let radio = ui.add_enabled(is_default, egui::RadioButton::new(is_default, tr("Default")));
                not_available(radio);
                let label = trf("Remove {name}", &[("name", &name)]);
                let minus = ui.add_enabled_ui(can_change && !is_built_in, |ui| icon_button(ui, ic::MINUS, &label, false, 20.0)).inner;
                let minus = match (is_built_in, is_default) {
                    (true, _) if i == 0 => minus.on_disabled_hover_text(tr(DEFAULT_LIBRARY_NOTE)),
                    (true, _) => not_available(minus),
                    _ => minus,
                };
                if minus.clicked() {
                    remove = i.checked_sub(built_in.len());
                }
            });
        }
    });
    if let Some(index) = remove
        && index < draft.prefs.libraries.len()
    {
        draft.prefs.libraries.remove(index);
    }
}

/// The logical CPUs and free RAM, read once a second while Settings is open (not every frame).
#[cfg(not(target_arch = "wasm32"))]
fn settings_resources(ctx: &egui::Context) -> (u32, u64) {
    let id = egui::Id::unique("settings_resources");
    let now = ctx.input(|input| input.time);
    ctx.request_repaint_after(std::time::Duration::from_secs(1));

    if let Some((sampled_at, cores, available_ram_bytes)) = ctx.data(|data| data.get_temp::<(f64, u32, u64)>(id))
        && (0.0..1.0).contains(&(now - sampled_at))
    {
        return (cores, available_ram_bytes);
    }

    let (cores, available_ram_bytes) = texturecraft_engine::formats::available_save_resources();
    let cores = u32::try_from(cores).unwrap_or(u32::MAX);
    ctx.data_mut(|data| data.insert_temp(id, (now, cores, available_ram_bytes)));
    (cores, available_ram_bytes)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn feedback_control_preferences_survive_confirming_an_open_draft() {
        let context = egui::Context::default();
        let session = texturecraft_engine::Session::closed().unwrap();
        let mut app = TextureCraftApp::new(&eframe::CreationContext::_new_kittest(context.clone()), session);
        let mut draft = SettingsDraft::of(&app);
        draft.keymap.insert("file.save".into(), "Ctrl+Alt+S".into());
        draft.library_name = "Studio".into();
        draft.prefs.autosave_minutes = 30;
        app.ui.windows.settings = true;
        app.ui.windows.settings_draft = Some(draft);

        let layout = serde_json::json!({"layout": "Only3D"});
        assert_eq!(crate::control::handle(&mut app, &context, "ui.set", &layout)["ok"], true);
        assert_eq!(app.ui.windows.settings_draft.as_ref().unwrap().prefs.autosave_minutes, 30);

        let prefs = serde_json::json!({"prefs": {"gpu_raytracing": false}});
        assert_eq!(crate::control::handle(&mut app, &context, "ui.set", &prefs)["ok"], true);
        let draft = app.ui.windows.settings_draft.take().unwrap();
        assert_eq!(draft.prefs.autosave_minutes, 30, "an unrelated control update keeps the pending interval");
        assert_eq!(draft.library_name, "Studio");
        apply(&mut app, draft);

        assert!(!app.ui.prefs.gpu_raytracing, "confirming Settings keeps the control update");
        assert_eq!(app.ui.prefs.autosave_minutes, 30, "confirming Settings applies the pending interval");
        assert_eq!(app.ui.keymap.get("file.save").map(String::as_str), Some("Ctrl+Alt+S"));
    }

    #[test]
    fn feedback_control_refreshes_changed_preferences_in_an_open_draft() {
        let context = egui::Context::default();
        let session = texturecraft_engine::Session::closed().unwrap();
        let mut app = TextureCraftApp::new(&eframe::CreationContext::_new_kittest(context.clone()), session);
        app.ui.prefs.language = "fr".into();
        let mut draft = SettingsDraft::of(&app);
        draft.prefs.autosave_minutes = 30;
        draft.prefs.language = "ja".into();
        draft.prefs.data_folder = "pending-data".into();
        app.ui.windows.settings = true;
        app.ui.windows.settings_draft = Some(draft);

        let invalid = serde_json::json!({"prefs": {"autosave_minutes": -1}});
        assert_eq!(crate::control::handle(&mut app, &context, "ui.set", &invalid)["ok"], false);
        assert_eq!(app.ui.prefs.autosave_minutes, Prefs::default().autosave_minutes);
        assert_eq!(app.ui.windows.settings_draft.as_ref().unwrap().prefs.autosave_minutes, 30);

        let update = serde_json::json!({"prefs": {"autosave_minutes": 45}});
        assert_eq!(crate::control::handle(&mut app, &context, "ui.set", &update)["ok"], true);
        let draft = app.ui.windows.settings_draft.take().unwrap();

        assert_eq!(draft.prefs.autosave_minutes, 45, "the external change takes precedence for the same field");
        assert_eq!(draft.prefs.language, Prefs::default().language, "an omitted field reset by the request counts as a change");
        assert_eq!(draft.prefs.data_folder, "pending-data", "an unchanged live field keeps its pending edit");
    }

    #[test]
    fn feedback_changing_the_autosave_interval_reschedules_the_deadline() {
        let context = egui::Context::default();
        let session = texturecraft_engine::sample_session(128, 32).unwrap();
        let mut app = TextureCraftApp::new(&eframe::CreationContext::_new_kittest(context), session);
        app.session.modified = true;
        app.ui.prefs.autosave_minutes = 120;
        app.last_time = 10.0;
        app.next_autosave = 120.0 * 60.0;
        let mut draft = SettingsDraft::of(&app);
        draft.prefs.autosave_minutes = 1;

        apply(&mut app, draft);

        assert_eq!(app.next_autosave, 70.0, "the new interval starts at the current frame time");
        app.last_time = 20.0;
        let draft = SettingsDraft::of(&app);
        apply(&mut app, draft);
        assert_eq!(app.next_autosave, 70.0, "an unchanged interval keeps the deadline");
    }

    #[test]
    fn a_folder_answer_cannot_edit_a_reopened_settings_window() {
        let session = texturecraft_engine::Session::closed().unwrap();
        let context = egui::Context::default();
        let mut app = TextureCraftApp::new(&eframe::CreationContext::_new_kittest(context), session);
        let folder = std::path::PathBuf::from("chosen-folder");
        app.file_dialogs.set_override(Some(Box::new(move |_| crate::file_dialogs::Answer::Paths(vec![folder.clone()]))));
        app.ui.windows.settings = true;
        app.ui.windows.settings_draft = Some(SettingsDraft::of(&app));
        ask_folder(&mut app, SettingsFolder::LibraryPath);
        assert_eq!(app.file_dialogs.pending(), 1);

        app.ui.windows.settings_draft = Some(SettingsDraft::of(&app));
        let frame = eframe::Frame::_new_kittest();
        for _ in 0..100 {
            app.file_dialogs.show_pending(&frame);
            app.run_file_dialog_answers();
            if app.file_dialogs.pending() == 0 {
                break;
            }

            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(app.file_dialogs.pending(), 0);
        assert!(app.ui.windows.settings_draft.unwrap().library_path.is_empty());
    }
}
