use egui::{Color32, CornerRadius, Painter, Rect};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

use super::number_field::shown_number;
use super::{NumberField, PropertySheet};
use crate::theme::{Accent, Brightness, Tokens};

mod filtering;
mod gallery;

use filtering::Filtering;
use gallery::Gallery;

/// Width of the Properties panel the mockups are drawn at
const PANEL_WIDTH: f32 = 300.0;

/// The harness window: the panel plus room on its right for popups and tips, tall enough for the whole gallery unscrolled
const HARNESS_SIZE: egui::Vec2 = egui::vec2(PANEL_WIDTH + 260.0, 1000.0);

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

/// A culling name too long for the label column moves every culling row's picture and switch, so the switches stay one column
#[test]
fn culling_rows_keep_their_switches_in_one_column_when_a_name_is_long() {
    let mut harness = builder(Brightness::MediumDark).build_ui_state(culling_rows, [false; 2]);
    harness.run_steps(3);

    let short = harness.get_by_label("Depth").rect();
    let long = harness.get_by_label("Backface culling past the column").rect();

    assert_eq!(short.right(), long.right(), "both switches end at one x: {short:?} vs {long:?}");
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
