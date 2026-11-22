//! One integration-test executable avoids linking the same dependencies for every feature.
//! Keep feature tests in their own modules; the standalone ui_snapshots.rs target remains available.

mod alpha_parameters;
mod bake_job_mesh;
mod bake_mode;
mod bake_presets;
mod brush_drag;
mod brush_section_reset;
mod camera_views;
mod clone_source_view;
mod close_unsaved;
mod color_picker;
mod color_selection_editor;
mod curvature_generator;
mod display_environment;
mod display_post_effects;
mod effect_rows;
mod effect_sliders;
mod empty_start;
mod export_file_types;
mod export_map_overrides;
mod export_overrides;
mod export_progress;
mod export_settings;
mod export_template_guards;
mod file_dialogs;
mod fill_channel_image;
mod fill_projection_rows;
mod fill_support;
mod fill_symmetry_rows;
mod filter_picker;
mod frame_selection;
mod generator_panel;
mod gradient_map_menu;
mod help_menu_664;
mod history_panel;
mod import_resources;
mod inactive_set_hint;
mod layer_blending_options;
mod layer_clipboard;
mod layer_multi_select;
mod layer_rename;
mod layers_panel;
mod levels_editor;
mod log_panel_680;
mod mask_export;
mod mask_value_swap;
mod named_layouts;
mod nav_buttons;
mod new_project_baked_maps;
mod new_project_window;
mod output_templates;
mod panel_header;
mod path_in_view;
mod path_list;
mod pen_pressure;
mod polygon_fill;
mod popup_press_close;
mod project_config;
mod properties_min_width;
mod properties_popup;
mod rail;
mod reference_order;
mod rename_anchor;
mod reset_ui;
mod settings_window;
mod shelf_click;
mod shelf_drop;
mod shelf_hover_preview;
mod shelf_panel;
mod shift_lines_2d;
mod shift_lines_3d;
mod shortcut_gestures;
mod shortcut_keys;
mod shortcut_list;
mod side_panels;
mod skew_paint;
mod smudge_channels;
mod snapped_lines;
mod split_divider;
mod temp_dir;
mod template_invert;
mod templates_681;
mod tool_availability;
mod tool_settings_saved;
mod toolbar;
mod user_smart_mask;
mod view2d_navigation;
mod view2d_tiles;
mod view_mode_picker;
mod window_menu_684;

mod filter_support;
mod stencil;
mod symmetry;
mod symmetry_2d;
mod tile_generator_panel;

/// eframe's in-memory storage, shared by the tests that save and restore the UI state
#[path = "../../src/ui_state_store/memory_storage.rs"]
mod memory_storage;

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use texturecraft_ui_egui::TextureCraftApp;

/// Window of the Properties tests: tall enough that a whole sheet shows without scrolling
const PROPERTIES_WINDOW: egui::Vec2 = egui::vec2(1600.0, 1400.0);

// Small texture and bake sizes keep the sample session fast; the panels don't depend on them
const TEXTURE_RESOLUTION: u32 = 256;
const BAKE_RESOLUTION: u32 = 64;

/// Frames for the first layout and fonts to settle
const STARTUP_FRAMES: usize = 4;

/// Frames for a click or key to reach the panels and its change to show
const SETTLE_FRAMES: usize = 3;

/// Frames between the moves of a drag, and between a press and its release
const DRAG_STEP_FRAMES: usize = 2;

/// Presses and releases the primary button at `at` without moving, as a pointer click does
fn click_at(harness: &mut Harness<'_, TextureCraftApp>, at: egui::Pos2) {
    harness.hover_at(at);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        harness.run_steps(1);
    }

    harness.run_steps(SETTLE_FRAMES);
}

/// The close button nearest the right of a docked panel's `title` (its tab or header title), on the same row
fn panel_close_button(harness: &Harness<'_, TextureCraftApp>, title: egui::Rect) -> egui::Rect {
    harness
        .get_all_by_role(egui::Role::Button)
        .filter(|button| button.accesskit_node().label().as_deref() == Some("Close Panel"))
        .map(|button| button.rect())
        .filter(|close| close.min.x > title.max.x && title.y_range().contains(close.center().y))
        .min_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .expect("a close button right of the title")
}
