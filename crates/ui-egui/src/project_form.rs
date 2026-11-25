//! Rows and sections that New project and Edit > Project Configuration share: the label, control
//! and info icon of a setting, collapsible sections, and the settings the engine does not have
//! yet (shown disabled, with a hover hint that says so).

use egui::{Response, RichText, Sense, Ui};

pub use crate::color_management::ColorManagement;
use crate::dock::info_icon;
use crate::i18n::{n_, tr};
use crate::theme::{Tokens, caption};
use crate::widgets::section_chevron;

/// Hover hint of a control whose function TextureCraft does not have yet.
pub const NOT_AVAILABLE: &str = n_("Not available yet");

/// Hover hint of the Color management dropdown while OpenColorIO is chosen.
pub const OCIO_NOT_AVAILABLE: &str = n_("OpenColorIO is not available yet. The project uses Legacy.");

/// Width of the dropdowns and fields at the right of a row.
pub(crate) const FIELD_WIDTH_POINTS: f32 = 220.0;

/// Tallest the scrolling form of the two windows grows (the rest scrolls).
pub(crate) const FORM_HEIGHT_POINTS: f32 = 760.0;

/// Scrolling form height when viewport space is limited.
pub(crate) const MIN_FORM_HEIGHT_POINTS: f32 = 120.0;

/// Indent of the rows below a section title.
pub(crate) const ROW_INDENT_POINTS: f32 = 16.0;

/// `response` with the not-available hint while it is disabled.
pub(crate) fn unavailable(response: Response) -> Response {
    response.on_disabled_hover_text(tr(NOT_AVAILABLE))
}

/// A row: `label` at the left, then `control` right-aligned before the info icon (`info`).
pub(crate) fn row<R>(ui: &mut Ui, label: &str, info: Option<&str>, control: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_space(ROW_INDENT_POINTS);
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            info_slot(ui, info);
            control(ui)
        })
        .inner
    })
    .inner
}

/// A row that starts with its control (a checkbox or radio button) and ends with the info icon.
/// `trailing` adds a control just before the icon (a button).
pub(crate) fn check_row<R>(ui: &mut Ui, info: Option<&str>, control: impl FnOnce(&mut Ui) -> R, trailing: impl FnOnce(&mut Ui)) -> R {
    ui.horizontal(|ui| {
        ui.add_space(ROW_INDENT_POINTS);
        let inner = control(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            info_slot(ui, info);
            trailing(ui);
        });
        inner
    })
    .inner
}

fn info_slot(ui: &mut Ui, info: Option<&str>) {
    match info {
        Some(text) => info_icon(ui, text),

        // An invisible icon keeps the controls of rows without one in the same column
        None => {
            ui.add_visible(false, egui::Label::new(egui_phosphor::regular::INFO));
        }
    }
}

/// A bold sub-section title such as Import settings, with an optional info icon at the end.
pub(crate) fn sub_title(ui: &mut Ui, title: &str, info: Option<&str>) {
    ui.horizontal(|ui| {
        ui.add_space(ROW_INDENT_POINTS);
        ui.label(caption(title));
        if let Some(text) = info {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| info_icon(ui, text));
        }
    });
}

/// A dim italic note in a section.
pub(crate) fn note(ui: &mut Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(2.0 * ROW_INDENT_POINTS);
        ui.add(egui::Label::new(RichText::new(text).italics().color(Tokens::get(ui.ctx()).text_dim)).wrap());
    });
}

/// A dropdown that cannot be opened, showing `value`, with the not-available hint.
pub(crate) fn disabled_combo(ui: &mut Ui, id: &str, value: &str) -> Response {
    let response =
        ui.add_enabled_ui(false, |ui| egui::ComboBox::from_id_salt(id).width(FIELD_WIDTH_POINTS).selected_text(value).show_ui(ui, |_| {}).response).inner;
    unavailable(response)
}

/// A collapsible section: a chevron and `title` (clicking either toggles it), an optional info
/// icon at the end of the header, and `body` while open.
pub(crate) fn collapsible(ui: &mut Ui, id: &str, title: &str, default_open: bool, info: Option<&str>, body: impl FnOnce(&mut Ui)) {
    let id = ui.make_persistent_id(id);
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    ui.horizontal(|ui| {
        ui.add_space(ROW_INDENT_POINTS);
        let header = ui
            .horizontal(|ui| {
                ui.add(egui::Label::new(section_chevron(ui, state.is_open())).selectable(false));
                ui.add(egui::Label::new(caption(title)).selectable(false));
            })
            .response
            .interact(Sense::click());
        if header.clicked() {
            state.toggle(ui);
        }
        if let Some(text) = info {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| info_icon(ui, text));
        }
    });
    state.show_body_unindented(ui, |ui| {
        ui.indent(id.with("body"), body);
    });
}

/// "Compute tangent space per fragment" (not available yet, so unticked and disabled).
pub(crate) fn tangent_space_row(ui: &mut Ui) {
    check_row(
        ui,
        Some(tr("Compute the tangent space of normal maps per pixel instead of per vertex")),
        |ui| unavailable(ui.add_enabled(false, egui::Checkbox::new(&mut false, tr("Compute tangent space per fragment")))),
        |_| {},
    );
}

/// The UV tile workflow radios under "Use UV Tile workflow": per material (what the UV tile
/// workflow does) or one texture set per tile (not available yet). `enabled` while the workflow
/// can be chosen and is on.
pub(crate) fn uv_tile_radios(ui: &mut Ui, enabled: bool, disabled_hint: &str) {
    ui.indent("uv_tile_radios", |ui| {
        let preserve = check_row(
            ui,
            Some(tr("Each material keeps its UV tiles in one texture set, and strokes paint across tiles")),
            |ui| ui.add_enabled(enabled, egui::RadioButton::new(true, tr("Keep UV tiles together per material"))),
            |_| {},
        );
        preserve.on_disabled_hover_text(disabled_hint);
        check_row(
            ui,
            Some(tr("Each UV tile becomes a texture set of its own")),
            |ui| unavailable(ui.add_enabled(false, egui::RadioButton::new(false, tr("Use a separate texture set for each tile")))),
            |_| {},
        );
    });
}

/// Import settings ▸ Auto-unwrap and its options. Meshes without UVs are always unwrapped, so the
/// box is ticked; turning it off and the options are not available yet.
pub(crate) fn auto_unwrap_row(ui: &mut Ui) {
    check_row(
        ui,
        Some(tr("Generate UVs for meshes that have none")),
        |ui| unavailable(ui.add_enabled(false, egui::Checkbox::new(&mut true, tr("Auto-unwrap")))),
        |ui| {
            unavailable(ui.add_enabled(false, egui::Button::new(tr("Auto-unwrap options"))));
        },
    );
}

/// The Physical size section: the mesh file's own unit scale is what TextureCraft uses; a custom
/// scale and physical-size fill layers are not available yet.
pub(crate) fn physical_size(ui: &mut Ui, id: &str, default_open: bool) {
    const UNIT_FIELD_WIDTH_POINTS: f32 = 60.0;

    collapsible(ui, id, tr("Physical size"), default_open, Some(tr("The real-world size of one unit of the mesh")), |ui| {
        check_row(ui, Some(tr("Use the unit scale stored in the mesh file")), |ui| ui.radio(true, tr("Use the mesh file’s unit scale")), |_| {});
        check_row(
            ui,
            Some(tr("Set the size of one unit yourself")),
            |ui| unavailable(ui.add_enabled(false, egui::RadioButton::new(false, tr("Custom unit scale")))),
            |_| {},
        );
        ui.horizontal(|ui| {
            ui.add_space(3.0 * ROW_INDENT_POINTS);
            ui.label(tr("1 unit ="));
            unavailable(ui.add_enabled(false, egui::TextEdit::singleline(&mut String::new()).desired_width(UNIT_FIELD_WIDTH_POINTS)));
            ui.label(tr("cm"));
        });
        check_row(
            ui,
            Some(tr("Fill layers of applied materials are scaled in physical size instead of UV space")),
            |ui| unavailable(ui.add_enabled(false, egui::Checkbox::new(&mut false, tr("Scale material fills in real-world units")))),
            |_| {},
        );
    });
}

/// Rows of the OpenColorIO settings: (label, shown value, info). They are not available yet.
const OCIO_CONFIGURATION: [(&str, &str, Option<&str>); 1] =
    [(n_("OpenColorIO configuration"), "ACES 2.0", Some(n_("The OpenColorIO configuration that defines the color spaces")))];
const OCIO_IMPORT: [(&str, &str, Option<&str>); 4] = [
    (n_("8 bit images"), "sRGB Encoded Rec.709 (sRGB)", None),
    (n_("16 bit images"), "sRGB Encoded Rec.709 (sRGB)", None),
    (n_("Floating point images"), "Linear Rec.709 (sRGB)", None),
    (n_("Auto detect color spaces"), n_("Parse file name"), Some(n_("Read an imported image's color space from its file name"))),
];
const OCIO_EXPORT: [(&str, &str, Option<&str>); 3] = [
    (n_("8 bit images"), "sRGB Encoded Rec.709 (sRGB)", None),
    (n_("16 bit images"), "sRGB Encoded Rec.709 (sRGB)", None),
    (n_("Floating point images"), "ACEScg", None),
];

/// The Color management section. `mode` is the dropdown's choice: Legacy shows the working color
/// space. OpenColorIO previews disabled settings while the project continues to use Legacy.
pub(crate) fn color_management(ui: &mut Ui, id: &str, mode: &mut ColorManagement) {
    collapsible(ui, id, tr("Color management"), true, None, |ui| {
        ui.label(caption(tr("Configuration")));
        row(ui, tr("Color management"), Some(tr("Legacy works in linear sRGB. OpenColorIO manages color through a configuration")), |ui| {
            let response = egui::ComboBox::from_id_salt(format!("{id}_mode"))
                .width(FIELD_WIDTH_POINTS)
                .selected_text(tr(mode.label()))
                .show_ui(ui, |ui| {
                    for choice in [ColorManagement::Legacy, ColorManagement::OpenColorIo] {
                        ui.selectable_value(mode, choice, tr(choice.label()));
                    }
                })
                .response;
            if *mode == ColorManagement::OpenColorIo {
                response.on_hover_text(tr(OCIO_NOT_AVAILABLE));
            }
        });
        if *mode == ColorManagement::OpenColorIo {
            for (label, value, info) in OCIO_CONFIGURATION {
                row(ui, tr(label), info.map(tr), |ui| disabled_combo(ui, &format!("{id}_{label}"), tr(value)));
            }
            row(ui, tr("Configuration file"), Some(tr("A custom OpenColorIO configuration file")), |ui| {
                unavailable(ui.add_enabled(false, egui::TextEdit::singleline(&mut String::new()).desired_width(FIELD_WIDTH_POINTS)));
            });
        }

        ui.add_space(4.0);
        ui.label(caption(tr("Color settings")));
        let working_space = if *mode == ColorManagement::OpenColorIo { "ACEScg" } else { "Linear - sRGB Chrominance (ITU-R BT.709)" };
        row(ui, tr("Working color space"), Some(tr("The color space layers are blended in")), |ui| ui.label(working_space));
        if *mode != ColorManagement::OpenColorIo {
            return;
        }

        row(ui, tr("Standard sRGB color space"), Some(tr("The color space of sRGB images and colors")), |ui| {
            disabled_combo(ui, &format!("{id}_srgb"), "sRGB Encoded Rec.709 (sRGB)")
        });
        ui.add_space(4.0);
        ui.label(caption(tr("Bitmap import color space defaults")));
        for (label, value, info) in OCIO_IMPORT {
            row(ui, tr(label), info.map(tr), |ui| disabled_combo(ui, &format!("{id}_import_{label}"), tr(value)));
        }
        ui.add_space(4.0);
        ui.label(caption(tr("Materials")));
        row(ui, tr("Material color space default"), Some(tr("The color space of material inputs that do not state one")), |ui| {
            disabled_combo(ui, &format!("{id}_material"), "sRGB Encoded Rec.709 (sRGB)")
        });
        ui.add_space(4.0);
        ui.label(caption(tr("Export color spaces")));
        for (label, value, info) in OCIO_EXPORT {
            row(ui, tr(label), info.map(tr), |ui| disabled_combo(ui, &format!("{id}_export_{label}"), tr(value)));
        }
    });
}
