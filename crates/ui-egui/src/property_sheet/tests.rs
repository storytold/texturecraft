use egui::{Color32, CornerRadius, Painter, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use egui_phosphor::regular as ic;

use super::number_field::shown_number;
use super::{NumberField, PropertySheet};
use crate::theme::{Accent, Brightness, Tokens};
use texturecraft_engine::folder_cmds::next_seed;

mod filtering;
mod gallery;

use filtering::Filtering;
use gallery::Gallery;

/// Width of the Properties panel the mockups are drawn at
const PANEL_WIDTH: f32 = 300.0;

/// The harness window: the panel plus room on its right for popups and tips, tall enough for the whole gallery unscrolled
const HARNESS_SIZE: egui::Vec2 = egui::vec2(PANEL_WIDTH + 260.0, 1000.0);

/// The i of the gallery's Depth Culling row
const DEPTH_CULLING_TIP: &str = "Paints only the surface the projector sees first";

fn swatch(color: Color32) -> impl Fn(&Painter, Rect) {
    move |painter: &Painter, rect: Rect| {
        painter.rect_filled(rect, CornerRadius::ZERO, color);
    }
}

fn filtering_picture(painter: &Painter, rect: Rect, filtering: Filtering) {
    let shade = match filtering {
        Filtering::BilinearHq => 200,
        Filtering::BilinearSharp => 150,
        Filtering::Nearest => 100,
    };
    painter.rect_filled(rect.shrink(2.0), CornerRadius::same(2), Color32::from_gray(shade));
}

fn gallery(ui: &mut egui::Ui, state: &mut Gallery) {
    let panel = Tokens::get(ui.ctx()).panel;
    egui::Frame::NONE.fill(panel).show(ui, |ui| {
        ui.set_width(PANEL_WIDTH);
        PropertySheet::show(ui, "gallery", |sheet| {
            sheet.section("Fill", true);
            if let Some(scale) = sheet.switch("Scale", &["Tiling", "Physical size"], state.scale) {
                state.scale = scale;
            }
            if let Some(picked) = sheet
                .dropdown(
                    "Filtering",
                    state.filtering,
                    &[Filtering::BilinearHq, Filtering::BilinearSharp, Filtering::Nearest],
                    Filtering::name,
                    Some(&filtering_picture),
                )
                .0
            {
                state.filtering = picked;
            }
            if let Some(merge) = sheet.slider("Hardness", &mut state.hardness, 0.0..=1.0, 2) {
                state.hardness_merges.push(merge);
            }
            sheet.signed_slider("Offset", &mut state.offset, -1.0..=1.0, 2);
            sheet.angle("Rotation", &mut state.rotation, 0.0..=360.0);
            let names = ["Projection Extends Outside Shape", "Projection Is Cropped To Shape"];
            if let Some(picked) = sheet
                .dropdown(
                    "A label wider than the column's cap",
                    state.long_choice,
                    &[0, 1],
                    |index| names.get(index).copied().unwrap_or_default().to_owned(),
                    None,
                )
                .0
            {
                state.long_choice = picked;
            }

            let reset = sheet.band("3D projection settings", true, Some(state.position != [0.0; 3]), None);
            if reset.reset {
                state.position = [0.0; 3];
            }
            if reset.open {
                let mut fields = [("X", 0), ("Y", 1), ("Z", 2)].map(|(tag, axis)| NumberField {
                    tag,
                    tag_color: crate::theme::AXIS_COLORS.get(axis).copied(),
                    value: state.position[axis],
                    speed: 0.01,
                    decimals: 2,
                    suffix: "",
                });
                if let Some((axis, merge)) = sheet.number_fields("Position", &mut fields, Some(&mut state.linked), false)
                    && let Some(slot) = state.position.get_mut(axis)
                {
                    *slot = fields[axis].value;
                    state.position_merges.push(merge);
                }
            }

            if sheet.card("Edges", Some(true)) {
                sheet.toggle("Wrap", &mut state.wrap);
                sheet.toggle("Shape Crop", &mut state.crop);
                sheet.info(DEPTH_CULLING_TIP);
                sheet.picture_toggle(&swatch(Color32::from_gray(160)), "Depth Culling", &mut state.depth_culling);
                sheet.picture_toggle(&swatch(Color32::from_gray(100)), "Backface Culling", &mut state.backface_culling);
            }
            sheet.slot(&swatch(Color32::from_rgb(0xb0, 0x5a, 0x3a)), "Base color", "Uniform color", Some(&mut state.channel_on), false, false);
            sheet.slider("Value", &mut state.channel_value, 0.0..=1.0, 2);

            sheet.section("Filter", false);
            if let Some((index, _)) = sheet.channel_pills(&["Color", "Metal", "Rough", "Normal", "Height"], &state.channels)
                && let Some(on) = state.channels.get_mut(index)
            {
                *on = !*on;
            }
            let (_, chip) = sheet.slider_with_chip("Intensity", &mut state.intensity, 0.0..=1.0, 2, &format!("÷{}", state.divider), "÷1000");
            let dividers = ["1", "10", "100", "1000"];
            if let Some(picked) = sheet.hanging_list(&chip, state.divider, &dividers, &|divider: &str| format!("÷{divider}"), None).0 {
                state.divider = picked;
            }
            let band = sheet.band("Parameters", true, None, Some("3"));
            if band.open {
                sheet.caption("Offset");
                sheet.signed_slider("Horizontal", &mut state.offset, -1.0..=1.0, 2);
                if sheet.fold("Attributes", Some("Default"), false) {
                    sheet.button(ic::DOT, "No attributes");
                }
                sheet.seed("Seed", &mut state.seed);
                if sheet.button(ic::DICE_FIVE, "Randomize seeds").clicked() {
                    state.randomized += 1;
                }
            }
        });
    });
}

/// A harness in `brightness`, with the app's fonts from the first frame
fn builder<State>(brightness: Brightness) -> egui_kittest::HarnessBuilder<State> {
    Harness::builder().with_size(HARNESS_SIZE).with_context_setup(move |ctx| {
        crate::theme::install(ctx);
        crate::theme::apply(ctx, brightness, Accent::default());
    })
}

fn gallery_harness(brightness: Brightness, gallery_state: Gallery) -> Harness<'static, Gallery> {
    let mut harness = builder(brightness).build_ui_state(gallery, gallery_state);
    harness.run_steps(3);
    harness
}

#[test]
fn toggles_switches_and_pills_change_their_state() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let at = harness.get_by_label("Shape Crop").rect().center();
    click_at(&mut harness, at);
    let at = harness.get_by_label("Physical size").rect().center();
    click_at(&mut harness, at);
    let at = harness.get_by_label("Normal").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    assert!(harness.state().crop);
    assert_eq!(harness.state().scale, 1);
    assert_eq!(harness.state().channels, [true, true, true, true, true]);
}

#[test]
fn keyboard_focus_is_visible_on_switches_toggles_and_channel_pills() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for brightness in [Brightness::MediumDark, Brightness::Light] {
        let mut harness = builder(brightness).build_ui(|ui| {
            ui.set_width(PANEL_WIDTH);
            PropertySheet::show(ui, "keyboard", |sheet| {
                sheet.switch("Scale", &["Tiling", "Physical size"], 0);
                sheet.toggle("Shape Crop", &mut false);
                sheet.channel_pills(&["Normal"], &[true]);
            });
        });
        harness.run_steps(3);

        for label in ["Tiling", "Physical size", "Shape Crop", "Normal"] {
            harness.key_press(egui::Key::Tab);
            harness.run_steps(2);

            let node = harness.get_by_label(label);
            assert!(node.is_focused(), "{label} receives keyboard focus");

            let rect = node.rect();
            assert!(
                harness.output().shapes.iter().any(|clipped| {
                    matches!(&clipped.shape, egui::Shape::Rect(shape) if shape.stroke.width > 1.0 && shape.rect.center().distance(rect.center()) < 1.0)
                }),
                "{label} paints a visible focus outline"
            );
        }
    }
}

#[test]
fn keyboard_can_remove_an_id_color_without_hovering_the_row() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = builder(Brightness::MediumDark).build_ui_state(
        |ui, removed| {
            ui.set_width(PANEL_WIDTH);
            PropertySheet::show(ui, "keyboard", |sheet| {
                if !*removed {
                    *removed = sheet.id_color_row([10, 20, 30], false, true).remove;
                }
            });
        },
        false,
    );
    harness.run_steps(3);

    for label in ["Pick color R 10 G 20 B 30", "Remove R 10 G 20 B 30"] {
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);

        assert!(harness.get_by_label(label).is_focused(), "{label} receives keyboard focus");
    }
    harness.key_press(egui::Key::Space);
    harness.run_steps(2);

    assert!(*harness.state(), "Space activates the focused Remove button");
    assert!(harness.query_by_label("Remove R 10 G 20 B 30").is_none());
}

/// A toggle's pill starts on the controls column, where a dropdown's button starts (it sat 3 px in)
#[test]
fn a_toggles_pill_starts_where_a_dropdown_starts() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let dropdown = harness.get_by_label("Bilinear | HQ").rect();

    // The toggle's hit area ends with its pill
    let pill_left = harness.get_by_label("Shape Crop").rect().right() - super::switch_control::PILL_SIZE.x;
    assert_eq!(pill_left, dropdown.left(), "the pill starts at {pill_left}, the dropdown at {}", dropdown.left());
}

/// A culling row sits on the panel's grid like a plain toggle: its name on the label column, its picture and switch where the controls start, its i at the row's end
#[test]
fn a_culling_row_reads_name_then_picture_and_switch_with_its_i_at_the_end() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let culling = harness.get_by_label("Depth Culling").rect();
    let plain = harness.get_by_label("Wrap").rect();
    let info = harness.get_by_label(DEPTH_CULLING_TIP).rect();
    assert_eq!(culling.left(), plain.left(), "the name starts where a plain toggle's does: {culling:?} vs {plain:?}");
    assert!(info.left() > culling.right(), "the i sits after the switch, at the row's end: {info:?} vs {culling:?}");

    let was = harness.state().depth_culling;
    let at = harness.get_by_label("Depth Culling").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    assert_ne!(harness.state().depth_culling, was, "a click on the row flips it");
}

/// Two culling rows, the second named past the label column
fn culling_rows(ui: &mut egui::Ui, on: &mut [bool; 2]) {
    let panel = Tokens::get(ui.ctx()).panel;
    egui::Frame::NONE.fill(panel).show(ui, |ui| {
        ui.set_width(PANEL_WIDTH);
        PropertySheet::show(ui, "culling", |sheet| {
            sheet.picture_toggle(&swatch(Color32::from_gray(160)), "Depth", &mut on[0]);
            sheet.picture_toggle(&swatch(Color32::from_gray(100)), "Backface culling past the column", &mut on[1]);
        });
    });
}

/// A section caption's switch and an action row's toggle over a dropdown
fn right_end_switches(ui: &mut egui::Ui, mirror: &mut bool) {
    let panel = Tokens::get(ui.ctx()).panel;
    egui::Frame::NONE.fill(panel).show(ui, |ui| {
        ui.set_width(PANEL_WIDTH);
        PropertySheet::show(ui, "right end", |sheet| {
            sheet.section_switch("Symmetry", true);
            sheet.action_row(&[("Reset", true)], "Mirror", mirror);
            sheet.dropdown("Filtering", Filtering::Nearest, &[Filtering::Nearest], Filtering::name, None);
        });
    });
}

/// A switch at a caption's or a row's right end ends on the right padding, where a dropdown ends (they sat 3 px in)
#[test]
fn right_end_switches_end_where_a_dropdown_ends() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = builder(Brightness::MediumDark).build_ui_state(right_end_switches, false);
    harness.run_steps(3);

    let dropdown_right = harness.get_by_label("Nearest").rect().right();
    let pill_rights: Vec<f32> = harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.rect.size() == super::switch_control::PILL_SIZE => Some(rect.rect.right()),
            _ => None,
        })
        .collect();

    assert_eq!(pill_rights, [dropdown_right; 2], "the pills end at {pill_rights:?}, the dropdown at {dropdown_right}");
}

/// A culling name too long for the label column moves every culling row's picture and switch, so the switches stay one column
#[test]
fn culling_rows_keep_their_switches_in_one_column_when_a_name_is_long() {
    let mut harness = builder(Brightness::MediumDark).build_ui_state(culling_rows, [false; 2]);
    harness.run_steps(3);

    let short = harness.get_by_label("Depth").rect();
    let long = harness.get_by_label("Backface culling past the column").rect();

    assert_eq!(short.right(), long.right(), "both switches end at one x: {short:?} vs {long:?}");
}

/// Under a section caption, an instance card (`instance` true) or a slot card; records where its thumbnail was painted
fn card_under_caption(ui: &mut egui::Ui, state: &mut (bool, std::cell::Cell<Option<Rect>>)) {
    let (instance, thumbnail) = state;
    let record = |_: &Painter, rect: Rect| thumbnail.set(Some(rect));
    let panel = Tokens::get(ui.ctx()).panel;
    egui::Frame::NONE.fill(panel).show(ui, |ui| {
        ui.set_width(PANEL_WIDTH);
        PropertySheet::show(ui, "card", |sheet| {
            sheet.section("Caption", false);
            if *instance {
                sheet.source_card(&record, "Layer", "Set", false);
            } else {
                sheet.slot_restorable(&record, "Layer", "Set", false);
            }
        });
    });
}

/// Where the card under a caption paints its thumbnail
fn thumbnail_under_caption(instance: bool) -> Rect {
    let mut harness = builder(Brightness::MediumDark).build_ui_state(card_under_caption, (instance, std::cell::Cell::new(None)));
    harness.run_steps(3);
    harness.state().1.get().unwrap_or_else(|| panic!("the card (instance: {instance}) paints no thumbnail"))
}

/// The instance card keeps a card's air under its caption and above its thumbnail, as a slot card does (it sat 6 px higher)
#[test]
fn an_instance_card_sits_under_its_caption_as_a_slot_card_does() {
    let slot = thumbnail_under_caption(false);
    let instance = thumbnail_under_caption(true);

    assert_eq!(instance.top(), slot.top(), "the thumbnails start at one y: {instance:?} vs {slot:?}");
}

/// A gray slider in a panel-wide column; records the column's right edge
fn gray_row(ui: &mut egui::Ui, state: &mut (f32, f32)) {
    ui.set_width(PANEL_WIDTH);
    state.1 = ui.max_rect().right();
    crate::gray_slider::gray_slider(ui, "Color", &mut state.0);
}

/// A gray slider's invert button ends on the column's right edge (a plain button was wider than the room kept for it)
#[test]
fn a_gray_sliders_invert_button_ends_on_its_columns_right_edge() {
    // The upstream typed slider field has no accessible name
    let mut harness = builder(Brightness::MediumDark).with_accessibility_check(false).build_ui_state(gray_row, (0.5, 0.0));
    harness.run_steps(3);

    let invert = harness.get_by_role(egui::accesskit::Role::Button).rect();
    let right = harness.state().1;

    assert!(invert.right() <= right + 0.5, "the button ends at {} past the column's {right}", invert.right());
}

/// Where `wanted` is painted, first to last
fn painted_at<State>(harness: &Harness<'_, State>, wanted: &str) -> Vec<egui::Pos2> {
    let shapes = harness.output().shapes.iter();
    shapes
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text() == wanted => Some(text.pos),
            _ => None,
        })
        .collect()
}

/// An open dropdown's options start their words at the button's own word x (they started 1 px in, inside the list's border)
#[test]
fn an_open_dropdowns_options_start_at_the_buttons_word() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let button = harness.get_by_label("Bilinear | HQ").rect();

    let at = button.center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    let button_word = painted_at(&harness, "Bilinear | HQ").into_iter().find(|at| button.contains(*at)).map(|at| at.x);
    let option_word = painted_at(&harness, "Nearest").first().map(|at| at.x);

    assert!(button_word.is_some() && option_word == button_word, "the option's word at {option_word:?}, the button's at {button_word:?}");
}

#[test]
fn dropdown_list_hangs_from_its_button_at_its_width_and_picks() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let button = harness.get_by_label("Bilinear | HQ").rect();
    let at = harness.get_by_label("Bilinear | HQ").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    let option = harness.get_by_label("Nearest").rect();
    let list = option.expand(super::choice::LIST_BORDER);

    assert_eq!(list.left(), button.left(), "the list starts at the button's left edge");
    assert_eq!(list.width(), button.width(), "the list is the button's width");
    assert!(option.top() > button.bottom(), "the list hangs below the button");

    let at = harness.get_by_label("Nearest").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    assert_eq!(harness.state().filtering, Filtering::Nearest);
}

#[test]
fn keyboard_selection_closes_the_dropdown_and_restores_button_focus() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    harness.get_by_role_and_label(egui::Role::ComboBox, "Bilinear | HQ").focus();
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    harness.get_by_role_and_label(egui::Role::ListBoxOption, "Nearest").focus();
    harness.run_steps(2);

    assert!(harness.get_by_role_and_label(egui::Role::ListBoxOption, "Nearest").is_focused());

    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);

    assert_eq!(harness.state().filtering, Filtering::Nearest);
    assert!(harness.query_by_role_and_label(egui::Role::ListBoxOption, "Nearest").is_none());
    assert!(harness.get_by_role_and_label(egui::Role::ComboBox, "Nearest").is_focused());
}

/// Presses and lets go at `at` through the real pointer path, so a control drawn over another gets the click it would get from a person
fn click_at<State>(harness: &mut Harness<'_, State>, at: egui::Pos2) {
    let modifiers = egui::Modifiers::NONE;
    harness.event(egui::Event::PointerMoved(at));
    harness.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers });
    harness.run_steps(1);
    harness.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers });
    harness.run_steps(1);
}

/// Clicks at `at`, then types `text` into the field that opened and presses Enter
fn type_at(harness: &mut Harness<'_, Gallery>, at: egui::Pos2, text: &str) {
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run_steps(2);
    harness.get_by_role(egui::Role::TextInput).type_text(text);
    harness.run_steps(1);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
}

#[test]
fn a_click_on_a_value_or_the_seed_selects_it_so_typing_replaces_it() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let value = harness.get_by_label("Hardness value").rect();
    type_at(&mut harness, value.center(), "0.4");

    assert_eq!(harness.state().hardness, 0.4, "the slider's value is replaced, not appended to");

    let seed = harness.get_by_label("Seed value").rect();
    type_at(&mut harness, seed.center(), "77");

    assert_eq!(harness.state().seed, 77, "the seed is replaced, not appended to");
}

/// Clicks at `at` to type, then presses Enter without typing
fn open_and_leave_at(harness: &mut Harness<'_, Gallery>, at: egui::Pos2) {
    harness.drag_at(at);
    harness.drop_at(at);
    harness.run_steps(2);
    assert!(harness.query_by_role(egui::Role::TextInput).is_some(), "a click at {at:?} opens the value to type");
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert!(harness.query_by_role(egui::Role::TextInput).is_none(), "Enter closes the value at {at:?}");
}

/// A value opened to type and left as it opened is not an edit: no undo step, and its hidden digits are not rounded away
#[test]
fn a_value_opened_and_left_untyped_changes_nothing() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery { hardness: 0.333, position: [1.2345, 0.0, 0.0], ..Gallery::default() });
    let hardness = harness.get_by_label("Hardness value").rect();
    let position = harness.get_by_label("Position X").rect();

    open_and_leave_at(&mut harness, hardness.center());
    open_and_leave_at(&mut harness, position.center());

    assert_eq!(harness.state().hardness, 0.333, "the slider keeps its unrounded value");
    assert!(harness.state().hardness_merges.is_empty(), "the slider reports no edit: {:?}", harness.state().hardness_merges);
    assert_eq!(harness.state().position[0], 1.2345, "the number field keeps its unrounded value");
    assert!(harness.state().position_merges.is_empty(), "the number field reports no edit: {:?}", harness.state().position_merges);
}

/// A value opened to type and left untyped while something else changed it (an agent's command) keeps the newer value: no edit, no undo step
#[test]
fn a_value_left_untyped_keeps_a_change_made_while_it_was_open() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery { hardness: 0.333, position: [1.2345, 0.0, 0.0], ..Gallery::default() });
    for (newer_seed, label) in (1..).zip(["Hardness value", "Position X", "Seed value"]) {
        let at = harness.get_by_label(label).rect().center();
        harness.drag_at(at);
        harness.drop_at(at);
        harness.run_steps(2);

        assert!(harness.query_by_role(egui::Role::TextInput).is_some(), "{label}: the click opens the value to type");

        // Changed elsewhere while the field is open, then Enter without typing
        harness.state_mut().hardness = 0.9;
        harness.state_mut().position[0] = 0.5;
        harness.state_mut().seed = newer_seed;
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);

        assert!(harness.query_by_role(egui::Role::TextInput).is_none(), "{label}: Enter closes the value");
        assert_eq!(harness.state().hardness, 0.9, "{label}: the slider keeps the newer value");
        assert_eq!(harness.state().position[0], 0.5, "{label}: the number field keeps the newer value");
        assert_eq!(harness.state().seed, newer_seed, "{label}: the seed keeps the newer value");
    }

    assert!(harness.state().hardness_merges.is_empty(), "the slider reports no edit: {:?}", harness.state().hardness_merges);
    assert!(harness.state().position_merges.is_empty(), "the number field reports no edit: {:?}", harness.state().position_merges);
}

/// A reason too long for its caption's row is cut with "…", and hovering it shows it whole
#[test]
fn a_cut_unavailable_reason_reads_whole_in_its_tooltip() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    const REASON: &str = "Not available with this projection, for a reason far too long to fit beside the caption";
    const TOOLTIP_WAIT_FRAMES: usize = 60;
    let mut harness = builder(Brightness::MediumDark).build_ui(|ui| {
        ui.set_width(PANEL_WIDTH);
        PropertySheet::show(ui, "unavailable", |sheet| sheet.section_unavailable("Symmetry", REASON));
    });
    harness.run_steps(2);
    let reason = harness.get_by_label(REASON).rect();

    assert!(reason.right() <= PANEL_WIDTH + 1.0, "the reason is cut to fit the panel: {reason:?}");

    harness.hover_at(reason.center());
    harness.run_steps(TOOLTIP_WAIT_FRAMES);

    assert!(harness.get_all_by_label(REASON).count() > 1, "its tooltip reads it whole");
}

/// A typed value that lands on the current one is no edit: one past the end kept at the end, or the same number written another way
#[test]
fn a_typed_value_equal_to_the_current_one_changes_nothing() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery { hardness: 1.0, ..Gallery::default() });
    let hardness = harness.get_by_label("Hardness value").rect();
    type_at(&mut harness, hardness.center(), "5");

    assert_eq!(harness.state().hardness, 1.0, "a value past the end is kept at the end");
    assert!(harness.state().hardness_merges.is_empty(), "kept at the value it had, it reports no edit: {:?}", harness.state().hardness_merges);

    harness.state_mut().hardness = 0.75;
    harness.run_steps(1);
    type_at(&mut harness, hardness.center(), ".750");

    assert_eq!(harness.state().hardness, 0.75);
    assert!(harness.state().hardness_merges.is_empty(), "the same number written another way reports no edit: {:?}", harness.state().hardness_merges);
}

/// A value just under zero opens to type as it reads, without a minus sign
#[test]
fn a_value_just_under_zero_opens_without_a_minus_sign() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery { hardness: -0.001, ..Gallery::default() });
    let hardness = harness.get_by_label("Hardness value").rect();
    harness.drag_at(hardness.center());
    harness.drop_at(hardness.center());
    harness.run_steps(2);

    assert_eq!(harness.get_by_role(egui::Role::TextInput).value().as_deref(), Some("0.00"));
}

#[test]
fn a_drag_merges_after_its_first_step_and_a_click_does_not() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let track = harness.get_by_label("Hardness").rect();
    harness.drag_at(track.center());
    harness.run();
    harness.hover_at(track.center() + egui::vec2(20.0, 0.0));
    harness.run();
    harness.hover_at(track.center() + egui::vec2(40.0, 0.0));
    harness.run();
    harness.drop_at(track.center() + egui::vec2(40.0, 0.0));
    harness.run();
    let drag_merges = harness.state().hardness_merges.clone();

    let click = track.left_center() + egui::vec2(20.0, 0.0);
    harness.drag_at(click);
    harness.drop_at(click);
    harness.run();

    assert!(drag_merges.len() >= 2, "the drag changes the value more than once: {drag_merges:?}");
    assert_eq!(drag_merges.first(), Some(&false), "the drag's first change starts an undo step: {drag_merges:?}");
    assert!(drag_merges.iter().skip(1).all(|merge| *merge), "later changes continue it: {drag_merges:?}");

    let merges = &harness.state().hardness_merges;
    assert_eq!(merges.len(), drag_merges.len() + 1, "the click changes the value once: {merges:?}");
    assert_eq!(merges.last(), Some(&false), "the click starts its own undo step: {merges:?}");
}

/// A drag whose first frames change nothing (the knob held at its end) still starts its own undo step with its first change
#[test]
fn a_drag_pressed_past_the_tracks_end_starts_an_undo_step_with_its_first_change() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery { hardness: 1.0, ..Gallery::default() });
    let track = harness.get_by_label("Hardness").rect();
    let past_end = track.right_center() - egui::vec2(1.0, 0.0);
    harness.drag_at(past_end);
    harness.run();
    harness.hover_at(past_end + egui::vec2(20.0, 0.0));
    harness.run();
    harness.hover_at(track.center());
    harness.run();
    harness.hover_at(track.center() - egui::vec2(20.0, 0.0));
    harness.run();
    harness.drop_at(track.center() - egui::vec2(20.0, 0.0));
    harness.run();

    let merges = &harness.state().hardness_merges;
    assert!(merges.len() >= 2, "the drag changes the value more than once: {merges:?}");
    assert_eq!(merges.first(), Some(&false), "the first real change starts an undo step: {merges:?}");
    assert!(merges.iter().skip(1).all(|merge| *merge), "later changes continue it: {merges:?}");
}

/// A number field held still until egui starts its drag, then moved, starts its own undo step with its first change
#[test]
fn a_number_field_held_still_then_dragged_starts_an_undo_step_with_its_first_change() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let field = harness.get_by_label("Position X").rect();
    harness.drag_at(field.center());

    // Past egui's longest click, the press becomes a drag that has not moved
    harness.run_steps(5);
    harness.hover_at(field.center() + egui::vec2(10.0, 0.0));
    harness.run();
    harness.hover_at(field.center() + egui::vec2(20.0, 0.0));
    harness.run();
    let before_release = harness.state().position[0];
    let release = field.center() + egui::vec2(30.0, 0.0);
    harness.input_mut().events.extend([
        egui::Event::PointerMoved(release),
        egui::Event::PointerButton { pos: release, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ]);
    harness.step();

    let merges = &harness.state().position_merges;

    assert!(harness.state().position[0] > before_release, "motion on release changes the value");
    assert!(merges.len() >= 3, "the drag also changes the value on release: {merges:?}");
    assert_eq!(merges.first(), Some(&false), "the first move starts an undo step: {merges:?}");
    assert!(merges.iter().skip(1).all(|merge| *merge), "later moves continue it: {merges:?}");
}

/// A ramp with one marker at `state.0`, recording each marker drag's merge in `state.1`
fn one_marker_ramp(ui: &mut egui::Ui, state: &mut (f32, Vec<bool>)) {
    ui.set_width(PANEL_WIDTH);
    PropertySheet::show(ui, "ramp", |sheet| {
        if let Some(drag) = sheet.gradient_ramp(&[[1.0, 0.0, 0.0]], &[state.0], &|_| [0.5; 3]) {
            state.0 = drag.position;
            state.1.push(drag.merge);
        }
    });
}

/// A marker pressed past the ramp's end changes nothing at first; its first move inward still starts its own undo step
#[test]
fn a_gradient_marker_pressed_past_the_ramps_end_starts_an_undo_step_with_its_first_change() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = builder(Brightness::MediumDark).build_ui_state(one_marker_ramp, (1.0, Vec::new()));
    harness.run_steps(2);
    let marker = harness.get_by_label("Color 1").rect();
    let past_end = marker.right_center() - egui::vec2(1.0, 0.0);
    harness.drag_at(past_end);
    harness.run();
    harness.hover_at(past_end + egui::vec2(20.0, 0.0));
    harness.run();
    harness.hover_at(past_end - egui::vec2(40.0, 0.0));
    harness.run();
    harness.hover_at(past_end - egui::vec2(60.0, 0.0));
    harness.run();
    harness.drop_at(past_end - egui::vec2(60.0, 0.0));
    harness.run();

    let merges = &harness.state().1;
    assert!(merges.len() >= 2, "the drag changes the value more than once: {merges:?}");
    assert_eq!(merges.first(), Some(&false), "the first real change starts an undo step: {merges:?}");
    assert!(merges.iter().skip(1).all(|merge| *merge), "later changes continue it: {merges:?}");
}

#[test]
fn the_label_column_fits_the_widest_short_label_and_long_labels_stack() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let hardness = harness.get_by_label("Hardness").rect();
    let filtering = harness.get_by_label("Bilinear | HQ").rect();
    let crop = harness.get_by_label("Projection Extends Outside Shape").rect();

    let column = filtering.left();
    assert!((hardness.left() - column).abs() < 0.5, "sliders start on the controls column");
    assert!(crop.left() < column, "a label wider than the cap stacks above a full-width control");
}

#[test]
fn reset_restores_the_band_and_the_seed_dice_steps_deterministically() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let at = harness.get_by_label("Random").rect().center();
    click_at(&mut harness, at);
    let at = harness.get_by_label("Restore defaults").rect().center();
    click_at(&mut harness, at);
    let at = harness.get_by_label("Randomize seeds").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    assert_eq!(harness.state().seed, next_seed(1234));
    assert_eq!(harness.state().position, [0.0; 3]);
    assert_eq!(harness.state().randomized, 1);
}

/// The gallery in every theme, closed and with a list open, for comparing with the mockups
#[test]
#[ignore]
fn property_sheet_gallery_snapshots() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(output_dir) = std::env::var_os("TC_UI_SNAPSHOTS").map(std::path::PathBuf::from) else { return };

    std::fs::create_dir_all(&output_dir).unwrap();
    for (brightness, name) in [(Brightness::MediumDark, "medium"), (Brightness::Dark, "dark"), (Brightness::Light, "light")] {
        let mut harness = builder(brightness).with_pixels_per_point(2.0).wgpu().build_ui_state(gallery, Gallery::default());
        harness.run_steps(4);
        harness.render().unwrap().save(output_dir.join(format!("sheet-{name}.png"))).unwrap();

        let at = harness.get_by_label("Bilinear | HQ").rect().center();
        click_at(&mut harness, at);
        harness.run_steps(2);
        harness.get_by_label("Bilinear | Sharp").hover();
        harness.run_steps(2);
        harness.render().unwrap().save(output_dir.join(format!("sheet-{name}-list-open.png"))).unwrap();
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);

        // A value: the grey line on hover, then typing on that line in accent; the seed's box lights the same way
        let value = harness.get_by_label("Hardness value").rect().center();
        harness.hover_at(value);
        harness.run_steps(2);
        harness.render().unwrap().save(output_dir.join(format!("sheet-{name}-value-hover.png"))).unwrap();
        harness.drag_at(value);
        harness.drop_at(value);
        harness.run_steps(2);
        harness.get_by_role(egui::Role::TextInput).type_text("0.4");
        harness.run_steps(2);
        harness.render().unwrap().save(output_dir.join(format!("sheet-{name}-value-typing.png"))).unwrap();
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        let seed = harness.get_by_label("Seed value").rect().center();
        harness.hover_at(seed);
        harness.run_steps(2);
        harness.render().unwrap().save(output_dir.join(format!("sheet-{name}-seed-hover.png"))).unwrap();
        harness.drag_at(seed);
        harness.drop_at(seed);
        harness.run_steps(2);
        harness.render().unwrap().save(output_dir.join(format!("sheet-{name}-seed-typing.png"))).unwrap();
    }
}

#[test]
fn a_number_field_release_and_new_press_in_one_frame_start_separate_undo_steps() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = gallery_harness(Brightness::MediumDark, Gallery::default());
    let start = harness.get_by_label("Position X").rect().center();
    let restart = start + egui::vec2(10.0, 0.0);
    harness.drag_at(start);
    harness.hover_at(restart);
    harness.run();
    let before_restart = harness.state().position[0];

    assert!(before_restart > 0.0);
    assert_eq!(harness.state().position_merges, [false]);

    let button = |pressed| egui::Event::PointerButton { pos: restart, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    harness.input_mut().events.extend([button(false), button(true)]);
    harness.step();
    harness.hover_at(restart + egui::vec2(10.0, 0.0));
    harness.run();

    assert!(harness.state().position[0] > before_restart);
    assert_eq!(harness.state().position_merges, [false, false], "the new press starts its own undo step");
}

#[test]
fn a_number_field_reads_zero_without_a_minus_sign() {
    assert_eq!(shown_number(-0.0, 0), "0");
    assert_eq!(shown_number(-0.004, 2), "0.00");
    assert_eq!(shown_number(-0.3, 0), "0");
    assert_eq!(shown_number(-0.6, 0), "-1");
    assert_eq!(shown_number(-1.25, 2), "-1.25");
    assert_eq!(shown_number(2.5, 1), "2.5");
}

/// Folds keep their own open state when a fold above them comes and goes (the state follows the name, not the row's place)
#[test]
fn a_fold_keeps_its_state_when_rows_above_it_come_and_go() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let folds = |ui: &mut egui::Ui, extra_shown: &mut bool| {
        PropertySheet::show(ui, "folds", |sheet| {
            if *extra_shown && sheet.fold("Extra", None, true) {
                sheet.button(ic::DOT, "Extra rows");
            }
            if sheet.fold("Edges", None, true) {
                sheet.button(ic::DOT, "Edge rows");
            }
            if sheet.fold("Parameters", None, false) {
                sheet.button(ic::DOT, "Parameter rows");
            }
        });
    };
    let mut harness = builder(Brightness::MediumDark).build_ui_state(folds, true);
    harness.run_steps(2);
    let at = harness.get_by_label("Edges").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    assert!(harness.query_by_label("Edge rows").is_none());

    *harness.state_mut() = false;
    harness.run_steps(2);

    assert!(harness.query_by_label("Edge rows").is_none(), "Edges stays folded");
    assert!(harness.query_by_label("Parameter rows").is_none(), "Parameters stays folded");
}

/// Two colour bars, one under the other, each opening its own picker
fn two_color_bars(ui: &mut egui::Ui) {
    PropertySheet::show(ui, "color_bars", |sheet| {
        sheet.color_bar("Bar A", [0.2, 0.4, 0.6], None, &[]);
        sheet.color_bar("Bar B", [0.6, 0.4, 0.2], None, &[]);
    });
}

/// The colour space picked in one picker is the one the next picker opens in, even for a bar drawn before it all along
#[test]
fn the_next_picker_opens_in_the_colour_space_last_picked() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = builder(Brightness::MediumDark).build_ui(two_color_bars);
    harness.run_steps(2);
    let at = harness.get_by_role_and_label(egui::Role::Button, "Bar B").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);
    let at = harness.get_by_role_and_label(egui::Role::ComboBox, "sRGB").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);
    let at = harness.get_by_role_and_label(egui::Role::ListBoxOption, "Linear").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    assert!(harness.query_by_role_and_label(egui::Role::ComboBox, "Linear").is_some(), "Bar B's picker shows Linear");

    let at = harness.get_by_role_and_label(egui::Role::Button, "Close").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);
    let at = harness.get_by_role_and_label(egui::Role::Button, "Bar A").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    assert!(harness.query_by_role_and_label(egui::Role::TitleBar, "Bar A").is_some(), "Bar A's picker is open");
    assert!(harness.query_by_role_and_label(egui::Role::ComboBox, "Linear").is_some(), "Bar A's picker opens in Linear");
}

/// A colour bar's picker stays open when a colour bar above it comes and goes (its state follows its name, not its place)
#[test]
fn a_color_bars_picker_stays_open_when_a_row_above_it_goes_away() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let bars = |ui: &mut egui::Ui, extra_shown: &mut bool| {
        PropertySheet::show(ui, "color_bars", |sheet| {
            if *extra_shown {
                sheet.color_bar("Extra", [0.5; 3], None, &[]);
            }
            sheet.color_bar("Tint", [0.2, 0.4, 0.6], None, &[]);
        });
    };
    let mut harness = builder(Brightness::MediumDark).build_ui_state(bars, true);
    harness.run_steps(2);
    let at = harness.get_by_role_and_label(egui::Role::Button, "Tint").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    assert!(harness.query_by_role_and_label(egui::Role::TitleBar, "Tint").is_some(), "the picker opened");

    *harness.state_mut() = false;
    harness.run_steps(2);

    assert!(harness.query_by_role_and_label(egui::Role::TitleBar, "Tint").is_some(), "the picker stays open");
}

/// A fold under a later section, before that section's first band, keeps its state when a band of the section above comes and goes
#[test]
fn a_fold_before_a_sections_first_band_keeps_its_state_when_the_band_above_goes_away() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let sections = |ui: &mut egui::Ui, band_shown: &mut bool| {
        PropertySheet::show(ui, "sections", |sheet| {
            sheet.section("First", false);
            if *band_shown && sheet.band("Band", true, None, None).open {
                sheet.button(ic::DOT, "Band rows");
            }
            sheet.section("Second", false);
            if sheet.fold("Edges", None, true) {
                sheet.button(ic::DOT, "Edge rows");
            }
        });
    };
    let mut harness = builder(Brightness::MediumDark).build_ui_state(sections, true);
    harness.run_steps(2);
    let at = harness.get_by_label("Edges").rect().center();
    click_at(&mut harness, at);
    harness.run_steps(2);

    assert!(harness.query_by_label("Edge rows").is_none());

    *harness.state_mut() = false;
    harness.run_steps(2);

    assert!(harness.query_by_label("Edge rows").is_none(), "Edges stays folded");
}

/// The colour picker's hex field ends where the colour field above it ends (it ended 4 px short)
#[test]
fn the_pickers_hex_field_ends_on_the_colour_fields_right_edge() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = builder(Brightness::MediumDark).build_ui(two_color_bars);
    harness.run_steps(2);
    let at = harness.get_by_role_and_label(egui::Role::Button, "Bar A").rect().center();

    click_at(&mut harness, at);
    harness.run_steps(2);

    let field = harness.get_by_role_and_label(egui::Role::Slider, "Saturation and value").rect();
    let hex = harness.get_by_role_and_label(egui::Role::TextInput, "Hex color").rect();
    assert_eq!(hex.right(), field.right(), "the hex field ends at {}, the colour field at {}", hex.right(), field.right());
}

/// A comparison's two sources, its operators beside the first
fn compare_rows(ui: &mut egui::Ui) {
    let panel = Tokens::get(ui.ctx()).panel;
    egui::Frame::NONE.fill(panel).show(ui, |ui| {
        ui.set_width(PANEL_WIDTH);
        PropertySheet::show(ui, "compare", |sheet| {
            let names = ["Left source", "Right source", "Other source"];
            let name = |index: usize| names.get(index).copied().unwrap_or_default().to_owned();
            sheet.compare_rows((0, 1), &[0, 1, 2], name, [Color32::RED, Color32::BLUE], (&["Min", "Max"], 0));
        });
    });
}

/// A compare source's tagged button shows its value where its open list's options start, a dropdown's value x (it sat 2 px right)
#[test]
fn a_compare_sources_value_starts_where_its_lists_options_do() {
    let mut harness = builder(Brightness::MediumDark).build_ui(compare_rows);
    harness.run_steps(3);
    let button = harness.get_by_role_and_label(egui::Role::ComboBox, "Left source").rect();

    click_at(&mut harness, button.center());
    harness.run_steps(2);

    let button_word = painted_at(&harness, "Left source").into_iter().find(|at| button.contains(*at)).map(|at| at.x);
    let option_word = painted_at(&harness, "Other source").first().map(|at| at.x);
    assert!(button_word.is_some() && option_word == button_word, "the option's word at {option_word:?}, the button's at {button_word:?}");
}

/// A linked Tiling and Offset block over a ramp
fn ramp_under_tiling(ui: &mut egui::Ui, state: &mut ([f32; 2], [f32; 2], bool, f32)) {
    let (tiling, offset, linked, gray) = state;
    let panel = Tokens::get(ui.ctx()).panel;
    egui::Frame::NONE.fill(panel).show(ui, |ui| {
        ui.set_width(PANEL_WIDTH);
        PropertySheet::show(ui, "ramp", |sheet| {
            let size = super::PreviewSize { label: "Tiling", values: tiling, range: 0.0..=10.0, typed: 0.0..=100.0, decimals: 2, linked };
            let lower = super::PreviewLower::Offset { values: offset, range: 0.0..=1.0, typed: -10.0..=10.0 };
            sheet.preview_block(&swatch(Color32::from_gray(128)), size, lower);
            sheet.ramp("Grayscale", gray, 0.0..=1.0);
        });
    });
}

/// A ramp's value ends on the linked Tiling and Offset values above it (it ended 2 px right of them)
#[test]
fn a_ramps_value_ends_on_the_linked_tiling_value() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut harness = builder(Brightness::MediumDark).build_ui_state(ramp_under_tiling, ([1.0; 2], [0.0; 2], true, 0.5));

    harness.run_steps(3);

    let tiling = harness.get_by_label("Tiling value").rect();
    let ramp = harness.get_by_label("Grayscale value").rect();
    assert_eq!(ramp.right(), tiling.right(), "the ramp's value ends at {}, Tiling's at {}", ramp.right(), tiling.right());
}

#[test]
fn a_reopened_editor_starts_a_new_undo_step_after_a_hidden_release() {
    let context = egui::Context::default();
    let button =
        |pressed| egui::Event::PointerButton { pos: egui::Pos2::ZERO, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    let edit = |owner, pressed| {
        let mut result = None;
        let input = egui::RawInput { events: vec![button(pressed)], ..Default::default() };
        let mut output = context.run_ui(input, |ui| {
            if let Some(owner) = owner {
                result = crate::widgets::press_edit(ui, owner, true);
            }
        });
        output.textures_delta.clear();
        result
    };

    let owner = egui::Id::unique("first editor");

    assert_eq!(edit(Some(owner), true), Some(false));
    edit(None, false);

    assert_eq!(edit(Some(owner), true), Some(false), "a separate press starts an undo step after reopening");
}

#[test]
fn section_tabs_show_focus_before_a_keyboard_jump() {
    let mut harness = builder(Brightness::MediumDark).build_ui(|ui| {
        super::section_tabs(ui, super::SectionScope::Docked, 0, &[("", "Fill"), ("", "Symmetry")]);
    });
    let tab = harness.get_by_role_and_label(egui::Role::Tab, "Symmetry");
    let bounds = tab.rect();
    tab.focus();
    harness.run_steps(2);

    assert!(harness.get_by_role_and_label(egui::Role::Tab, "Symmetry").is_focused());
    assert!(
        harness.output().shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(rect)
            if rect.stroke.width > 0.0 && bounds.contains_rect(rect.rect) && rect.rect.contains(bounds.center()))
        }),
        "the focused section has a visible outline before activation"
    );
}

#[test]
fn changing_anchor_names_releases_the_previous_editor_and_draft() {
    let mut harness = builder(Brightness::MediumDark).build_ui_state(
        |ui, anchor: &mut usize| {
            PropertySheet::show(ui, *anchor, |sheet| {
                sheet.name_field("Anchor point name", "Original");
            });
        },
        0,
    );

    let counts = |harness: &Harness<'_, usize>| harness.ctx.data(|data| (data.count::<egui::text_edit::TextEditState>(), data.count::<String>()));
    let mut first = None;
    for anchor in 1..=32 {
        *harness.state_mut() = anchor;
        harness.run();
        harness.get_by_role_and_label(egui::Role::TextInput, "Anchor point name").click();
        harness.run();
        harness.event(egui::Event::Text(" draft".into()));
        harness.run();
        first.get_or_insert_with(|| counts(&harness));
    }

    let (editors, drafts) = counts(&harness);
    eprintln!("32 anchors with focused drafts: {editors} text editors, {drafts} draft strings");
    assert_eq!(drafts, 1);
    assert_eq!(Some((editors, drafts)), first, "switching owners retains only the current editor footprint");
}
