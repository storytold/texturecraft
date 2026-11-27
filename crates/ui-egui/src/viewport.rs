//! The 3D viewport and the 2D UV view: navigation, painting input, brush cursor and the GPU
//! callbacks that render them.

use std::sync::{Arc, Mutex};

use egui::emath::GuiRounding as _;
use egui::{Color32, PointerButton, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;
use texturecraft_engine::camera_cmds::DEFAULT_CAMERA;
use texturecraft_engine::core::BakedMap;
use texturecraft_engine::geom::glam::{Vec2, Vec3};
use texturecraft_engine::{Session, Tool, ViewMode};
use texturecraft_render::{Frame, SetHighlight, Shade, ViewKind};

use crate::TextureCraftApp;
use crate::decal_drop::DecalDrop;
use crate::gpu::ViewCallback;
use crate::i18n::{tr, trf};
use crate::path_hit::PathHit;
use crate::path_surface_cache::{PathSurfaceCache, SurfaceCurve};
use crate::projection_gizmo::ShownGizmo;
use crate::state::ViewLayout;
use crate::theme::Tokens;
use crate::toolbar::{MAX_BRUSH_SIZE, MIN_BRUSH_SIZE};

/// Hold this key with the Projection tool to rotate, scale or move the stencil with the mouse
pub const STENCIL_KEY: egui::Key = egui::Key::S;
/// Hold this key with the Clone tool and click to set the source
pub const CLONE_SOURCE_KEY: egui::Key = egui::Key::V;

pub const VIEW_3D: u64 = 1;
pub const VIEW_2D: u64 = 2;

/// While on, a click in the 3D view focuses the depth of field there (a transient UI mode kept in
/// egui's memory).
fn focus_pick_id() -> egui::Id {
    egui::Id::unique("tc_focus_pick")
}

pub fn focus_picking(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(focus_pick_id())).unwrap_or(false)
}

pub fn set_focus_picking(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(focus_pick_id(), on));
}

/// What the views' aux texture shows: the quick mask being edited, a mesh map, the image a pending ID pick reads instead of the ID mesh map, or a mask
fn aux_view(app: &TextureCraftApp) -> Option<crate::gpu::AuxView> {
    match (app.session.view, app.picking_id_color) {
        _ if quick_mask_shown(app) => Some(crate::gpu::AuxView::QuickMask),
        (ViewMode::MeshMap(BakedMap::Id), Some(pick)) => Some(crate::gpu::AuxView::IdMask(pick.effect)),
        (ViewMode::MeshMap(map), _) => Some(crate::gpu::AuxView::MeshMap(map)),
        (ViewMode::Mask(id), _) => Some(crate::gpu::AuxView::Mask(id)),
        (ViewMode::Image, _) => Some(crate::gpu::AuxView::Image),
        _ if app.ui.mode == crate::state::Mode::Baking && app.ui.bake_visualization.bent_normal_shown() => {
            Some(crate::gpu::AuxView::MeshMap(BakedMap::BentNormal))
        }
        _ => None,
    }
}

/// The 3D view's widget: the same in the dock and with the UI hidden, so it keeps the focus when Tab hides the UI
pub fn view_3d_id() -> egui::Id {
    egui::Id::unique("texturecraft-view3d")
}

/// The 2D view's widget, as [`view_3d_id`]
pub fn view_2d_id() -> egui::Id {
    egui::Id::unique("texturecraft-view2d")
}

/// Draw the widget of a view and declare it as a document canvas of the window. In a split view both are, so the keys
/// are the same in either.
fn view_widget(ui: &Ui, rect: Rect, id: egui::Id, sense: Sense) -> egui::Response {
    let response = ui.interact(rect, id, sense);
    crate::framework::host_canvas(ui.ctx(), id);
    response
}

/// The viewport dock tab: the paint bar, then the 3D and 2D views in the chosen layout with the mode's overlays
pub fn central(app: &mut TextureCraftApp, ui: &mut Ui) {
    let mode = app.ui.mode;
    let bake_painting = mode == crate::state::Mode::Baking
        && app.ui.bake_paint_brush.on
        && !app.session.doc.bake_meshes.high.is_empty()
        && app.session.doc.project.texture_sets.get(app.session.ts()).is_some_and(|texture_set| {
            let settings = &texture_set.bake;
            match app.ui.bake_paint_brush.target {
                texturecraft_engine::core::BakePaintTarget::Skew => settings.skew_applies(),
                texturecraft_engine::core::BakePaintTarget::Cage => {
                    !settings.high_poly.use_low_as_high && settings.cage.kind == texturecraft_engine::core::CageType::DistanceBased
                }
            }
        });
    app.ui.bake_paint_brush.on = bake_painting;

    if !app.ui.hide_ui {
        crate::toolbar::paint_bar(app, ui);
    }

    // No project open: empty views with only their view-mode picker
    if !app.session.project_open {
        if app.render.is_active() {
            crate::render_mode::stop(app);
        }

        // The closed project's last frames must not stay readable (`viewport.screenshot`)
        if let Some(render_state) = app.render_state.as_ref()
            && let Some(gpu) = render_state.renderer.write().callback_resources.get_mut::<crate::gpu::GpuState>()
        {
            gpu.renderer.drop_view(VIEW_3D);
            gpu.renderer.drop_view(VIEW_2D);
        }
        empty_views(app, ui);
        return;
    }

    let stroke_end = ui.scope_id().with("paint_2d_stroke_end");

    // A cancelled stroke can leave a pending 2D endpoint; do not attach it to a later gesture
    let pending_2d_endpoint = if !app.session.stroke_active() || !tool_acts(app) {
        ui.data_mut(|data| {
            let endpoint = data.get_temp(stroke_end);
            data.remove::<crate::uv_paint_point::UvPaintPoint>(stroke_end);
            endpoint
        })
    } else {
        None
    };
    finish_unavailable_tool_gestures(app, pending_2d_endpoint);
    crate::id_color_pick::IdColorPick::end_when_stale(app);
    let tokens = Tokens::get(ui.ctx());
    let aux_view = aux_view(app);
    let mut sync = PendingSync(Some(aux_view));
    let full = ui.available_rect_before_wrap();
    path_keys(app, ui);

    // Skew painting needs the 2D view alongside the 3D Bake preview
    let layout = if mode == crate::state::Mode::Painting || bake_painting { app.ui.layout } else { ViewLayout::Only3D };
    match layout {
        ViewLayout::Only3D => view_3d(app, ui, full, &mut sync),
        ViewLayout::Only2D => view_2d(app, ui, full, &mut sync),
        ViewLayout::Split => {
            // The 2D view's edges sit on whole pixels, so rounding can't nudge a texture its zoom keeps at one size
            let pixels_per_point = ui.ctx().pixels_per_point();
            let full = full.with_max_x(full.right().round_to_pixels(pixels_per_point));
            let divider_x = split_divider_x(app, ui, full);
            let left = Rect::from_min_max(full.min, pos2(divider_x - SPLIT_LINE_HALF_WIDTH, full.max.y));
            let right = Rect::from_min_max(pos2(divider_x + SPLIT_LINE_HALF_WIDTH, full.min.y), full.max);
            let (rect_3d, rect_2d) = if app.ui.swap_views { (right, left) } else { (left, right) };

            // Each view draws only on its side, so overlays such as the texture set name stay out of a narrow neighbour
            let clip = ui.clip_rect();
            ui.set_clip_rect(rect_3d.intersect(clip));
            view_3d(app, ui, rect_3d, &mut sync);
            ui.set_clip_rect(rect_2d.intersect(clip));
            view_2d(app, ui, rect_2d, &mut sync);
            ui.set_clip_rect(clip);
            split_divider_grab(ui, full, divider_x);
            let line = Stroke::new(SPLIT_LINE_HALF_WIDTH * 2.0, tokens.chrome);
            ui.painter().line_segment([pos2(divider_x, full.top()), pos2(divider_x, full.bottom())], line);
        }
    }

    crate::properties_popup::show(app, ui.ctx());

    if mode == crate::state::Mode::Rendering {
        crate::render_mode::show(app, ui, full);
    } else if app.render.is_active() {
        crate::render_mode::stop(app);
    }
    if mode == crate::state::Mode::Baking {
        crate::bake_mode::bake_bar(app, ui.ctx(), full);
        crate::bake_mode::bake_warnings(app, ui.ctx(), full);
    }

    // Keep drawing until the 3D view finishes accumulating anti-aliasing samples
    let accumulating = layout != ViewLayout::Only2D
        && app.render_state.as_ref().is_some_and(|render_state| {
            render_state.renderer.read().callback_resources.get::<crate::gpu::GpuState>().is_some_and(|gpu| gpu.renderer.accumulating(VIEW_3D))
        });
    if app.session.stroke_active() || accumulating {
        ui.ctx().request_repaint();
    }
}

/// The views while no project is open: no mesh, no texture, each with its view-mode picker
fn empty_views(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    let full = ui.available_rect_before_wrap();
    let regions: Vec<(Rect, &str)> = match app.ui.layout {
        ViewLayout::Only3D => vec![(full, "v3d")],
        ViewLayout::Only2D => vec![(full, "v2d")],
        ViewLayout::Split => {
            let middle = (full.left() + full.width() * 0.5).round();
            let left = Rect::from_min_max(full.min, pos2(middle - SPLIT_LINE_HALF_WIDTH, full.max.y));
            let right = Rect::from_min_max(pos2(middle + SPLIT_LINE_HALF_WIDTH, full.min.y), full.max);
            if app.ui.swap_views { vec![(left, "v2d"), (right, "v3d")] } else { vec![(left, "v3d"), (right, "v2d")] }
        }
    };

    for (rect, id) in regions {
        // Nothing to click or drag, but still the window's canvas: Tab hides and shows the UI
        view_widget(ui, rect, if id == "v2d" { view_2d_id() } else { view_3d_id() }, Sense::focusable_noninteractive());
        ui.painter().rect_filled(rect, 0.0, tokens.viewport);
        view_header(app, ui, rect, id);
    }
}

/// Half the split divider's line width: each view starts this far from the line's centre
const SPLIT_LINE_HALF_WIDTH: f32 = 1.0;

/// Narrowest a split view may get: a sliver that is still easy to grab and drag back out
const MIN_SPLIT_VIEW_WIDTH: f32 = 16.0;

/// Width of the split divider's grab zone, centred on its line
const SPLIT_DIVIDER_GRAB_WIDTH: f32 = 8.0;

/// The split divider's widget id, shared by its drag read and its grab zone
fn split_divider_id(ui: &Ui) -> egui::Id {
    ui.scope_id().with("split_divider")
}

/// The 3D view's share of `width`, keeping both views at least [`MIN_SPLIT_VIEW_WIDTH`] wide when they fit
fn clamp_split_ratio(ratio: f32, width: f32) -> f32 {
    let min_ratio = (MIN_SPLIT_VIEW_WIDTH / width).min(0.5);
    let ratio = if ratio.is_finite() { ratio } else { 0.5 };
    ratio.clamp(min_ratio, 1.0 - min_ratio)
}

/// The split divider's x position: while it is dragged it follows the pointer and changes the 2D zoom so the texture keeps its size on screen
fn split_divider_x(app: &mut TextureCraftApp, ui: &Ui, full: Rect) -> f32 {
    // The 2D view's left edge, beside the line, sits on a whole pixel
    let pixels_per_point = ui.ctx().pixels_per_point();

    // The ratio is the 3D view's share, so a swap moves the divider to the other side of the width
    let swapped = app.ui.swap_views;
    let divider_x = |ratio: f32| {
        let left_share = clamp_split_ratio(ratio, full.width());
        let left_share = if swapped { 1.0 - left_share } else { left_share };
        let unsnapped = full.left() + full.width() * left_share;
        (unsnapped + SPLIT_LINE_HALF_WIDTH).round_to_pixels(pixels_per_point) - SPLIT_LINE_HALF_WIDTH
    };
    let old_x = divider_x(app.ui.split_ratio);

    // Read the drag before the views lay out so they follow the pointer in the same frame
    let ctx = ui.ctx();
    if ctx.is_being_dragged(split_divider_id(ui))
        && ctx.input(|i| i.pointer.primary_down())
        && let Some(pointer) = ctx.pointer_interact_pos()
    {
        // `max` and `min` rather than `clamp`, so a NaN bound from a corrupt saved zoom can't panic
        let (min_x, max_x) = divider_range_keeping_uv_size(&app.ui, full, old_x, pixels_per_point);
        let target_x = pointer.x.max(min_x).min(max_x);
        let left_share = (target_x - full.left()) / full.width();
        app.ui.split_ratio = clamp_split_ratio(if swapped { 1.0 - left_share } else { left_share }, full.width());
        let new_x = divider_x(app.ui.split_ratio);
        keep_uv_view_on_screen(&mut app.ui, full, old_x, new_x);
    }

    divider_x(app.ui.split_ratio)
}

/// Lowest 2D zoom, for the wheel and the divider: a tile of about 2 px, as measured in the reference app
pub(crate) const UV_VIEW_MIN_ZOOM: f32 = texturecraft_render::UV_VIEW_MIN_ZOOM;

/// Highest 2D zoom, for the wheel and the divider: about ×220 of the fit, as measured in the reference app
pub(crate) const UV_VIEW_MAX_ZOOM: f32 = texturecraft_render::UV_VIEW_MAX_ZOOM;

/// 2D view turn per point of an Alt+left drag (radians): the 3D orbit's speed
const UV_RADIANS_PER_POINT: f32 = ORBIT_RADIANS_PER_POINT;

/// Step of an Alt+Shift+left drag's turn of the 2D view (radians)
const UV_SNAP_RADIANS: f32 = std::f32::consts::FRAC_PI_2;

/// Alt+right drag in the 2D view: change of the log zoom per point dragged right and per point dragged down.
/// Fitted to the reference app's measurements: 100 points right 760 -> 860 px, 100 points up 760 -> 704 px.
const UV_DRAG_ZOOM_PER_POINT_RIGHT: f32 = 0.001236;
const UV_DRAG_ZOOM_PER_POINT_DOWN: f32 = 0.00077;

/// UV zoom change per point of mouse wheel, as a fraction of the zoom
pub(crate) const UV_WHEEL_ZOOM_PER_POINT: f32 = 0.002;

/// Dash length of the straight-line preview, in points
const RUBBER_BAND_DASH: f32 = 6.0;

/// Gap between the straight-line preview's dashes, in points
const RUBBER_BAND_GAP: f32 = 4.0;

/// Opacity of the UV tile outlines in the 2D view
const UV_TILE_OUTLINE_ALPHA: u8 = 40;

/// Width of the UV tile outlines, in points
const UV_TILE_OUTLINE_WIDTH: f32 = 1.0;

/// The texture set label in the 2D view's bottom-left corner: its inset from the corner and its text size, in points
const SET_NAME_INSET: egui::Vec2 = vec2(10.0, 10.0);
const SET_NAME_TEXT_SIZE: f32 = 11.0;

/// Smallest skew brush circle in the 2D view, in points, so a tiny brush stays visible
const BAKE_PAINT_CURSOR_MIN_RADIUS: f32 = 2.0;

/// The bake brush cursor's outline: width in points and white alpha
const BAKE_PAINT_CURSOR_STROKE: f32 = 1.0;
const BAKE_PAINT_CURSOR_ALPHA: u8 = 200;

/// The projection stencil's dashed outline: width, white alpha, dash and gap, in points
const STENCIL_OUTLINE_STROKE: f32 = 1.0;
const STENCIL_OUTLINE_ALPHA: u8 = 120;
const STENCIL_OUTLINE_DASH: f32 = 5.0;
const STENCIL_OUTLINE_GAP: f32 = 4.0;

/// The 2D view's width when the split divider is at `divider_x`
fn uv_view_width(full: Rect, divider_x: f32, swapped: bool) -> f32 {
    let width = if swapped { divider_x - full.left() } else { full.right() - divider_x };
    (width - SPLIT_LINE_HALF_WIDTH).max(1.0)
}

/// The share of the 2D view's height that a zoom of 1 fills at `width`: the view fits the texture to its narrower side
fn uv_view_fit(full: Rect, width: f32) -> f32 {
    (width / full.height().max(1.0)).min(1.0)
}

/// The divider positions where the 2D zoom that keeps the texture's screen size stays inside the 2D zoom range
fn divider_range_keeping_uv_size(ui_state: &crate::state::UiState, full: Rect, old_x: f32, pixels_per_point: f32) -> (f32, f32) {
    // The zoom times the fit is the texture's screen size, which a divider drag keeps
    let screen_zoom = ui_state.uv_zoom * uv_view_fit(full, uv_view_width(full, old_x, ui_state.swap_views));

    // Whole pixels, rounded inward, so snapping the divider to a pixel can't step past the range
    let narrowest_view = (full.height() * screen_zoom / UV_VIEW_MAX_ZOOM * pixels_per_point).ceil() / pixels_per_point;
    let widest_view = (full.height() * screen_zoom / UV_VIEW_MIN_ZOOM * pixels_per_point).floor() / pixels_per_point;

    if ui_state.swap_views {
        // The 2D view is on the left: the divider moves right to widen it
        let edge = full.left() + SPLIT_LINE_HALF_WIDTH;
        return (edge + narrowest_view, edge + widest_view);
    }

    let edge = full.right() - SPLIT_LINE_HALF_WIDTH;
    (edge - widest_view, edge - narrowest_view)
}

/// Keeps the 2D view's texture at the same size and offset from the view's centre when the divider moves its edge from `old_x` to `new_x`
fn keep_uv_view_on_screen(ui_state: &mut crate::state::UiState, full: Rect, old_x: f32, new_x: f32) {
    let (old_width, new_width) = (uv_view_width(full, old_x, ui_state.swap_views), uv_view_width(full, new_x, ui_state.swap_views));
    ui_state.uv_zoom *= uv_view_fit(full, old_width) / uv_view_fit(full, new_width);

    // The horizontal pan is in half view widths
    ui_state.uv_pan[0] *= old_width / new_width;
}

/// The split divider's grab zone with a resize cursor, claimed after the views so it wins the pointer over them
fn split_divider_grab(ui: &Ui, full: Rect, divider_x: f32) {
    let zone = Rect::from_x_y_ranges(divider_x - SPLIT_DIVIDER_GRAB_WIDTH * 0.5..=divider_x + SPLIT_DIVIDER_GRAB_WIDTH * 0.5, full.y_range());
    ui.interact(zone, split_divider_id(ui), Sense::drag()).on_hover_and_drag_cursor(egui::CursorIcon::ResizeHorizontal);
}

/// The shader's view modes (`Shade::view_mode`): the material, one channel, the aux texture (mesh map, mask or quick mask), the UV checker, the combined normal
const VIEW_MODE_MATERIAL: u32 = 0;
const VIEW_MODE_CHANNEL: u32 = 1;
const VIEW_MODE_AUX_TEXTURE: u32 = 2;
const VIEW_MODE_UV_CHECKER: u32 = 3;
const VIEW_MODE_NORMAL_HEIGHT_MESH: u32 = 4;

/// Whether the views draw the mesh wireframe (the 3D edges and the 2D UV edges follow the one switch)
fn wireframe_shown(ui: &crate::state::UiState, polygon_fill_tool: bool) -> bool {
    ui.mesh_wireframe || polygon_fill_tool
}

/// Opacity of the wireframe polygon fill shows while the Show mesh wireframe switch is off
const POLYGON_FILL_WIREFRAME_OPACITY: f32 = 0.4;

fn shade(app: &TextureCraftApp) -> Shade {
    let (view_mode, channel) = match app.session.view {
        _ if quick_mask_shown(app) => (VIEW_MODE_AUX_TEXTURE, 0),
        ViewMode::Material => (VIEW_MODE_MATERIAL, 0),
        ViewMode::Channel(selected_channel) => (VIEW_MODE_CHANNEL, selected_channel.index() as u32),
        ViewMode::MeshMap(_) | ViewMode::Mask(_) | ViewMode::Image => (VIEW_MODE_AUX_TEXTURE, 0),
        ViewMode::UvChecker => (VIEW_MODE_UV_CHECKER, 0),
        ViewMode::NormalHeightMesh => (VIEW_MODE_NORMAL_HEIGHT_MESH, 0),
    };
    let set_index = app.session.ts();
    let height_strength = app.session.doc.project.texture_sets.get(set_index).map(|texture_set| texture_set.atlas_height_strength()).unwrap_or(1.0);

    // Polygon fill shows the polygons it fills without changing the saved wireframe toggles
    let polygon_fill_tool = tool_acts(app) && app.session.tool.tool == Tool::PolygonFill;
    Shade {
        height_strength,
        view_mode,
        channel,
        wireframe: wireframe_shown(&app.ui, polygon_fill_tool),
        wireframe_3d: wireframe_shown(&app.ui, polygon_fill_tool),
        background: app.ui.prefs.background,
        // Polygon fill's own wireframe keeps its look when the user's opacity hides the switch's wireframe
        wireframe_opacity: if app.ui.mesh_wireframe { app.ui.wireframe_opacity / 100.0 } else { POLYGON_FILL_WIREFRAME_OPACITY },
        ..Shade::from_project(&app.session.doc.project)
    }
}

/// The texture set a shelf material or image dragged over the 3D view `response` would drop on, its edges drawn in the accent color
fn drop_highlight(app: &TextureCraftApp, ui: &Ui, response: &egui::Response) -> Option<SetHighlight> {
    let set = crate::shelf::drop_set(app, ui, response)?;

    Some(set_highlight(ui, set))
}

fn set_highlight(ui: &Ui, texture_set: usize) -> SetHighlight {
    let accent = egui::Rgba::from(Tokens::get(ui.ctx()).viewport_accent);

    SetHighlight { set: texture_set, color: [accent.r(), accent.g(), accent.b()] }
}

/// Show what the GPU side reported (each message once while its toast is up).
fn gpu_report_toasts(app: &mut TextureCraftApp, reports: crate::gpu::GpuReports) {
    let names: Vec<String> = reports.memory.iter().filter_map(|ts| app.session.doc.project.texture_sets.get(*ts).map(|t| t.name.clone())).collect();

    if !names.is_empty() {
        // Failed allocations are not retried, so keep a notice until the user dismisses it
        // Start a new list on the first failure after dismissal
        let (sets, shown) = &mut app.gpu_memory;
        let current = shown.take().filter(|text| app.toasts.iter().any(|t| &t.text == text));
        match &current {
            Some(text) => app.toasts.retain(|t| &t.text != text),
            None => sets.clear(),
        }
        for n in names {
            if !sets.contains(&n) {
                sets.push(n);
            }
        }
        let text = match sets.as_slice() {
            [one] => trf(
                "Not enough GPU memory to show texture set \"{name}\" fully: lower the Texture resolution in Display Settings › Render settings. Painting and export are not affected.",
                &[("name", one)],
            ),
            many => {
                let short = |n: &String| match n.char_indices().nth(MAX_NAME_CHARS) {
                    Some((cut, _)) => format!("\"{}...\"", &n[..cut]),
                    None => format!("\"{n}\""),
                };
                let mut list = many.iter().take(MAX_NAMED_SETS).map(short).collect::<Vec<_>>().join(", ");
                if many.len() > MAX_NAMED_SETS {
                    list.push_str(", ...");
                }
                trf(
                    "Not enough GPU memory to show {count} texture sets fully ({names}): lower the Texture resolution in Display Settings › Render settings. Painting and export are not affected.",
                    &[("count", &many.len()), ("names", &list)],
                )
            }
        };
        app.gpu_memory.1 = Some(text.clone());
        app.toast_sticky(text);
    }
    for e in &reports.errors {
        let text = trf("GPU error: {error}", &[("error", e)]);
        if !app.toasts.iter().any(|t| t.text == text) {
            app.toast(text, true);
        }
    }
}

/// Texture sets a GPU memory notice names before "...".
const MAX_NAMED_SETS: usize = 5;
/// Characters of a set name in that notice (long names are cut, with "...").
const MAX_NAME_CHARS: usize = 32;

/// This frame's scene sync (compositing and GPU uploads), run by the first view drawn, after
/// that view handled its input: a stroke shows in the frame it is painted, not the next.
pub struct PendingSync(Option<Option<crate::gpu::AuxView>>);

/// A view's render size in pixels, rounded as egui rounds the rect it draws the frame into, so the frame is never stretched
fn frame_size_in_pixels(ui: &Ui, rect: Rect) -> [u32; 2] {
    let pixels_per_point = ui.ctx().pixels_per_point();
    let pixels = |from: f32, to: f32| ((to * pixels_per_point).round() - (from * pixels_per_point).round()).max(1.0) as u32;
    [pixels(rect.left(), rect.right()), pixels(rect.top(), rect.bottom())]
}

#[cfg(test)]
const TEST_SCENE_SET_HOVER: &str = "scene_frame_set_hover";

fn add_callback(app: &mut TextureCraftApp, ui: &Ui, rect: Rect, mut frame: Frame, sync: &mut PendingSync) {
    #[cfg(test)]
    if frame.view_id == VIEW_3D {
        ui.data_mut(|data| data.insert_temp(egui::Id::unique(TEST_SCENE_SET_HOVER), frame.shade.set_hover.map(|highlight| highlight.set)));
    }

    let tokens = Tokens::get(ui.ctx());
    let bake = bake_visualization(app);
    frame.shade.bake = bake;
    let scene = match (sync.0.take(), app.gpu.as_mut()) {
        (Some(aux), Some(gpu_link)) => {
            let sync_start_ms = crate::now_ms();
            let scene = gpu_link.sync(&mut app.session, aux, bake, u32::from(app.ui.prefs.viewport_texture.min(2)), f64::from(app.profiler.logic_ms));
            let reports = gpu_link.take_reports();
            app.profiler.sync_ms += (crate::now_ms() - sync_start_ms) as f32;
            gpu_report_toasts(app, reports);

            // Display tiles left for later frames (the frame budget ran out), or the bake visualization still being worked out
            if app.session.caches.iter().any(|cache| !cache.upload.is_empty()) || app.gpu.as_ref().is_some_and(crate::gpu::GpuLink::bake_busy) {
                ui.ctx().request_repaint();
            }
            Some(scene)
        }
        _ => None,
    };

    let Some(gpu) = &app.gpu else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            tr("No GPU renderer available"),
            egui::FontId::proportional(NO_GPU_TEXT_SIZE),
            tokens.viewport_text,
        );
        return;
    };

    let callback = ViewCallback { frame, sync: Mutex::new(scene), rect, lost: gpu.lost.clone() };
    ui.painter().add(eframe::egui_wgpu::Callback::new_paint_callback(rect, callback));
}

/// Text size of the notice a view shows without a GPU renderer
const NO_GPU_TEXT_SIZE: f32 = 14.0;

/// Width of the camera and view pickers centred at the top of a view
const VIEW_HEADER_WIDTH: f32 = 300.0;

/// Height of the view pickers' row
const VIEW_HEADER_HEIGHT: f32 = 22.0;

/// Gap between the view's top edge and its pickers
const VIEW_HEADER_TOP_GAP: f32 = 8.0;

/// Width of each of the two pickers
const VIEW_PICKER_WIDTH: f32 = 130.0;

/// Where `view_header` draws its pickers in a view of `rect`; `None` when the view is too narrow.
fn view_header_rect(rect: Rect) -> Option<Rect> {
    (rect.width() >= VIEW_HEADER_WIDTH).then(|| {
        Rect::from_min_size(rect.min + vec2((rect.width() - VIEW_HEADER_WIDTH) * 0.5, VIEW_HEADER_TOP_GAP), vec2(VIEW_HEADER_WIDTH, VIEW_HEADER_HEIGHT))
    })
}

/// Where the solo notice sits from a view's top-left corner, and its text size
const SOLO_NOTICE_OFFSET: egui::Vec2 = vec2(12.0, 12.0);
const SOLO_NOTICE_TEXT_SIZE: f32 = 12.0;

/// While a layer is soloed, a line in the view's corner says so: the view does not show the whole stack
fn solo_notice(app: &TextureCraftApp, ui: &Ui, rect: Rect) {
    let Some(solo) = app.session.solo else { return };
    let Some(layer) = app.session.doc.project.layers.get(&solo.layer) else { return };
    let text = if solo.ignore_mask { trf("Solo: {layer} (mask ignored)", &[("layer", &layer.name)]) } else { trf("Solo: {layer}", &[("layer", &layer.name)]) };
    let tokens = Tokens::get(ui.ctx());
    ui.painter_at(rect).text(
        rect.left_top() + SOLO_NOTICE_OFFSET,
        egui::Align2::LEFT_TOP,
        text,
        egui::FontId::proportional(SOLO_NOTICE_TEXT_SIZE),
        tokens.warning,
    );
}

/// The camera and view pickers at the top of a view, left out when the view is too narrow to hold them
fn view_header(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect, id: &str) {
    let Some(header) = view_header_rect(rect) else { return };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(header).layout(egui::Layout::left_to_right(egui::Align::Center)));

    // With no project open there is no camera to pick: the view keeps only its view-mode picker
    let project_open = app.session.project_open;
    let current = texturecraft_engine::camera_cmds::current(&app.session);
    let names: Vec<String> = app.session.doc.project.cameras.iter().map(|camera| camera.name.clone()).collect();
    let shown = current.and_then(|index| names.get(index)).map_or(tr(DEFAULT_CAMERA), String::as_str);
    if project_open {
        egui::ComboBox::from_id_salt(format!("{id}_cam")).width(VIEW_PICKER_WIDTH).selected_text(shown).show_ui(&mut child, |ui| {
            if ui.selectable_label(current.is_none(), tr(DEFAULT_CAMERA)).clicked() {
                app.run("camera.select", json!({}));
            }
            for (index, name) in names.iter().enumerate() {
                if ui.selectable_label(current == Some(index), name).clicked() {
                    app.run("camera.select", json!({"index": index}));
                }
            }
            ui.separator();
            if ui.selectable_label(false, tr("Save view as camera")).clicked() {
                app.run("camera.add", json!({}));
            }
        });
    }

    // The 2D view of a UV tile project: a checkable button left of the view mode, with no tooltip, that shows or hides the tile numbers and the hover label
    if id == "v2d" && uv_tile_project(app) {
        child.spacing_mut().item_spacing.x = UV_TILE_INFO_SPACING;
        let shown = app.ui.uv_tile_info;
        let button = crate::widgets::icon_button_without_tooltip(&mut child, egui_phosphor::regular::DOTS_NINE, UV_TILE_INFO_NAME, shown, VIEW_HEADER_HEIGHT);
        if !shown {
            // Crossed out while the numbers are hidden
            let (from, to) = (button.rect.left_bottom() + vec2(5.0, -5.0), button.rect.right_top() + vec2(-5.0, 5.0));
            child.painter().line_segment([from, to], Stroke::new(1.5, Tokens::get(child.ctx()).text));
        }
        if button.clicked() {
            app.ui.uv_tile_info = !shown;
        }
    }
    view_mode_picker(app, &mut child, &format!("{id}_mode"), VIEW_PICKER_WIDTH);
}

/// The tile info button's name for screen readers: the reference's object name, since it has no text or tooltip
const UV_TILE_INFO_NAME: &str = "uvtile_info_menu_button";

/// Gap between the header's widgets while the tile info button is in it, so that both pickers and the button fit the header
const UV_TILE_INFO_SPACING: f32 = 4.0;

/// Whether the open project's active texture set is a UV tile set
pub(crate) fn uv_tile_project(app: &TextureCraftApp) -> bool {
    app.session.project_open && app.session.doc.project.texture_sets.get(app.session.ts()).is_some_and(|set| set.tile_grid().is_some())
}

/// The view-mode picker of a view's header and of Display Settings
pub(crate) fn view_mode_picker(app: &mut TextureCraftApp, child: &mut Ui, id_salt: &str, width: f32) {
    crate::view_mode_picker::show(app, child, id_salt, width);
}

/// Height above a view's bottom edge of the "Mask" label, clear of the axis gizmo
const MASK_BADGE_RAISE: f32 = 70.0;

/// Left inset of the "Mask" label
const MASK_BADGE_INSET: f32 = 14.0;

/// Text size of the "Mask" label
const MASK_BADGE_TEXT_SIZE: f32 = 15.0;

/// Whether the quick mask replaces whatever view mode is chosen: editing it in Painting mode, unless picking an ID colour
fn quick_mask_shown(app: &TextureCraftApp) -> bool {
    app.ui.mode == crate::state::Mode::Painting && app.session.quick_mask.editing && app.picking_id_color.is_none()
}

/// The layer whose mask the view really displays: none while the quick mask takes over the view
fn mask_view_layer(app: &TextureCraftApp) -> Option<texturecraft_engine::core::LayerId> {
    match app.session.view {
        ViewMode::Mask(layer) if !quick_mask_shown(app) => Some(layer),
        _ => None,
    }
}

/// The label that says a view shows one layer's mask alone and how to leave it
fn mask_view_badge(app: &TextureCraftApp, ui: &Ui, rect: Rect) {
    let Some(layer) = mask_view_layer(app) else { return };
    let tk = Tokens::get(ui.ctx());
    let name = app.session.doc.project.layers.get(&layer).map(|layer| layer.name.as_str()).unwrap_or_default();
    let text = trf("MASK: {layer}", &[("layer", &name)]);
    ui.painter_at(rect).text(
        pos2(rect.left() + MASK_BADGE_INSET, rect.bottom() - MASK_BADGE_RAISE),
        egui::Align2::LEFT_BOTTOM,
        text,
        egui::FontId::proportional(MASK_BADGE_TEXT_SIZE),
        tk.viewport_accent,
    );
}

/// The screen directions of the U axis (right) and the V axis (up) in a 2D view turned `rotation` radians counter-clockwise
fn uv_axis_directions(rotation: f32) -> (egui::Vec2, egui::Vec2) {
    let (sin, cos) = rotation.sin_cos();
    (vec2(cos, -sin), vec2(-sin, -cos))
}

/// Distance of the 2D view's axis gizmo origin from the view's right and bottom edges: room for the axis letters in any rotation
const UV_GIZMO_INSET: f32 = 36.0;

fn axis_gizmo(ui: &Ui, rect: Rect, app: &TextureCraftApp, uv_rotation: Option<f32>) {
    let tk = Tokens::get(ui.ctx());
    let o = pos2(rect.left() + 28.0, rect.bottom() - 28.0);
    let p = ui.painter();
    if let Some(rotation) = uv_rotation {
        // The 2D view keeps it in the bottom-right corner, the label of the texture set sits bottom left
        let o = pos2(rect.right() - UV_GIZMO_INSET, rect.bottom() - UV_GIZMO_INSET);

        // The U and V axes turn with the 2D view
        let (u, v) = uv_axis_directions(rotation);
        p.line_segment([o, o + u * 16.0], Stroke::new(1.5, Color32::from_rgb(0xd8, 0x4a, 0x3a)));
        p.line_segment([o, o + v * 16.0], Stroke::new(1.5, Color32::from_rgb(0x5c, 0xc0, 0x4a)));
        p.text(o + u * 22.0, egui::Align2::CENTER_CENTER, "U", egui::FontId::proportional(9.0), tk.viewport_text);
        p.text(o + v * 22.0, egui::Align2::CENTER_CENTER, "V", egui::FontId::proportional(9.0), tk.viewport_text);
        return;
    }

    let view = app.session.camera.view();
    for (axis, color, name) in [
        (Vec3::X, Color32::from_rgb(0xd8, 0x4a, 0x3a), "X"),
        (Vec3::Y, Color32::from_rgb(0x5c, 0xc0, 0x4a), "Y"),
        (Vec3::Z, Color32::from_rgb(0x4a, 0x80, 0xe0), "Z"),
    ] {
        let d = view.transform_vector3(axis);
        let end = o + vec2(d.x, -d.y) * 16.0;
        p.line_segment([o, end], Stroke::new(1.5, color));
        p.text(end + vec2(d.x, -d.y) * 6.0, egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(9.0), tk.viewport_text);
    }
}

/// Camera turn per point of a drag (radians)
pub(crate) const ORBIT_RADIANS_PER_POINT: f32 = 0.008;

/// Camera zoom per point of an Alt+right drag
pub(crate) const DRAG_ZOOM_PER_POINT: f32 = 0.01;

/// Camera zoom per point of mouse wheel
const WHEEL_ZOOM_PER_POINT: f32 = 0.0025;

/// Environment light turn per point of a Shift+right drag (degrees)
const LIGHT_DEGREES_PER_POINT: f32 = 0.5;

/// Brush size change per point of Shift+wheel, as a fraction of the size
const WHEEL_BRUSH_SIZE_PER_POINT: f32 = 0.002;

/// Stencil turn per point of an S+left drag (degrees)
const STENCIL_DEGREES_PER_POINT: f32 = 0.4;

/// Step of an S+Shift+left drag's turn (degrees)
const STENCIL_SNAP_DEGREES: f32 = 15.0;

/// Stencil scale change per point of an S+right drag, as a fraction of the scale
const STENCIL_SCALE_PER_POINT: f32 = 0.005;

/// Smallest stencil scale an S+right drag reaches
const MIN_STENCIL_SCALE: f32 = 0.05;

/// Largest stencil scale an S+right drag reaches
const MAX_STENCIL_SCALE: f32 = 10.0;

/// Smallest viewport the camera projects into, so a collapsed view keeps a usable aspect (points)
const MIN_CAMERA_VIEWPORT: f32 = 16.0;

/// Remember the model point under viewport pixel `screen` as the start of the next Shift+click line
fn remember_line_anchor(app: &mut TextureCraftApp, screen: Vec2) {
    if let Some(hit) = app.session.pick(screen, app.session.viewport) {
        app.last_paint_point = Some(crate::uv_paint_point::SurfacePaintPoint::new(&app.session, hit.pos));
    }
}

/// Where the remembered line start is in the view now (viewport pixels), also when the model hides it
fn line_anchor_screen(app: &TextureCraftApp, rect: Rect) -> Option<Vec2> {
    let pos = app.last_paint_point?.position_for(&app.session)?;
    let screen = project(app, rect, pos)?;
    Some(Vec2::new(screen.x - rect.left(), screen.y - rect.top()))
}

/// Alt+Shift+left drag: orbit in 90° steps. The drag's own yaw and pitch follow the pointer; the
/// camera takes the nearest step, with the pitch held inside the orbit's ±89° limit.
fn snapped_orbit(app: &mut TextureCraftApp, delta: egui::Vec2) {
    let step = std::f32::consts::FRAC_PI_2;
    let limit = 89f32.to_radians();
    let camera = &mut app.session.camera;
    let (yaw, pitch) = app.orbit_snap.get_or_insert((camera.yaw, camera.pitch));
    let radians_per_point = ORBIT_RADIANS_PER_POINT * app.ui.prefs.orbit_scale();
    *yaw -= delta.x * radians_per_point;
    *pitch = (*pitch + delta.y * radians_per_point).clamp(-step, step);

    // A free camera is put level on the step
    camera.set_angles((*yaw / step).round() * step, ((*pitch / step).round() * step).clamp(-limit, limit));
}

/// Orbit the 3D view by a drag of `delta` points at the Settings rotation speed.
pub(crate) fn orbit(app: &mut TextureCraftApp, delta: egui::Vec2) {
    let radians_per_point = ORBIT_RADIANS_PER_POINT * app.ui.prefs.orbit_scale();
    app.session.camera.orbit(delta.x * radians_per_point, delta.y * radians_per_point);
}

fn view_3d(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect, sync: &mut PendingSync) {
    let response = view_widget(ui, rect, view_3d_id(), Sense::click_and_drag());
    app.viewport_rect = rect;
    let size = rect.size();
    app.session.viewport = Vec2::new(size.x.max(MIN_CAMERA_VIEWPORT), size.y.max(MIN_CAMERA_VIEWPORT));
    let (alt, shift, snap) = ui.input(|input| (input.modifiers.alt, input.modifiers.shift, input.modifiers.shift && input.modifiers.command));
    let local = |pointer: egui::Pos2| Vec2::new(pointer.x - rect.left(), pointer.y - rect.top());
    let adjusting = brush_drag(app, ui, &response);
    let ctrl = ui.input(|input| input.modifiers.ctrl);

    // The move pad owns both pointer and wheel until release
    let holding = crate::move_on_model::holding(app, ui.ctx());
    let navigating = !adjusting && !holding && (alt || ui.input(|input| input.pointer.middle_down() || input.pointer.secondary_down()));

    // Navigation; S + drag moves the projection stencil instead, below
    let delta = ui.input(|input| input.pointer.delta());
    let s_held =
        app.ui.mode == crate::state::Mode::Painting && ui.input(|input| input.key_down(STENCIL_KEY)) && !crate::framework::widget_has_keyboard(ui.ctx());

    // Only an equipped Projection tool that can act moves its stencil: it isn't drawn otherwise
    let stencil_moves = tool_acts(app) && app.session.tool.tool == Tool::Projection;
    let stencil_drag = s_held && stencil_moves && app.session.tool.stencil.is_some();
    if stencil_drag || adjusting || holding {
        // The stencil, the brush or the held Move on model button changes instead of the camera
    } else if response.dragged_by(PointerButton::Middle) {
        app.camera_glide = None;
        app.session.camera.pan(delta.x / size.y, delta.y / size.y);
    } else if alt && ctrl && response.dragged_by(PointerButton::Primary) {
        // Ctrl+Alt+left is "Camera translate (alt.)": it pans like the middle button
        app.camera_glide = None;
        app.session.camera.pan(delta.x / size.y, delta.y / size.y);
    } else if alt && response.dragged_by(PointerButton::Primary) {
        // Navigating takes over from a glide in progress, which would put the camera back
        app.camera_glide = None;
        if shift {
            snapped_orbit(app, delta);
        } else {
            orbit(app, delta);
        }
    } else if response.dragged_by(PointerButton::Secondary) {
        app.camera_glide = None;
        if alt {
            app.session.camera.zoom(-delta.y * DRAG_ZOOM_PER_POINT * app.ui.prefs.drag_zoom_scale());
        } else if shift {
            let display = &mut app.session.doc.project.display;
            display.env_rotation = (display.env_rotation + delta.x * LIGHT_DEGREES_PER_POINT).rem_euclid(360.0);
        } else {
            orbit(app, delta);
        }
    }

    // The next snapping drag starts again from where the camera is
    if !(alt && shift && response.dragged_by(PointerButton::Primary)) {
        app.orbit_snap = None;
    }

    let camera_input = !holding && !ShownGizmo::dragging(ui.ctx());
    if response.hovered() && camera_input {
        size_brush_by_ctrl_wheel(app, ui);
        if ui.input(|input| input.multi_touch().is_none()) {
            let pinch = trackpad_pinch(ui);
            if pinch != 1.0 {
                app.session.camera.zoom(pinch.ln());
            }
        }

        let scroll = wheel_scroll(ui, shift);
        if scroll != 0.0 {
            if shift && wheel_sizes_brush(app) {
                scale_brush_by_wheel(app, scroll);
            } else {
                app.camera_glide = None;
                app.session.camera.zoom(scroll * WHEEL_ZOOM_PER_POINT * app.ui.prefs.wheel_zoom_scale());
            }
        }
    }

    // Touch: two fingers orbit and pinch zooms, three pan; a second finger cancels the first one's stroke
    let touch = ui.input(|input| input.multi_touch()).filter(|_| !holding && (response.contains_pointer() || response.dragged()));
    if let Some(gesture) = touch.filter(|_| camera_input) {
        if app.session.stroke_active() {
            app.session.cancel_stroke();
        }

        app.camera_glide = None;
        let moved = gesture.translation_delta;
        if gesture.num_touches >= 3 {
            app.session.camera.pan(moved.x / size.y, moved.y / size.y);
        } else {
            orbit(app, moved);
        }
        if gesture.zoom_delta.is_finite() && gesture.zoom_delta > 0.0 {
            app.session.camera.zoom(gesture.zoom_delta.ln());
        }
    }

    let raw_id = ui.scope_id().with("stencil_snap_raw");
    if !(s_held && shift && response.dragged_by(PointerButton::Primary)) {
        ui.data_mut(|data| data.remove::<f32>(raw_id));
    }

    // With S held, left rotates, Shift snaps, right scales and middle moves the stencil
    if s_held
        && stencil_moves
        && let Some(stencil) = app.session.tool.stencil.as_mut()
    {
        if response.dragged_by(PointerButton::Primary) {
            if shift {
                // The unsnapped angle is kept so slow drags still reach the next step
                let raw: f32 = ui.data(|data| data.get_temp(raw_id)).unwrap_or(stencil.rotation) + delta.x * STENCIL_DEGREES_PER_POINT;
                ui.data_mut(|data| data.insert_temp(raw_id, raw));
                stencil.rotation = ((raw / STENCIL_SNAP_DEGREES).round() * STENCIL_SNAP_DEGREES).rem_euclid(360.0);
            } else {
                stencil.rotation = (stencil.rotation + delta.x * STENCIL_DEGREES_PER_POINT).rem_euclid(360.0);
            }
        } else if response.dragged_by(PointerButton::Secondary) {
            stencil.scale = (stencil.scale * (1.0 + delta.x * STENCIL_SCALE_PER_POINT)).clamp(MIN_STENCIL_SCALE, MAX_STENCIL_SCALE);
        } else if response.dragged_by(PointerButton::Middle) {
            stencil.center[0] += delta.x / size.x * 2.0;
            stencil.center[1] -= delta.y / size.y * 2.0;
        }
    }

    // Clone source: V + click
    let v_held = clone_source_key(app, ui);
    if v_held
        && !alt
        && (response.clicked_by(PointerButton::Primary) || response.drag_started_by(PointerButton::Primary))
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let (at, viewport) = (local(pointer), app.session.viewport);
        app.run("tool.cloneSource", json!({"x": at.x, "y": at.y, "viewport": [viewport.x, viewport.y]}));
    }

    // A 3D generator's armed point pick: a click sets the point there instead of painting
    let point_pick = crate::generator_point_pick::armed_on(ui.ctx(), &app.session);
    if let Some(pick) = point_pick {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if !alt
            && response.clicked_by(PointerButton::Primary)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let (at, viewport) = (local(pointer), app.session.viewport);
            let params = json!({"effect": pick.effect, "point": pick.point, "x": at.x, "y": at.y, "viewport": [viewport.x, viewport.y]});

            // A click off the mesh keeps it armed for the next
            if app.run("effect.generator.pickPoint", params).is_some() {
                crate::generator_point_pick::set(ui.ctx(), None);
            }
        }
    }

    // Depth of field focus pick: a click focuses there instead of painting
    let focus_pick = focus_picking(ui.ctx()) || point_pick.is_some();
    if focus_picking(ui.ctx()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        if !alt
            && response.clicked_by(PointerButton::Primary)
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let (at, viewport) = (local(pointer), app.session.viewport);
            app.run("viewport.focusAt", json!({"x": at.x, "y": at.y, "viewport": [viewport.x, viewport.y]}));
            set_focus_picking(ui.ctx(), false);
        }
    }

    // A dismissing click cannot paint or place a decal through the rail popup
    let rail_popup = crate::rail_popup::blocks_paint_input(app, ui.ctx());

    crate::properties_popup::open_on_right_click(app, ui, &response, rect, s_held || v_held);

    // A press that closes an open colour picker only closes it: it neither paints nor moves a projection
    let picker = crate::color_picker::blocks_view_press(ui.ctx());
    let popup = app.properties_popup.is_some() || rail_popup || picker;

    // A pending ID color pick or eyedropper takes the next click instead of the brush
    let picking = app.picking_id_color.is_some() || app.picking_color.is_some();
    if picking {
        // The eyedropper's own pointer is drawn over the whole window (screen_pick)
        if app.picking_color.is_none() {
            crate::id_color_pick::IdColorPick::draw_cursor(ui, rect);
        }

        // The eyedropper's disc shows the colour a click here would pick
        if app.picking_color.is_some()
            && let Some(hover) = response.hover_pos()
            && let Ok(values) = app.session.color_at(local(hover), app.session.viewport)
            && let Some(color) = values.get(&texturecraft_engine::core::Channel::BaseColor)
        {
            crate::screen_pick::SwatchColor::over_model(ui.ctx(), *color);
        }
        if !alt
            && response.clicked_by(PointerButton::Primary)
            && let Some(position) = response.interact_pointer_pos()
        {
            let (pixel, viewport) = (local(position), app.session.viewport);
            match app.picking_color {
                Some(bar) => eyedrop(app, ui.ctx(), bar, pixel, viewport),
                None => crate::id_color_pick::IdColorPick::pick_at(app, json!({"x": pixel.x, "y": pixel.y, "viewport": [viewport.x, viewport.y]})),
            }
        }
    }

    // Only a press on a projection handle moves the selected fill
    let gizmo_free =
        !adjusting && !alt && !holding && !s_held && !v_held && !focus_pick && !popup && !picking && touch.is_none() && !DecalDrop::blocks_input(ui.ctx());
    let gizmo = ShownGizmo::of(app, rect);
    let began_on_handle = handle_press(ui.ctx(), |at| gizmo.as_ref().is_some_and(|gizmo| gizmo.on_handle(rect, at)));
    let frame_drag = gizmo_free && began_on_handle && ShownGizmo::input(app, ui, &response, rect, gizmo.as_ref());

    // End the gizmo drag when navigation takes over because the camera may have moved
    if !gizmo_free || !began_on_handle {
        ShownGizmo::let_go(ui.ctx());
    }

    // A drag moved the projection: its handles and outline show where it is now
    let gizmo = if frame_drag { ShownGizmo::of(app, rect) } else { gizmo };

    let projection_drag = frame_drag || holding;
    let plane_pick = !popup
        && !s_held
        && !v_held
        && !focus_pick
        && !picking
        && touch.is_none()
        && !DecalDrop::blocks_input(ui.ctx())
        && !projection_drag
        && crate::symmetry::plane_pick_input(app, ui, &response, rect);
    let symmetry_drag = !alt
        && !plane_pick
        && !adjusting
        && !s_held
        && !v_held
        && !focus_pick
        && !popup
        && !picking
        && touch.is_none()
        && !DecalDrop::blocks_input(ui.ctx())
        && !projection_drag
        && crate::symmetry::manipulator_input(app, ui, &response, rect);

    // A handle owns a new click, but a stroke already under way must still finish over it
    let symmetry_handle = !app.session.stroke_active() && crate::symmetry::manipulator_hovered(app, ui, rect);

    let gizmo_handle = gizmo_free && !app.session.stroke_active() && began_on_handle;

    // Painting
    let painting = tool_acts(app)
        && !popup
        && !alt
        && !adjusting
        && !s_held
        && !v_held
        && !focus_pick
        && !picking
        && touch.is_none()
        && !projection_drag
        && !plane_pick
        && !symmetry_drag
        && !symmetry_handle
        && !gizmo_handle
        && !DecalDrop::blocks_input(ui.ctx());
    let path_tool = app.session.tool.tool == Tool::Path;
    let viewport = app.session.viewport;
    let show = |app: &TextureCraftApp, point: Vec3| project(app, rect, point);
    let mut fill_box = None;
    if painting && path_tool {
        path_input(app, ui, &response, (rect, false), |app, pointer| app.session.pick(local(pointer), viewport).map(|hit| hit.pos), show);
    } else if painting && app.session.tool.tool == Tool::PolygonFill {
        fill_box = polygon_fill_input(
            app,
            ui,
            &response,
            |session, point| session.polygon_fill(local(point), viewport),
            |session, corner, opposite| session.drag_polygon_fill_box(local(corner), local(opposite), viewport),
        );
    } else if painting {
        let pressure = pen_pressure(ui);
        if response.drag_started_by(PointerButton::Primary) || response.clicked_by(PointerButton::Primary) {
            // Shift+click paints a straight line from where the last stroke ended on the model; Ctrl+Shift snaps its angle
            let line_from = if shift { line_anchor_screen(app, rect) } else { None };
            if let Some(pointer) = response.interact_pointer_pos() {
                let pointer = snapped_end(line_from.filter(|_| snap).map(|from| pos2(rect.left() + from.x, rect.top() + from.y)), pointer);
                let start = line_from.unwrap_or(local(pointer));
                match app.session.begin_stroke(start, app.session.viewport, pressure) {
                    Ok(_) => {
                        crate::inactive_set_hint::hint_on_stroke_start(app, start);
                        if line_from.is_some() {
                            app.session.continue_stroke(local(pointer), pressure);
                        }
                        remember_line_anchor(app, local(pointer));
                    }
                    Err(error) => app.toast(tr(&error.to_string()).to_string(), true),
                }
            }
            if response.clicked_by(PointerButton::Primary)
                && let Err(error) = app.session.end_stroke()
            {
                app.toast(error.to_string(), true);
            }
        } else if response.dragged_by(PointerButton::Primary)
            && app.session.stroke_active()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            app.session.continue_stroke(local(pointer), pressure);
            remember_line_anchor(app, local(pointer));
        }
        if response.drag_stopped_by(PointerButton::Primary)
            && let Err(error) = app.session.end_stroke()
        {
            app.toast(error.to_string(), true);
        }
    }

    let set_hover = app
        .hovered_texture_set
        .filter(|&(project_id, _)| project_id == app.session.project_id && !app.ui.hide_ui && !crate::docking::input_blocked(app, ui.ctx()))
        .map(|(_, texture_set)| set_highlight(ui, texture_set));
    let frame = Frame {
        view_id: crate::viewport::VIEW_3D,
        size: frame_size_in_pixels(ui, rect),
        kind: ViewKind::Scene,
        camera: app.session.camera,
        shade: Shade { highlight: drop_highlight(app, ui, &response), set_hover, ..shade(app) },
        visible: app.session.doc.project.texture_sets.iter().map(|texture_set| texture_set.visible).collect(),
        overlay: crate::symmetry::scene_overlay(app, ui, rect),
    };
    add_callback(app, ui, rect, frame, sync);
    polygon_fill_box_overlay(ui, rect, fill_box);

    // Route shelf drops by asset type to a layer, decal, or brush
    crate::shelf::drop_target(app, ui, &response, crate::shelf::DropZone::View3d);

    // Brush cursor; while a Ctrl+drag changes the brush it stays where the drag began
    let mut symmetry_cursor = None;
    if tool_acts(app)
        && !navigating
        && response.hovered()
        && brush_tool(app.session.tool.tool)
        && (app.session.tool.tool != Tool::Path || adjusting)
        && !crate::symmetry::manipulator_hovered(app, ui, rect)
        && app.picking_id_color.is_none()
        && app.picking_color.is_none()
        && app.moving_on_model.is_none()
        && !ShownGizmo::hovered(ui, rect, gizmo.as_ref())
        && let Some(hover_pos) = if adjusting { ui.input(|input| input.pointer.press_origin()) } else { response.hover_pos() }
        && let Some(hit) = app.session.pick(local(hover_pos), app.session.viewport)
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        let (bounds_min, bounds_max) = app.viewport_mesh_bounds.get(&app.session.mesh);
        let radius = app.session.tool.brush.size * (bounds_max - bounds_min).length();
        brush_cursor(app, ui, rect, hit.pos, hit.nrm, radius);
        symmetry_cursor = Some((hit.pos, hit.nrm));
        if app.session.tool.tool == Tool::Clone
            && let Some(source) = app.session.clone_source_point(Some(hit.uv))
        {
            clone_marker(app, ui, rect, source);
        }

        // Shift: preview the straight line Shift+click paints from the last stroke (snapped with Ctrl)
        if shift
            && !path_tool
            && let Some(from) = line_anchor_screen(app, rect)
        {
            let from = pos2(rect.left() + from.x, rect.top() + from.y);
            rubber_band(ui, rect, &[from, snapped_end(snap.then_some(from), hover_pos)]);
        }
    }

    if tool_acts(app) && path_tool {
        path_overlay(app, ui, rect, &response, false, show);
    }
    crate::symmetry::overlay(app, ui, rect, symmetry_cursor);
    stencil_overlay(app, ui, rect);
    if let Some(gizmo) = &gizmo {
        gizmo.overlay(app, ui, rect);
    }
    brush_readout(app, ui, &response, adjusting);
    if app.session.quick_mask.editing && app.picking_id_color.is_none() {
        ui.painter_at(rect).text(
            rect.left_top() + QUICK_MASK_NOTICE_OFFSET,
            egui::Align2::LEFT_TOP,
            tr("Editing quick mask: paint where painting is allowed (U to finish)"),
            egui::FontId::proportional(QUICK_MASK_NOTICE_TEXT_SIZE),
            QUICK_MASK_NOTICE_COLOR,
        );
    }
    if app.ui.mode == crate::state::Mode::Baking {
        crate::bake_mode::bake_visualization_panel(app, ui, rect);
    }
    solo_notice(app, ui, rect);
    view_header(app, ui, rect, "v3d");
    mask_view_badge(app, ui, rect);
    if app.ui.prefs.show_world_axes {
        axis_gizmo(ui, rect, app, None);
    }

    crate::nav_buttons::show(app, ui, rect, crate::nav_buttons::NavView::Scene);
}

/// Where the quick-mask editing notice sits from the 3D view's top-left corner, its text size and its warning red
const QUICK_MASK_NOTICE_OFFSET: egui::Vec2 = vec2(12.0, 40.0);
const QUICK_MASK_NOTICE_TEXT_SIZE: f32 = 12.0;
const QUICK_MASK_NOTICE_COLOR: Color32 = Color32::from_rgb(0xe8, 0x6a, 0x6a);

/// What bake mode's 3D view draws besides the low poly (nothing in the other modes)
fn bake_visualization(app: &TextureCraftApp) -> Option<texturecraft_render::BakeVisualization> {
    // Setup meshes can cover the low-poly map preview; keep the switches for when setup resumes
    if app.ui.mode != crate::state::Mode::Baking || app.session.bake_preview_active() {
        return None;
    }

    let mut visualization = app.ui.bake_visualization.visualization(app.session.ts());
    visualization.matching_error.shown &= crate::bake_mode::matching_error_applies(&app.session);
    Some(visualization)
}

/// Draw the projection stencil in Paint Mode
fn stencil_overlay(app: &mut TextureCraftApp, ui: &Ui, rect: Rect) {
    if !tool_acts(app) || app.session.tool.tool != Tool::Projection {
        return;
    }
    let Some(stencil) = app.session.tool.stencil.clone() else { return };
    let Some(image) = app.session.images.get(&stencil.image).cloned() else { return };

    // Where the stencil's corners land on screen
    let texture = app.thumbs.stencil(ui.ctx(), &stencil.image, &image);
    let aspect = rect.width() / rect.height().max(1.0);
    let (center, half_size, rotation) = stencil.placement(image.width as f32 / image.height.max(1) as f32, aspect);
    let to_screen = |x: f32, y: f32| pos2(rect.left() + (x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - y * 0.5) * rect.height());
    let (sine, cosine) = rotation.sin_cos();
    let corner = |x: f32, y: f32| {
        let (x, y) = (x * half_size[0], y * half_size[1]);
        to_screen(center[0] + x * cosine - y * sine, center[1] + x * sine + y * cosine)
    };

    let tint = Color32::from_white_alpha((stencil.opacity * f32::from(u8::MAX)) as u8);
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for (x, y, u, v) in [(-1.0, 1.0, 0.0, 0.0), (1.0, 1.0, 1.0, 0.0), (1.0, -1.0, 1.0, 1.0), (-1.0, -1.0, 0.0, 1.0)] {
        mesh.vertices.push(egui::epaint::Vertex { pos: corner(x, y), uv: pos2(u, v), color: tint });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    let painter = ui.painter_at(rect);
    painter.add(egui::Shape::mesh(mesh));

    let outline = vec![corner(-1.0, 1.0), corner(1.0, 1.0), corner(1.0, -1.0), corner(-1.0, -1.0), corner(-1.0, 1.0)];
    let outline_stroke = Stroke::new(STENCIL_OUTLINE_STROKE, Color32::from_white_alpha(STENCIL_OUTLINE_ALPHA));
    painter.add(egui::Shape::dashed_line(&outline, outline_stroke, STENCIL_OUTLINE_DASH, STENCIL_OUTLINE_GAP));
}

/// A world point on screen in the 3D view `rect` (None behind the camera)
pub(crate) fn project(app: &TextureCraftApp, rect: Rect, point: Vec3) -> Option<Pos2> {
    let ndc = app.session.camera.project(point, rect.width() / rect.height().max(1.0))?;
    Some(pos2(rect.left() + (ndc.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - ndc.y * 0.5) * rect.height()))
}

/// The camera ray through screen point `p` of the 3D view `rect`
pub(crate) fn view_ray(app: &TextureCraftApp, rect: Rect, p: Pos2) -> texturecraft_engine::geom::Ray {
    let ndc = Vec2::new((p.x - rect.left()) / rect.width() * 2.0 - 1.0, 1.0 - (p.y - rect.top()) / rect.height() * 2.0);
    app.session.camera.ray(ndc, rect.width() / rect.height().max(1.0))
}

/// Whether the window lost the focus (Alt+Tab): egui keeps a held button down with no release to come, so a drag ends here as on a release
pub(crate) fn focus_lost(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.viewport().focused == Some(false) || input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))))
}

/// Control point and curve grab radius (points)
const HANDLE_GRAB: f32 = 8.0;
/// Opacity of the selected layer's other stored paths
const PATH_STORED_OPACITY: f32 = 0.55;
/// Opacity of a curve where the mesh hides it
const PATH_HIDDEN_OPACITY: f32 = 0.5;
/// A control point counts as hidden when the surface under it is this much closer (relative)
const PATH_HIDDEN_DEPTH: f32 = 0.01;
/// The Path tool's pointer glyph size (points)
const PATH_CURSOR_SIZE: f32 = 18.0;
/// A path's line and the dark outline under it (points)
const PATH_LINE_WIDTH: f32 = 1.4;
const PATH_OUTLINE_WIDTH: f32 = 3.0;
/// Where the view hint sits from the view's bottom left corner, and its text size (points)
const PATH_HINT_OFFSET: egui::Vec2 = vec2(56.0, -12.0);
const PATH_HINT_FONT_SIZE: f32 = 12.0;
/// The pen pointer's drop shadow, and the space before its insert plus (points)
const PATH_CURSOR_SHADOW_OFFSET: egui::Vec2 = vec2(1.0, 1.0);
const PATH_CURSOR_PLUS_GAP: f32 = 1.0;
/// The insert plus beside the pen pointer, relative to the pen's size
const PATH_CURSOR_PLUS_SCALE: f32 = 0.6;
/// Space kept right of the view hint, and its narrowest wrap width (points)
const PATH_HINT_MARGIN: f32 = 8.0;
const PATH_HINT_MIN_WIDTH: f32 = 80.0;
/// How much a control point marker grows under the pointer, and when selected (points)
const PATH_POINT_HOVER_GROWTH: f32 = 1.5;
const PATH_POINT_SELECTED_GROWTH: f32 = 0.5;
/// Control point marker radius (points)
const PATH_POINT_RADIUS: f32 = 3.0;
/// Curve sampling step on screen (points)
const PATH_SCREEN_STEP: f32 = 3.0;

/// Path tool keys: Enter applies the path, Backspace removes the selected (or last) point; Esc is a global shortcut
fn path_keys(app: &mut TextureCraftApp, ui: &Ui) {
    #[cfg(not(target_arch = "wasm32"))]
    if crate::updates_ui::blocks_input(app) {
        return;
    }

    if !tool_acts(app) || app.session.tool.tool != Tool::Path || app.session.tool.path.points.is_empty() || crate::framework::widget_has_keyboard(ui.ctx()) {
        return;
    }

    let (enter, backspace) =
        ui.input_mut(|input| (input.consume_key(egui::Modifiers::NONE, egui::Key::Enter), input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)));
    if enter {
        app.run("tool.path", json!({"commit": true}));
    }
    if backspace {
        app.session.remove_selected_path_point();
    }
}

/// The selected layer's stored paths except the edited one: (path sublayer, index, path)
fn other_paths(app: &TextureCraftApp) -> Vec<(texturecraft_engine::core::LayerId, usize, texturecraft_engine::PathState)> {
    let s = &app.session;
    let Some(layer) = s.selected_layer().and_then(|id| s.doc.project.layer(id).ok()) else { return Vec::new() };
    let effects = layer.content_effects.iter().chain(layer.mask.iter().flat_map(|m| m.effects.iter()));
    effects
        .flat_map(|e| s.stored_path_states(e.id).into_iter().enumerate().map(move |(i, path)| (e.id, i, path)))
        .filter(|(e, i, _)| s.tool.path.editing != Some((*e, *i)))
        .collect()
}

/// The screen curve through `points` per segment, each starting on its first point
fn screen_segments(points: &[Pos2], corners: &[bool], closed: bool) -> Vec<Vec<Pos2>> {
    let pts: Vec<Vec2> = points.iter().map(|p| Vec2::new(p.x, p.y)).collect();
    texturecraft_engine::paint::spline_segments_with_corners(&pts, corners, closed, PATH_SCREEN_STEP)
        .into_iter()
        .zip(points)
        .map(|(samples, first)| std::iter::once(*first).chain(samples.into_iter().map(|v| pos2(v.x, v.y))).collect())
        .collect()
}

/// A path's curve in a view per segment: screen position and whether the mesh leaves it visible (None off the mesh or screen)
type ScreenCurve = Vec<Vec<Option<(Pos2, bool)>>>;

/// The nearest segment of `curve` within grab distance of `at`, with the nearest position on it
fn segment_near(curve: &ScreenCurve, at: Pos2) -> Option<(usize, Pos2)> {
    let nearest = |segment: &Vec<Option<(Pos2, bool)>>| {
        segment.windows(2).filter_map(|w| Some(closest_on_line(w[0]?.0, w[1]?.0, at))).min_by(|a, b| a.distance(at).total_cmp(&b.distance(at)))
    };
    curve
        .iter()
        .enumerate()
        .filter_map(|(i, segment)| nearest(segment).map(|p| (i, p)))
        .filter(|(_, p)| p.distance(at) < HANDLE_GRAB)
        .min_by(|a, b| a.1.distance(at).total_cmp(&b.1.distance(at)))
}

/// `path`'s curve in a view; a 3D curve lies where its paint lands, and without `depth_test` (hit tests) every sample counts as visible
fn curve_on_screen(
    app: &TextureCraftApp,
    ui: &Ui,
    (rect, uv, depth_test): (Rect, bool, bool),
    path: &texturecraft_engine::PathState,
    stored: Option<(texturecraft_engine::core::LayerId, usize)>,
    show: &impl Fn(&TextureCraftApp, Vec3) -> Option<Pos2>,
) -> ScreenCurve {
    if uv {
        let Some(points) = path.points.iter().map(|p| show(app, *p)).collect::<Option<Vec<_>>>() else { return Vec::new() };
        return screen_segments(&points, &path.corners, path.closed)
            .into_iter()
            .map(|segment| segment.into_iter().map(|at| Some((at, true))).collect())
            .collect();
    }

    let s = &app.session;
    let surface = stored
        .and_then(|(eid, index)| stored_surface(app, ui, eid, index))
        .unwrap_or_else(|| Arc::new(s.path_surface_with_corners(&path.points, &path.corners, path.closed, s.camera, s.viewport)));
    surface
        .iter()
        .map(|segment| segment.iter().map(|p| p.and_then(|p| show(app, p).map(|at| (at, !depth_test || path_point_visible(app, rect, p, at))))).collect())
        .collect()
}

/// Sample the stored curve once while visible, invalidating it when the path or mesh changes
fn stored_surface(app: &TextureCraftApp, ui: &Ui, eid: texturecraft_engine::core::LayerId, index: usize) -> Option<SurfaceCurve> {
    ui.memory_mut(|memory| memory.caches.cache::<PathSurfaceCache>().get(&app.session, eid, index))
}

/// The point of the segment `a`–`b` nearest to `p`
fn closest_on_line(a: Pos2, b: Pos2, p: Pos2) -> Pos2 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    a + ab * t
}

/// What `at` is over in a view (`uv`: the 2D view); `show` maps a path point to the screen
fn path_hit(app: &TextureCraftApp, ui: &Ui, (rect, uv): (Rect, bool), at: Pos2, show: &impl Fn(&TextureCraftApp, Vec3) -> Option<Pos2>) -> PathHit {
    let path = &app.session.tool.path;
    if path.uv == uv {
        if let Some(i) = path.points.iter().position(|p| show(app, *p).is_some_and(|s| s.distance(at) < HANDLE_GRAB)) {
            return PathHit::Point(i);
        }
        if let Some((i, on)) = segment_near(&curve_on_screen(app, ui, (rect, uv, false), path, path.editing, show), at) {
            return PathHit::Curve(i, on);
        }
    }
    for (eid, i, other) in other_paths(app) {
        if other.uv == uv
            && (other.points.iter().any(|p| show(app, *p).is_some_and(|s| s.distance(at) < HANDLE_GRAB))
                || segment_near(&curve_on_screen(app, ui, (rect, uv, false), &other, Some((eid, i)), show), at).is_some())
        {
            return PathHit::Stored(eid, i);
        }
    }
    PathHit::Miss
}

/// Path tool presses in a view: select and drag a point, insert on the curve, open another path, close on the first point, or add a point
fn path_input(
    app: &mut TextureCraftApp,
    ui: &Ui,
    resp: &egui::Response,
    (rect, uv): (Rect, bool),
    place: impl Fn(&TextureCraftApp, Pos2) -> Option<Vec3>,
    show: impl Fn(&TextureCraftApp, Vec3) -> Option<Pos2>,
) {
    let drag_id = resp.id.with("path_drag");
    let clicked = resp.clicked_by(PointerButton::Primary);
    if (clicked || resp.drag_started_by(PointerButton::Primary))
        && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        let path = &app.session.tool.path;
        let (closed, len) = (path.closed, path.points.len());
        let corner = ui.input(|i| i.modifiers.shift);

        // Points live in one view's coordinates; a press in the other view keeps them and says why
        let other_view = (len > 0 || path.editing.is_some()) && path.uv != uv;
        let hit = if other_view { PathHit::Miss } else { path_hit(app, ui, (rect, uv), at, &show) };

        let grab = match hit {
            _ if other_view => {
                let hint =
                    if path.uv { "Finish or cancel the path first; it is in the 2D view" } else { "Finish or cancel the path first; it is in the 3D view" };
                if !app.toasts.iter().any(|t| t.text == tr(hint)) {
                    app.toast(tr(hint), false);
                }
                None
            }
            PathHit::Point(0) if clicked && !closed && len >= 3 => {
                app.session.set_path_closed(true);
                None
            }
            PathHit::Point(i) => {
                app.session.select_path_point(i);
                Some(i)
            }
            PathHit::Stored(eid, index) => {
                app.run("path.edit", json!({"effect": eid.0, "index": index}));
                None
            }
            hit => match place(app, if let PathHit::Curve(_, on) = hit { on } else { at }) {
                Some(p) => match hit {
                    // Shift+click places a corner point
                    PathHit::Curve(i, _) if corner => app.session.insert_path_corner(i + 1, p),
                    PathHit::Curve(i, _) => app.session.insert_path_point(i + 1, p),
                    _ if corner => app.session.append_path_corner(p, uv),
                    _ => app.session.append_path_point(p, uv),
                },
                None => {
                    app.toast(tr("Place path points on the mesh"), false);
                    None
                }
            },
        };
        if let Some(i) = grab {
            ui.data_mut(|d| d.insert_temp(drag_id, (i, true)));
        }
    }

    if resp.dragged_by(PointerButton::Primary)
        && let Some((index, begin_drag)) = ui.data(|data| data.get_temp::<(usize, bool)>(drag_id))
        && let Some(p) = resp.interact_pointer_pos().and_then(|pos| place(app, pos))
        && app.session.move_path_point(index, p, begin_drag)
    {
        ui.data_mut(|data| data.insert_temp(drag_id, (index, false)));
    }

    if !resp.is_pointer_button_down_on() {
        ui.data_mut(|data| data.remove::<(usize, bool)>(drag_id));
    }
}

/// Is path point `p` (on screen at `at`) in front of the mesh in the 3D view `rect`?
fn path_point_visible(app: &TextureCraftApp, rect: Rect, p: Vec3, at: Pos2) -> bool {
    let eye = app.session.camera.eye();
    let local = Vec2::new(at.x - rect.left(), at.y - rect.top());
    app.session.pick(local, app.session.viewport).is_none_or(|hit| hit.pos.distance(eye) >= p.distance(eye) * (1.0 - PATH_HIDDEN_DEPTH))
}

/// Draw `curve` in `color` over a dark outline, as a faint line where the mesh hides it
fn draw_path_curve(painter: &egui::Painter, curve: &ScreenCurve, color: Color32) {
    let draw = |run: &mut Vec<Pos2>, shown: bool| {
        if run.len() > 1 {
            // Hidden parts are a ghost line: no outline, half strength
            if shown {
                painter.add(egui::Shape::line(run.clone(), Stroke::new(PATH_OUTLINE_WIDTH, Color32::from_black_alpha(140))));
            }
            painter.add(egui::Shape::line(
                std::mem::take(run),
                Stroke::new(PATH_LINE_WIDTH, if shown { color } else { color.gamma_multiply(PATH_HIDDEN_OPACITY) }),
            ));
        }
        run.clear();
    };
    for segment in curve {
        // Runs of samples the view shows (or hides) alike, joined at the sample where that changes
        let (mut run, mut shown) = (Vec::new(), true);
        for sample in segment {
            match *sample {
                Some((at, visible)) => {
                    if visible != shown && !run.is_empty() {
                        run.push(at);
                        draw(&mut run, shown);
                    }
                    shown = visible;
                    run.push(at);
                }
                None => draw(&mut run, shown),
            }
        }
        draw(&mut run, shown);
    }
}

/// The layer's other paths (lit under the pointer), then the edited path with its points, the next segment's preview and the pen cursor
fn path_overlay(app: &TextureCraftApp, ui: &Ui, rect: Rect, resp: &egui::Response, uv: bool, show: impl Fn(&TextureCraftApp, Vec3) -> Option<Pos2>) {
    let tk = Tokens::get(ui.ctx());
    let painter = ui.painter_at(rect);
    let hint = tr("Path: click adds a point, click the curve inserts one, drag moves it, Enter applies, Esc cancels");

    // Wrapped to the view: the hint is longer than a narrow split view
    let at = rect.left_bottom() + PATH_HINT_OFFSET;
    let wrap = (rect.right() - at.x - PATH_HINT_MARGIN).max(PATH_HINT_MIN_WIDTH);
    let hint = painter.layout(hint.to_string(), egui::FontId::proportional(PATH_HINT_FONT_SIZE), tk.viewport_text, wrap);
    painter.galley(at - vec2(0.0, hint.size().y), hint, tk.viewport_text);

    let hover = resp.hover_pos();
    let hit = hover.map(|at| (at, path_hit(app, ui, (rect, uv), at, &show)));
    for (eid, i, other) in other_paths(app).iter().filter(|(_, _, other)| other.uv == uv) {
        let curve = curve_on_screen(app, ui, (rect, uv, true), other, Some((*eid, *i)), &show);

        // The path a click opens lights up
        let opened = matches!(hit, Some((_, PathHit::Stored(e, n))) if (e, n) == (*eid, *i));
        draw_path_curve(&painter, &curve, if opened { Color32::WHITE } else { Color32::WHITE.gamma_multiply(PATH_STORED_OPACITY) });
    }

    let path = &app.session.tool.path;
    if path.uv == uv {
        draw_path_curve(&painter, &curve_on_screen(app, ui, (rect, uv, true), path, path.editing, &show), tk.viewport_accent);
        let handles: Vec<Pos2> = path.points.iter().filter_map(|q| show(app, *q)).collect();
        if !path.closed
            && !handles.is_empty()
            && let Some((to, PathHit::Miss)) = hit
        {
            // The preview segment starts on the last placed point, which may be a corner
            let mut next = handles.clone();
            next.push(to);
            let flags: Vec<bool> = path.points.iter().enumerate().filter(|(_, q)| show(app, **q).is_some()).map(|(i, _)| path.is_corner(i)).collect();
            if let Some(segment) = screen_segments(&next, &flags, false).last() {
                rubber_band(ui, rect, segment);
            }
        }

        for (index, point) in path.points.iter().enumerate() {
            let Some(handle) = show(app, *point) else { continue };

            // Keep the point's original index when earlier points project behind the camera
            let hovered = hover.is_some_and(|m| m.distance(handle) < HANDLE_GRAB);
            let radius = if hovered { PATH_POINT_RADIUS + PATH_POINT_HOVER_GROWTH } else { PATH_POINT_RADIUS };

            // A corner point is drawn as a square, a smooth one as a circle
            let marker = |radius: f32, fill: Color32, stroke: Stroke| {
                if path.is_corner(index) {
                    painter.rect(Rect::from_center_size(handle, vec2(radius * 2.0, radius * 2.0)), 0.0, fill, stroke, egui::StrokeKind::Middle);
                } else {
                    painter.circle(handle, radius, fill, stroke);
                }
            };
            if path.selected == Some(index) {
                marker(radius + PATH_POINT_SELECTED_GROWTH, tk.viewport_accent, Stroke::new(1.0, Color32::from_black_alpha(160)));
            } else {
                marker(radius, Color32::from_black_alpha(110), Stroke::new(1.0, Color32::from_white_alpha(if hovered { 230 } else { 150 })));
            }
        }
    }

    if let Some((at, hit)) = hit
        && !ui.input(|i| i.modifiers.command)
    {
        if let PathHit::Curve(_, on) = hit {
            // Where the click inserts its point
            painter.circle(on, PATH_POINT_RADIUS, Color32::from_black_alpha(60), Stroke::new(1.0, Color32::from_white_alpha(110)));
        }
        path_cursor(ui, &painter, at, matches!(hit, PathHit::Curve(..)));
    }
}

/// The Path tool's pointer: a pen with its nib at `at`, and a plus when a click inserts a point
fn path_cursor(ui: &Ui, painter: &egui::Painter, at: Pos2, insert: bool) {
    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    let glyph = |icon: &str, size: f32, color: Color32| painter.layout_no_wrap(icon.to_string(), egui::FontId::proportional(size), color);
    for (offset, color) in [(PATH_CURSOR_SHADOW_OFFSET, Color32::from_black_alpha(200)), (egui::Vec2::ZERO, Color32::WHITE)] {
        // The pen's nib is the bottom left corner of the drawn glyph
        let pen = glyph(egui_phosphor::regular::PEN_NIB, PATH_CURSOR_SIZE, color);
        let origin = at + offset - pen.mesh_bounds.left_bottom().to_vec2();
        let right = origin + pen.mesh_bounds.right_bottom().to_vec2();
        painter.galley(origin, pen, color);
        if insert {
            let plus = glyph(egui_phosphor::regular::PLUS, PATH_CURSOR_SIZE * PATH_CURSOR_PLUS_SCALE, color);
            painter.galley(right + vec2(PATH_CURSOR_PLUS_GAP, 0.0) - plus.mesh_bounds.left_top().to_vec2(), plus, color);
        }
    }
}

/// Draw the Path tool's dashed curve preview
fn rubber_band(ui: &Ui, rect: Rect, line: &[Pos2]) {
    let painter = ui.painter_at(rect);
    let strokes = [Stroke::new(2.5, Color32::from_black_alpha(120)), Stroke::new(1.0, Color32::from_white_alpha(200))];
    paint_dashed_outline(&painter, line, RUBBER_BAND_DASH, RUBBER_BAND_GAP, strokes);
}

/// Draw a dashed polyline with work proportional to its visible length
pub(crate) fn paint_dashed_outline(painter: &egui::Painter, line: &[Pos2], dash_points: f32, gap_points: f32, strokes: [Stroke; 2]) {
    let clip = painter.clip_rect().expand(strokes[0].width);
    if line.iter().all(|&point| clip.contains(point)) {
        for stroke in strokes {
            painter.add(egui::Shape::dashed_line(line, stroke, dash_points, gap_points));
        }
        return;
    }

    let [shadow_stroke, light_stroke] = strokes;
    let mut shadows = Vec::new();
    let mut highlights = Vec::new();
    visible_dashes(clip, line, dash_points, gap_points, |dash| {
        shadows.push(egui::Shape::line_segment(dash, shadow_stroke));
        highlights.push(egui::Shape::line_segment(dash, light_stroke));
    });
    if !shadows.is_empty() {
        painter.add(shadows);
        painter.add(highlights);
    }
}

/// Visit visible dashes while preserving the pattern across all polyline segments
fn visible_dashes(clip: Rect, line: &[Pos2], dash_points: f32, gap_points: f32, mut draw: impl FnMut([Pos2; 2])) {
    let dash_length = f64::from(dash_points);
    let period = dash_length + f64::from(gap_points);
    let mut phase = 0.0;
    for &[from, to] in line.array_windows() {
        let dx = f64::from(to.x) - f64::from(from.x);
        let dy = f64::from(to.y) - f64::from(from.y);
        let length = dx.hypot(dy);
        if !length.is_finite() || length == 0.0 {
            continue;
        }

        if let Some((enter, exit)) = segment_in_rect(clip, from, dx, dy) {
            let point_at =
                |fraction: f64| pos2((f64::from(from.x) + dx * fraction) as f32, (f64::from(from.y) + dy * fraction) as f32).clamp(clip.min, clip.max);
            let first = point_at(enter);
            let last = point_at(exit);
            let visible_length = f64::from(first.distance(last));
            if visible_length > 0.0 {
                let mut start = -(phase + (enter * length) % period) % period;
                while start < visible_length {
                    let end = (start + dash_length).min(visible_length);
                    if end > 0.0 {
                        let point = |distance: f64| first.lerp(last, (distance / visible_length) as f32);
                        draw([point(start.max(0.0)), point(end)]);
                    }
                    start += period;
                }
            }
        }

        phase = (phase + length % period) % period;
    }
}

/// The interval of the segment that intersects the two axis ranges of `clip`
fn segment_in_rect(clip: Rect, from: Pos2, dx: f64, dy: f64) -> Option<(f64, f64)> {
    let (mut enter, mut exit) = (0.0_f64, 1.0_f64);
    for (delta, origin, low, high) in [(dx, from.x, clip.left(), clip.right()), (dy, from.y, clip.top(), clip.bottom())] {
        if delta == 0.0 {
            if origin < low || origin > high {
                return None;
            }

            continue;
        }

        let origin = f64::from(origin);
        let (at_low, at_high) = ((f64::from(low) - origin) / delta, (f64::from(high) - origin) / delta);
        enter = enter.max(at_low.min(at_high));
        exit = exit.min(at_low.max(at_high));
    }

    (enter <= exit).then_some((enter, exit))
}

/// Polygon fill in a view: a click fills one target, a drag fills its box live (one undo step); returns the box to draw over the view image
fn polygon_fill_input(
    app: &mut TextureCraftApp,
    ui: &Ui,
    response: &egui::Response,
    fill_at: impl Fn(&mut Session, Pos2) -> texturecraft_engine::Result<usize>,
    drag_box: impl Fn(&mut Session, Pos2, Pos2) -> texturecraft_engine::Result<usize>,
) -> Option<[Pos2; 2]> {
    let origin_id = response.id.with("polygon_fill_box");
    let dragging = response.dragged_by(PointerButton::Primary);
    let released = response.drag_stopped_by(PointerButton::Primary);
    let held = ui.input(|input| input.pointer.primary_down());
    if !held && !released {
        // A drag this view started and cut short (Alt, a second finger) keeps what it filled as one step
        if ui.data_mut(|data| data.remove_temp::<Pos2>(origin_id)).is_some() {
            app.session.end_polygon_fill_box();
        }
    } else if dragging && let Some(press) = ui.input(|input| input.pointer.press_origin()) {
        ui.data_mut(|data| data.insert_temp(origin_id, press));
    }

    // Consume the origin on release so a new press cannot resume the previous rectangle
    let origin = if released { ui.data_mut(|data| data.remove_temp::<Pos2>(origin_id)) } else { ui.data(|data| data.get_temp::<Pos2>(origin_id)) };
    let pointer = ui.input(|input| input.pointer.latest_pos());
    let (filled, dragged_box) = match (origin, pointer) {
        _ if response.clicked_by(PointerButton::Primary) => (response.interact_pointer_pos().map(|point| fill_at(&mut app.session, point)), None),
        (Some(origin), Some(pointer)) if released => {
            // The drag ends on release even when its last move failed
            let shown = drag_box(&mut app.session, origin, pointer);
            let filled = app.session.end_polygon_fill_box();
            (Some(shown.map(|_| filled)), None)
        }
        _ if released => {
            app.session.end_polygon_fill_box();
            (None, None)
        }
        (Some(origin), Some(pointer)) => (Some(drag_box(&mut app.session, origin, pointer)), Some([origin, pointer])),
        _ => (None, None),
    };

    if let Some(Err(error)) = filled {
        app.toast(error.to_string(), true);
    }

    dragged_box
}

/// How strongly the polygon fill box tints what it covers (0..1)
const FILL_BOX_TINT: f32 = 0.15;

/// The polygon fill box being dragged, over view `rect`
fn polygon_fill_box_overlay(ui: &Ui, rect: Rect, fill_box: Option<[Pos2; 2]>) {
    let Some([origin, opposite]) = fill_box else { return };

    let accent = Tokens::get(ui.ctx()).viewport_accent;
    let fill_box = Rect::from_two_pos(origin, opposite);
    ui.painter_at(rect).rect(fill_box, 0.0, accent.gamma_multiply(FILL_BOX_TINT), Stroke::new(1.0, accent), egui::StrokeKind::Inside);
}

/// Outline of the texture-aligned fill bounds projected into the rotated view.
fn uv_fill_box_corners(view: crate::uv_view_map::UvViewMap, [origin, opposite]: [Pos2; 2]) -> [Pos2; 4] {
    let origin_uv = view.to_uv(origin);
    let opposite_uv = view.to_uv(opposite);
    let low = origin_uv.min(opposite_uv);
    let high = origin_uv.max(opposite_uv);

    [view.to_screen(low.x, low.y), view.to_screen(high.x, low.y), view.to_screen(high.x, high.y), view.to_screen(low.x, high.y)]
}

/// V held with the Clone tool in painting mode: clicks set the clone source
fn clone_source_key(app: &TextureCraftApp, ui: &Ui) -> bool {
    tool_acts(app)
        && app.session.tool.tool == Tool::Clone
        && ui.input(|input| input.key_down(CLONE_SOURCE_KEY))
        && !crate::framework::widget_has_keyboard(ui.ctx())
}

/// Crosshair where the Clone tool reads from.
fn clone_marker(app: &TextureCraftApp, ui: &Ui, rect: Rect, pos: Vec3) {
    if let Some(center) = project(app, rect, pos) {
        clone_marker_at(ui, rect, center);
    }
}

/// The clone source crosshair centred on the screen point `center`
fn clone_marker_at(ui: &Ui, rect: Rect, center: Pos2) {
    let tokens = Tokens::get(ui.ctx());
    let painter = ui.painter_at(rect);
    for (stroke_width, color) in [(3.0, Color32::from_black_alpha(140)), (1.2, tokens.viewport_accent)] {
        painter.circle_stroke(center, 7.0, Stroke::new(stroke_width, color));
        painter.line_segment([center - vec2(11.0, 0.0), center + vec2(11.0, 0.0)], Stroke::new(stroke_width, color));
        painter.line_segment([center - vec2(0.0, 11.0), center + vec2(0.0, 11.0)], Stroke::new(stroke_width, color));
    }
}

/// Whether the active tool can act on the selection now: a greyed tool ignores clicks, keys and drags in the views
fn tool_acts(app: &TextureCraftApp) -> bool {
    app.ui.mode == crate::state::Mode::Painting && app.session.tool_usable(app.session.tool.tool)
}

/// Whether the 2D view says that the equipped Projection tool paints in the 3D view only (the quick mask is painted like with a brush)
fn projection_hint_shown(app: &TextureCraftApp) -> bool {
    tool_acts(app) && app.session.tool.tool == Tool::Projection && !app.session.quick_mask.editing
}

/// Both views must close gestures even when the selection disables their input handlers.
fn finish_unavailable_tool_gestures(app: &mut TextureCraftApp, pending_2d_endpoint: Option<crate::uv_paint_point::UvPaintPoint>) {
    if !tool_acts(app) {
        if app.session.stroke_active() {
            match app.session.end_stroke() {
                Ok(()) => {
                    if let Some(endpoint) = pending_2d_endpoint {
                        app.last_paint_uv = Some(endpoint);
                    }
                }
                Err(error) => app.toast(error.to_string(), true),
            }
        }
        app.session.end_polygon_fill_box();
    }
}

/// Tools that paint with the brush (and show its cursor).
fn brush_tool(t: Tool) -> bool {
    matches!(t, Tool::Paint | Tool::Eraser | Tool::Projection | Tool::Smudge | Tool::Clone | Tool::Path)
}

/// Wheel travel this frame in points. egui turns a Shift+wheel into a horizontal scroll, so with Shift held
/// the travel arrives on x (on a mac the system already does this); the sum covers both.
fn wheel_scroll(ui: &Ui, shift: bool) -> f32 {
    ui.input(|input| {
        let delta = input.smooth_scroll_delta;
        if shift { delta.x + delta.y } else { delta.y }
    })
}

/// Only command-wheel input resizes the brush. A pinch zooms the viewport.
fn size_brush_by_ctrl_wheel(app: &mut TextureCraftApp, ui: &Ui) {
    if !wheel_sizes_brush(app) {
        return;
    }

    let line = ui.ctx().options(|options| options.input_options.line_scroll_speed);
    let points: f32 = ui.input(|input| {
        input
            .raw
            .events
            .iter()
            .filter_map(|event| {
                let egui::Event::MouseWheel { unit, delta, modifiers, .. } = event else { return None };
                let scale = match unit {
                    egui::MouseWheelUnit::Point => 1.0,
                    egui::MouseWheelUnit::Line => line,
                    egui::MouseWheelUnit::Page => input.viewport_rect().height(),
                };
                modifiers.command.then_some(delta.y * scale)
            })
            .sum()
    });
    if points.is_finite() && points != 0.0 {
        scale_brush_by_wheel(app, -points);
    }
}

fn trackpad_pinch(ui: &Ui) -> f32 {
    ui.input(|input| {
        input
            .raw
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Zoom(factor) if factor.is_finite() && *factor > 0.0 => Some(*factor),
                _ => None,
            })
            .product()
    })
}

/// Shift+wheel sizes the brush while a brush tool can act; otherwise the wheel zooms as usual.
fn wheel_sizes_brush(app: &TextureCraftApp) -> bool {
    tool_acts(app) && brush_tool(app.session.tool.tool)
}

/// Shift+wheel: scale the brush size.
fn scale_brush_by_wheel(app: &mut TextureCraftApp, scroll: f32) {
    let brush = &mut app.session.tool.brush;
    brush.size = (brush.size * (1.0 + scroll * WHEEL_BRUSH_SIZE_PER_POINT)).clamp(MIN_BRUSH_SIZE, MAX_BRUSH_SIZE);
}

/// Whether the press under way, or released this frame, began on a projection handle (`on_handle`); with no press, whether the pointer is on one
fn handle_press(ctx: &egui::Context, on_handle: impl Fn(Pos2) -> bool) -> bool {
    let id = egui::Id::unique(HANDLE_PRESS_ID);
    let (pressed_at, down, released, hover) = ctx.input(|input| {
        let pressed_at = input.events.iter().rev().find_map(|event| match event {
            egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } => Some(*pos),
            _ => None,
        });
        (pressed_at, input.pointer.primary_down(), input.pointer.button_released(PointerButton::Primary), input.pointer.hover_pos())
    });

    // Keep the press origin because egui clears it before the release frame
    if let Some(at) = pressed_at {
        let on = on_handle(at);
        ctx.data_mut(|data| data.insert_temp(id, on));
    }
    let kept = ctx.data(|data| data.get_temp::<bool>(id)).filter(|_| down || released);
    if !down {
        ctx.data_mut(|data| data.remove::<bool>(id));
    }

    kept.unwrap_or_else(|| !down && !released && hover.is_some_and(on_handle))
}

/// Where [`handle_press`] keeps whether the press began on a handle
const HANDLE_PRESS_ID: &str = "projection_handle_press";

/// Ctrl (Cmd on macOS) + drag changes the brush instead of painting: true while it applies (see `adjust_brush`)
fn brush_drag(app: &mut TextureCraftApp, ui: &Ui, response: &egui::Response) -> bool {
    // Ctrl+Shift+left is not a brush change: it is left to the snapped straight line
    let held = tool_acts(app)
        && brush_tool(app.session.tool.tool)
        && !app.session.stroke_active()
        && ui.input(|input| input.modifiers.command && !input.modifiers.alt && (!input.modifiers.shift || input.pointer.secondary_down()));
    if held && let Some(right) = brush_drag_button(response) {
        let (delta, fine) = ui.input(|input| (input.pointer.delta(), input.modifiers.shift));
        adjust_brush(&mut app.session.tool.brush, right, if fine { delta * CTRL_DRAG_FINE_FACTOR } else { delta });
    }

    held
}

/// Where a straight line to `pointer` ends: turned to a multiple of 15 degrees about `from` when it is set (Ctrl+Shift), else at `pointer`
fn snapped_end(from: Option<Pos2>, pointer: Pos2) -> Pos2 {
    from.map_or(pointer, |from| crate::line_snap::snap_line_end(from, pointer, crate::line_snap::LINE_SNAP_STEP_DEGREES))
}

/// The button of a drag on `resp`: Some(true) right, Some(false) left.
fn brush_drag_button(resp: &egui::Response) -> Option<bool> {
    let right = resp.dragged_by(PointerButton::Secondary);
    (right || resp.dragged_by(PointerButton::Primary)).then_some(right)
}

/// While a Ctrl+drag changes the brush, its values beside where the drag began (drawn over the
/// view; the tool bar and Properties show them too).
fn brush_readout(app: &TextureCraftApp, ui: &Ui, response: &egui::Response, adjusting: bool) {
    let Some(right_drag) = brush_drag_button(response).filter(|_| adjusting) else { return };
    let Some(press_origin) = ui.input(|input| input.pointer.press_origin()) else { return };

    // Keep the values clear of the pointer and the brush cursor at its press origin
    if ui.input(|input| input.pointer.latest_pos()).is_some_and(|pointer| response.rect.contains(pointer)) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    }

    let brush = &app.session.tool.brush;
    let text = if right_drag {
        format!("{} {:.1}   {} {:.0}", tr("Size"), brush.size / 0.25 * 100.0, tr("Hardness"), brush.hardness * 100.0)
    } else {
        format!("{} {:.0}   {} {:.0}°", tr("Flow"), brush.flow * 100.0, tr("Angle"), brush.angle)
    };

    let tokens = Tokens::get(ui.ctx());
    let painter = ui.painter_at(response.rect);
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.0), tokens.text_strong);
    let readout_rect = Rect::from_min_size(press_origin + vec2(18.0, -30.0), galley.size()).expand2(vec2(6.0, 3.0));
    painter.rect_filled(readout_rect, 3.0, tokens.panel);
    painter.galley(readout_rect.min + vec2(6.0, 3.0), galley, tokens.text_strong);
}

/// Part of the change a Ctrl+right drag makes while Shift is also held (fine mode)
const CTRL_DRAG_FINE_FACTOR: f32 = 0.1;

/// Size fraction, hardness and flow change per point of a Ctrl+drag
const CTRL_DRAG_RATE_PER_POINT: f32 = 0.005;

/// Brush angle change per point of a Ctrl+left drag (degrees)
const CTRL_DRAG_DEGREES_PER_POINT: f32 = 0.5;

/// Highest hardness a Ctrl+right drag sets
const CTRL_DRAG_MAX_HARDNESS: f32 = 0.999;

/// Brush change for a Ctrl+drag by `drag` points: a right drag sets the size (x) and hardness (y), a left drag the flow (x) and angle (y); up raises hardness and angle
fn adjust_brush(brush: &mut texturecraft_engine::paint::BrushSettings, right: bool, drag: egui::Vec2) {
    if right {
        brush.size = (brush.size * (1.0 + drag.x * CTRL_DRAG_RATE_PER_POINT)).clamp(MIN_BRUSH_SIZE, MAX_BRUSH_SIZE);
        brush.hardness = (brush.hardness - drag.y * CTRL_DRAG_RATE_PER_POINT).clamp(0.0, CTRL_DRAG_MAX_HARDNESS);
    } else {
        brush.flow = (brush.flow + drag.x * CTRL_DRAG_RATE_PER_POINT).clamp(0.0, 1.0);
        brush.angle = (brush.angle - drag.y * CTRL_DRAG_DEGREES_PER_POINT + 180.0).rem_euclid(360.0) - 180.0;
    }
}

/// Segments of the brush outline and hardness ring
const CURSOR_OUTLINE_SEGMENTS: usize = 48;

/// The roundness the cursor shows: only the procedural Round alpha has one
fn cursor_roundness(brush: &texturecraft_engine::paint::BrushSettings) -> f32 {
    let round = brush.alpha == texturecraft_engine::paint::BrushAlpha::Round && brush.alpha_image.is_none() && brush.alpha_library.is_none();
    if round { brush.alpha_params.roundness } else { 1.0 }
}

/// Whether the outline is a plain circle (equal sizes, full roundness), drawn as one
fn cursor_is_circle(brush: &texturecraft_engine::paint::BrushSettings) -> bool {
    brush.alpha_params.size_x == brush.alpha_params.size_y && cursor_roundness(brush) >= 1.0
}

/// The stamp's outline scaled by `scale` (the hardness ring's share of the radius), as points around the
/// stamp center in a y-up frame in units of the stamp radius: the alpha's extent (Size X and Y), the
/// squarer corners of a Round alpha with low roundness, then turned by the brush angle.
fn cursor_outline(brush: &texturecraft_engine::paint::BrushSettings, scale: f32) -> Vec<Vec2> {
    let params = brush.alpha_params;
    let roundness = cursor_roundness(brush);
    let (sin, cos) = brush.angle.to_radians().sin_cos();
    (0..=CURSOR_OUTLINE_SEGMENTS)
        .map(|segment| {
            let angle = segment as f32 / CURSOR_OUTLINE_SEGMENTS as f32 * std::f32::consts::TAU;
            let direction = Vec2::new(angle.cos(), angle.sin());

            // Distance of the unit direction in the Round alpha's metric: between the square's and the circle's
            let square = direction.x.abs().max(direction.y.abs());
            let reach = 1.0 / (square + (1.0 - square) * roundness);
            let point = direction * reach * scale * Vec2::new(params.size_x, params.size_y);
            Vec2::new(point.x * cos - point.y * sin, point.x * sin + point.y * cos)
        })
        .collect()
}

pub(crate) fn brush_cursor(app: &TextureCraftApp, ui: &Ui, rect: Rect, position: Vec3, normal: Vec3, radius: f32) {
    let brush = &app.session.tool.brush;

    // The stamp's frame as the stroke builds it: the camera's up direction seen along the surface
    let forward = app.session.camera.forward();
    let camera_up = forward.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X).cross(forward);
    let up = (camera_up - normal * normal.dot(camera_up)).try_normalize().unwrap_or_else(|| normal.any_orthonormal_vector());
    let tangent = up.cross(normal).try_normalize().unwrap_or_else(|| normal.any_orthonormal_vector());
    let bitangent = normal.cross(tangent);
    let to_screen = |point: Vec3| project(app, rect, point);
    let ring = |scale: f32| -> Vec<Pos2> {
        cursor_outline(brush, scale).into_iter().filter_map(|local| to_screen(position + (tangent * local.x + bitangent * local.y) * radius)).collect()
    };
    let outline = ring(1.0);

    let painter = ui.painter_at(rect);
    painter.add(egui::Shape::line(outline.clone(), Stroke::new(2.5, Color32::from_black_alpha(140))));
    painter.add(egui::Shape::line(outline, Stroke::new(1.2, Color32::from_white_alpha(230))));

    let hardness = brush.hardness;
    if hardness > 0.05 {
        painter.add(egui::Shape::dashed_line(&ring(hardness), Stroke::new(1.0, Color32::from_white_alpha(120)), 3.0, 3.0));
    }

    if let Some(center) = to_screen(position) {
        painter.circle_filled(center, 1.5, Color32::WHITE);
    }
}

/// This frame's pen pressure (0..1), or 1 without a pen. Pressure arrives as touch force: the
/// windowing layer reports it for Windows Ink pens, and in the browser for pens that send touch
/// events (tablets and phones); other pens come through as a mouse.
/// shortcut: the windowing layer reports no pen tilt, so tilt can't drive the brush angle yet.
pub fn pen_pressure(ui: &Ui) -> f32 {
    let pressure_id = egui::Id::unique("pen_pressure");
    let touch = ui.input(|input| {
        let events = &input.raw.events;
        events.iter().enumerate().rev().find_map(|(index, event)| match event {
            egui::Event::Touch { device_id, id: touch_id, phase, force, .. } => {
                let mut contact_force = force.filter(|pressure| pressure.is_finite());
                let mut effective_phase = *phase;
                if contact_force.is_none() && matches!(*phase, egui::TouchPhase::Move | egui::TouchPhase::End) {
                    // Recover pressure only from this contact, including an unforced press-frame move
                    for previous_event in events[..index].iter().rev() {
                        if let egui::Event::Touch { device_id: previous_device_id, id: previous_touch_id, phase: previous_phase, force: previous_force, .. } =
                            previous_event
                            && previous_device_id == device_id
                            && previous_touch_id == touch_id
                        {
                            // A completed contact cannot supply pressure to a reused touch ID
                            if matches!(previous_phase, egui::TouchPhase::End | egui::TouchPhase::Cancel) {
                                effective_phase = egui::TouchPhase::Cancel;
                                break;
                            }

                            contact_force = previous_force.filter(|pressure| pressure.is_finite());
                            if contact_force.is_none() && *previous_phase == egui::TouchPhase::Start {
                                effective_phase = egui::TouchPhase::Start;
                                break;
                            }

                            if contact_force.is_some() {
                                break;
                            }
                        }
                    }
                }

                Some((effective_phase, contact_force))
            }
            _ => None,
        })
    });

    match touch {
        Some((_, Some(pressure))) => {
            let pressure = pressure.clamp(0.0, 1.0);
            ui.data_mut(|data| *data.get_temp_mut_or(pressure_id, pressure) = pressure);
            pressure
        }
        // A pen lifts with no pressure, so keep its contact pressure for a tap
        Some((egui::TouchPhase::End, None)) => ui.data(|data| data.get_temp(pressure_id)).unwrap_or(1.0),
        // A touch without pressure must not inherit an earlier pen's pressure
        Some((egui::TouchPhase::Start | egui::TouchPhase::Cancel, None)) => {
            ui.data_mut(|data| data.remove_temp::<f32>(pressure_id));
            1.0
        }
        _ => {
            let (primary_pressed, primary_active) =
                ui.input(|input| (input.pointer.primary_pressed(), input.pointer.primary_down() || input.pointer.primary_released()));

            // A fresh mouse press must not inherit the previous pen's pressure
            if primary_active && !primary_pressed {
                ui.data(|data| data.get_temp::<f32>(pressure_id)).unwrap_or(1.0)
            } else {
                ui.data_mut(|data| data.remove_temp::<f32>(pressure_id));
                1.0
            }
        }
    }
}

/// F: puts the 2D view back to pan 0, zoom 1 and no rotation; in a UV tile set it frames tile 1001 (else the lowest tile)
fn frame_uv_view_on_request(app: &mut TextureCraftApp, size: egui::Vec2) {
    if !std::mem::take(&mut app.uv_view_reset) {
        return;
    }

    let (zoom, pan) = app.session.doc.project.texture_sets.get(app.session.ts()).map_or((1.0, [0.0, 0.0]), |set| framed_tile_view(set, size));
    app.ui.uv_zoom = zoom;
    app.ui.uv_pan = pan;
    app.ui.uv_rotation = 0.0;
}

/// The 2D zoom and pan that fit tile 1001 of a UV tile `set` (else its lowest tile) into a view of `size`; zoom 1 and no pan for a set without tiles
fn framed_tile_view(set: &texturecraft_engine::core::TextureSet, size: egui::Vec2) -> (f32, [f32; 2]) {
    let Some(grid) = set.tile_grid() else { return (1.0, [0.0, 0.0]) };
    let tile = if set.uv_tiles.contains(&1001) { 1001 } else { set.uv_tiles.iter().copied().min().unwrap_or(1001) };
    let Some((block_x, block_y)) = grid.block(tile) else { return (1.0, [0.0, 0.0]) };

    // The tile's square inside its atlas block, and the zoom that lets it fill what the whole atlas fills at zoom 1
    let tile_scale = set.tile_scale(tile);
    let zoom = (grid.cols.max(grid.rows) as f32 / tile_scale).clamp(UV_VIEW_MIN_ZOOM, UV_VIEW_MAX_ZOOM);
    let center = ((block_x as f32 + tile_scale * 0.5) / grid.cols as f32, (block_y as f32 + tile_scale * 0.5) / grid.rows as f32);
    let [scale_x, scale_y] = texturecraft_render::uv_view_scale(size.x / size.y.max(1.0), zoom, set.size());
    (zoom, [-(center.0 * 2.0 - 1.0) * scale_x, -(1.0 - center.1 * 2.0) * scale_y])
}

/// The 2D (UV) view: navigation, painting in texture coordinates and the overlays drawn over the texture.
fn view_2d(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect, sync: &mut PendingSync) {
    let tokens = Tokens::get(ui.ctx());
    let response = view_widget(ui, rect, view_2d_id(), Sense::click_and_drag());
    let size = rect.size();
    let (alt, shift, snap) = ui.input(|input| (input.modifiers.alt, input.modifiers.shift, input.modifiers.shift && input.modifiers.command));
    let ctrl = ui.input(|input| input.modifiers.ctrl);
    let adjusting = brush_drag(app, ui, &response);
    frame_uv_view_on_request(app, size);

    // Middle drag and Ctrl+Alt+left pan without changing the brush
    let delta = ui.input(|input| input.pointer.delta());
    let ctrl_alt_pan = alt && ctrl && response.dragged_by(PointerButton::Primary);
    if response.dragged_by(PointerButton::Middle) || ctrl_alt_pan {
        app.ui.uv_pan[0] += delta.x / size.x * 2.0;
        app.ui.uv_pan[1] -= delta.y / size.y * 2.0;
    }
    let turning = alt && !ctrl && response.dragged_by(PointerButton::Primary);
    if turning {
        let radians = -delta.x * UV_RADIANS_PER_POINT * app.ui.prefs.orbit_scale();
        if shift {
            // The drag's own turn follows the pointer; the view takes the nearest step
            let reached = app.uv_rotation_snap.get_or_insert(app.ui.uv_rotation);
            *reached += radians;
            let snapped = (*reached / UV_SNAP_RADIANS).round() * UV_SNAP_RADIANS;
            let step = snapped - app.ui.uv_rotation;
            let wrapped = (step + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            crate::uv_view_map::turn_about_center(&mut app.ui.uv_rotation, &mut app.ui.uv_pan, size, wrapped);
        } else {
            crate::uv_view_map::turn_about_center(&mut app.ui.uv_rotation, &mut app.ui.uv_pan, size, radians);
        }
    }

    // The next snapping drag starts again from where the view is
    if !(turning && shift) {
        app.uv_rotation_snap = None;
    }
    if alt && response.dragged_by(PointerButton::Secondary) {
        let change = (delta.x * UV_DRAG_ZOOM_PER_POINT_RIGHT + delta.y * UV_DRAG_ZOOM_PER_POINT_DOWN) * app.ui.prefs.drag_zoom_scale();
        let old_zoom = app.ui.uv_zoom;
        let new_zoom = (old_zoom * change.exp()).clamp(UV_VIEW_MIN_ZOOM, UV_VIEW_MAX_ZOOM);

        // About the view's centre
        for axis in 0..2 {
            app.ui.uv_pan[axis] *= new_zoom / old_zoom;
        }
        app.ui.uv_zoom = new_zoom;
    }

    // A held Move on model button takes the wheel
    if response.hovered() && app.moving_on_model.is_none() {
        size_brush_by_ctrl_wheel(app, ui);
        let scroll = wheel_scroll(ui, shift);
        if scroll != 0.0 && shift && wheel_sizes_brush(app) {
            scale_brush_by_wheel(app, scroll);
        } else {
            let pinch = ui.input(|input| input.multi_touch().map(|touch| touch.zoom_delta)).unwrap_or_else(|| trackpad_pinch(ui));
            let old_zoom = app.ui.uv_zoom;
            let new_zoom = (old_zoom * pinch * (1.0 + scroll * UV_WHEEL_ZOOM_PER_POINT)).clamp(UV_VIEW_MIN_ZOOM, UV_VIEW_MAX_ZOOM);
            if old_zoom != new_zoom
                && let Some(hover_pos) = response.hover_pos()
            {
                let cursor = [(hover_pos.x - rect.left()) / size.x * 2.0 - 1.0, 1.0 - (hover_pos.y - rect.top()) / size.y * 2.0];
                for axis in 0..2 {
                    app.ui.uv_pan[axis] = cursor[axis] - (cursor[axis] - app.ui.uv_pan[axis]) * new_zoom / old_zoom;
                }
            }
            app.ui.uv_zoom = new_zoom;
        }
    }

    // Conversions between the view's points and the texture set's UV space
    let texture_set_index = app.session.ts();
    let set = app.session.doc.project.texture_sets.get(texture_set_index);
    let [scale_x, scale_y] = texturecraft_render::uv_view_scale(size.x / size.y.max(1.0), app.ui.uv_zoom, set.map_or((1, 1), |texture_set| texture_set.size()));
    let view_map = crate::uv_view_map::UvViewMap { rect, scale: [scale_x, scale_y], pan: app.ui.uv_pan, rotation: app.ui.uv_rotation };
    let to_uv = |pointer: egui::Pos2| view_map.to_uv(pointer);
    let to_screen = |u: f32, v: f32| view_map.to_screen(u, v);
    let show = |_: &TextureCraftApp, point: Vec3| Some(to_screen(point.x, point.y));
    let path_tool = app.session.tool.tool == Tool::Path;

    // Clone source: V + click
    let v_held = clone_source_key(app, ui);
    if v_held
        && (response.clicked_by(PointerButton::Primary) || response.drag_started_by(PointerButton::Primary))
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let uv = to_uv(pointer);
        app.run("tool.cloneSource", json!({"uv": [uv.x, uv.y]}));
    }

    let bake_paint_on = app.ui.mode == crate::state::Mode::Baking && app.ui.bake_paint_brush.on && !alt;
    if bake_paint_on {
        bake_paint_input(app, ui, &response, &to_uv);
    }

    crate::properties_popup::open_on_right_click(app, ui, &response, rect, v_held);

    // A pending ID color pick takes the next click instead of the brush
    let picking_id_color = app.picking_id_color.is_some();
    if picking_id_color {
        crate::id_color_pick::IdColorPick::draw_cursor(ui, rect);
        if !alt
            && response.clicked_by(PointerButton::Primary)
            && let Some(position) = response.interact_pointer_pos()
        {
            let uv = to_uv(position);
            crate::id_color_pick::IdColorPick::pick_at(app, json!({"uv": [uv.x, uv.y]}));
        }
    }

    // Painting in UV space; the Projection tool only paints in the 3D view, with or without a stencil
    let projection_hint = projection_hint_shown(app);
    let painting = tool_acts(app)
        && !projection_hint
        && app.properties_popup.is_none()
        && !crate::rail_popup::blocks_paint_input(app, ui.ctx())
        && !crate::color_picker::blocks_view_press(ui.ctx())
        && !alt
        && !adjusting
        && !v_held
        && !picking_id_color
        && !app.picking_symmetry_plane
        && app.picking_color.is_none()
        && !DecalDrop::blocks_input(ui.ctx());
    let mut fill_box = None;
    if painting && path_tool {
        path_input(app, ui, &response, (rect, true), |_, pointer| Some(to_uv(pointer).extend(0.0)), show);
    } else if painting && app.session.tool.tool == Tool::PolygonFill {
        fill_box = polygon_fill_input(
            app,
            ui,
            &response,
            |session, point| session.polygon_fill_uv(to_uv(point)),
            |session, corner, opposite| session.drag_polygon_fill_box_uv(to_uv(corner), to_uv(opposite)),
        );
    } else if painting {
        paint_2d_input(app, ui, &response, (shift, snap), &to_uv, &to_screen);
    }
    if response.double_clicked() && !(painting && path_tool) {
        app.ui.uv_zoom = 1.0;
        app.ui.uv_pan = [0.0, 0.0];
        app.ui.uv_rotation = 0.0;
    }

    let frame = Frame {
        view_id: VIEW_2D,
        size: frame_size_in_pixels(ui, rect),
        kind: ViewKind::Uv { ts: texture_set_index, zoom: app.ui.uv_zoom, pan: app.ui.uv_pan, rotation: app.ui.uv_rotation },
        camera: app.session.camera,
        shade: shade(app),
        visible: app.session.doc.project.texture_sets.iter().map(|texture_set| texture_set.visible).collect(),
        overlay: Vec::new(),
    };
    add_callback(app, ui, rect, frame, sync);
    if let Some(bounds) = fill_box {
        let accent = tokens.viewport_accent;
        let corners = uv_fill_box_corners(view_map, bounds);
        ui.painter_at(rect).add(egui::Shape::convex_polygon(corners.to_vec(), accent.gamma_multiply(FILL_BOX_TINT), Stroke::new(1.0, accent)));
    }

    crate::shelf::drop_target(app, ui, &response, crate::shelf::DropZone::View2d);
    if projection_hint {
        ui.painter_at(rect).text(
            rect.left_top() + QUICK_MASK_NOTICE_OFFSET,
            egui::Align2::LEFT_TOP,
            tr("Projection paints in the 3D view"),
            egui::FontId::proportional(QUICK_MASK_NOTICE_TEXT_SIZE),
            QUICK_MASK_NOTICE_COLOR,
        );
    }

    // The UV tile outlines (numbered for UDIM sets) and the path being edited
    let painter = ui.painter_at(rect);
    let set = app.session.doc.project.texture_sets.get(texture_set_index);
    let outline = |corners: [Pos2; 4]| {
        painter.add(egui::Shape::closed_line(corners.to_vec(), Stroke::new(UV_TILE_OUTLINE_WIDTH, Color32::from_white_alpha(UV_TILE_OUTLINE_ALPHA))));
    };
    let quad = |u0: f32, v0: f32, u1: f32, v1: f32| [to_screen(u0, v0), to_screen(u1, v0), to_screen(u1, v1), to_screen(u0, v1)];
    let tile_grid = set.and_then(|texture_set| Some((texture_set, texture_set.tile_grid()?)));
    match tile_grid {
        Some((texture_set, grid)) => {
            // Show UV axes and tile numbers across the view and outline tiles painted by the set
            crate::uv_tile_grid::draw(&painter, view_map, texture_set, grid, app.ui.uv_tile_info, tokens.viewport_text);
            for (udim, (block_x, block_y)) in texture_set.uv_tiles.iter().filter_map(|udim| Some((*udim, grid.block(*udim)?))) {
                // A tile smaller than its block fills the block's top-left corner
                let (u0, v0) = (block_x as f32 / grid.cols as f32, block_y as f32 / grid.rows as f32);
                let tile_scale = texture_set.tile_scale(udim);
                outline(quad(u0, v0, u0 + tile_scale / grid.cols as f32, v0 + tile_scale / grid.rows as f32));
            }
        }
        None => {
            outline(quad(0.0, 0.0, 1.0, 1.0));
        }
    }
    if tool_acts(app) && path_tool {
        path_overlay(app, ui, rect, &response, true, show);
    }

    // Clone source: where the tool reads, as in the 3D view
    if tool_acts(app)
        && app.session.tool.tool == Tool::Clone
        && let Some(source) = app.session.clone_source_marker_uv(response.hover_pos().map(to_uv))
    {
        clone_marker_at(ui, rect, to_screen(source.x, source.y));
    }

    // Shift: preview the straight line Shift+click paints from the last stroke (snapped with Ctrl)
    if painting
        && shift
        && !path_tool
        && brush_tool(app.session.tool.tool)
        && let Some(from) = app.last_paint_uv.and_then(|point| point.uv_for(&app.session))
        && let Some(hover) = response.hover_pos()
    {
        let from = to_screen(from.x, from.y);
        rubber_band(ui, rect, &[from, snapped_end(snap.then_some(from), hover)]);
    }

    if app.ui.mode == crate::state::Mode::Baking {
        bake_paint_overlay(app, ui, &painter, &to_screen);
        if bake_paint_on && let Some(hover) = response.hover_pos() {
            let radius = (to_screen(app.ui.bake_paint_brush.radius, 0.0) - to_screen(0.0, 0.0)).length().max(BAKE_PAINT_CURSOR_MIN_RADIUS);
            painter.circle_stroke(hover, radius, Stroke::new(BAKE_PAINT_CURSOR_STROKE, Color32::from_white_alpha(BAKE_PAINT_CURSOR_ALPHA)));
        }
    }

    // Match the 3D view's stamp size and hardness
    let show_brush_cursor = tool_acts(app) && !alt && !v_held && app.picking_color.is_none();
    let pointer_in_view = ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| rect.contains(p));
    if show_brush_cursor
        && !path_tool
        && brush_tool(app.session.tool.tool)
        && !response.dragged_by(PointerButton::Middle)

        // Egui keeps hover during drags; adjustment may continue outside the view
        && (adjusting || pointer_in_view)

        // Keep the brush outline at the press origin while adjusting
        && let Some(cursor_center) = if adjusting { ui.input(|input| input.pointer.press_origin()) } else { response.hover_pos().or_else(|| response.interact_pointer_pos()) }

        // Header pickers need the normal pointer
        && !view_header_rect(rect).is_some_and(|header| header.expand(2.0).contains(cursor_center))
        && !crate::nav_buttons::covers(app, rect, crate::nav_buttons::NavView::Uv, cursor_center)
    {
        // Match the pressure-scaled radius used by continue_stroke_uv
        let radius_points =
            (to_screen(app.session.tool.brush.uv_stamp_radius(pen_pressure(ui)), 0.0) - to_screen(0.0, 0.0)).length().max(BRUSH_CURSOR_MIN_RADIUS_POINTS);
        if pointer_in_view {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        }
        brush_cursor_2d(&painter, cursor_center, radius_points, &app.session.tool.brush);
    }
    brush_readout(app, ui, &response, adjusting);
    solo_notice(app, ui, rect);
    view_header(app, ui, rect, "v2d");
    mask_view_badge(app, ui, rect);
    axis_gizmo(ui, rect, app, Some(app.ui.uv_rotation));
    crate::nav_buttons::show(app, ui, rect, crate::nav_buttons::NavView::Uv);
    if let Some(label) = hover_label(&app.session, app.ui.uv_tile_info, view_map, response.hover_pos()) {
        let label_position = rect.left_bottom() + vec2(SET_NAME_INSET.x, -SET_NAME_INSET.y);
        ui.painter().text(label_position, egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(SET_NAME_TEXT_SIZE), tokens.viewport_text);
    }
}

/// The label in the 2D view's bottom-left corner: the texture set and, in a UV tile set with tile info on, the tile under the cursor ("<set> - <tile>").
/// A set that is not tiled shows its name alone.
fn hover_label(session: &Session, tile_info: bool, view_map: crate::uv_view_map::UvViewMap, hover: Option<Pos2>) -> Option<String> {
    let set = session.doc.project.texture_sets.get(session.ts())?;
    let tile = set.tile_grid().and_then(|grid| crate::uv_tile_grid::tile_under(view_map, set, grid, hover?));
    hover_label_text(&set.name, set.tile_grid().is_some(), tile_info, tile)
}

/// [`hover_label`]'s text for a set called `name`: the name alone for a set that is not tiled, nothing in a tiled set with tile info off, else the name and the `tile` under the cursor
fn hover_label_text(name: &str, tiled: bool, tile_info: bool, tile: Option<u32>) -> Option<String> {
    match (tiled, tile_info, tile) {
        (false, ..) => Some(name.to_owned()),
        (true, false, _) => None,
        (true, true, Some(tile)) => Some(format!("{name} - {tile}")),
        (true, true, None) => Some(name.to_owned()),
    }
}

/// Smallest visible 2D brush outline, in points
const BRUSH_CURSOR_MIN_RADIUS_POINTS: f32 = 1.5;

/// Smaller hardness rings overlap the center dot
const BRUSH_CURSOR_MIN_RING_HARDNESS: f32 = 0.05;

/// The brush outline and hardness ring around `center`, with `radius_points` in screen points
fn brush_cursor_2d(painter: &egui::Painter, center: egui::Pos2, radius_points: f32, brush: &texturecraft_engine::paint::BrushSettings) {
    let ring = |scale: f32| -> Vec<Pos2> { cursor_outline(brush, scale).into_iter().map(|local| center + vec2(local.x, -local.y) * radius_points).collect() };
    if cursor_is_circle(brush) {
        let radius = radius_points * brush.alpha_params.size_x;
        painter.circle_stroke(center, radius, Stroke::new(2.5, Color32::from_black_alpha(140)));
        painter.circle_stroke(center, radius, Stroke::new(1.2, Color32::from_white_alpha(230)));
    } else {
        let outline = ring(1.0);
        painter.add(egui::Shape::line(outline.clone(), Stroke::new(2.5, Color32::from_black_alpha(140))));
        painter.add(egui::Shape::line(outline, Stroke::new(1.2, Color32::from_white_alpha(230))));
    }
    if brush.hardness > BRUSH_CURSOR_MIN_RING_HARDNESS {
        painter.add(egui::Shape::dashed_line(&ring(brush.hardness), Stroke::new(1.0, Color32::from_white_alpha(120)), 3.0, 3.0));
    }
    painter.circle_filled(center, 1.5, Color32::WHITE);
}

/// Paint in the 2D view: a press starts a stroke, a drag continues it, Shift+click draws a line from the last stroke
/// (`(shift, snap)`: Ctrl+Shift turns the line to a multiple of 15 degrees on screen).
fn paint_2d_input(
    app: &mut TextureCraftApp,
    ui: &Ui,
    response: &egui::Response,
    (shift, snap): (bool, bool),
    to_uv: &dyn Fn(egui::Pos2) -> Vec2,
    to_screen: &dyn Fn(f32, f32) -> egui::Pos2,
) {
    let pressure = pen_pressure(ui);

    // Where the stroke under way is; it becomes the line start only when the stroke finishes
    let stroke_end = ui.scope_id().with("paint_2d_stroke_end");

    if response.drag_started_by(PointerButton::Primary) || response.clicked_by(PointerButton::Primary) {
        let line_start = app.last_paint_uv.filter(|_| shift).and_then(|point| point.uv_for(&app.session));
        if let Some(pointer) = response.interact_pointer_pos() {
            let from = line_start.filter(|_| snap).map(|start| to_screen(start.x, start.y));
            let uv = to_uv(snapped_end(from, pointer));
            match app.session.begin_stroke_uv(line_start.unwrap_or(uv), pressure) {
                Ok(()) => {
                    if line_start.is_some() {
                        app.session.continue_stroke_uv(uv, pressure);
                    }
                    ui.data_mut(|data| data.insert_temp(stroke_end, crate::uv_paint_point::UvPaintPoint::new(&app.session, uv)));
                }
                Err(error) => app.toast(error.to_string(), true),
            }
        }
    } else if response.dragged_by(PointerButton::Primary)
        && app.session.stroke_active()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let uv = to_uv(pointer);
        app.session.continue_stroke_uv(uv, pressure);
        ui.data_mut(|data| data.insert_temp(stroke_end, crate::uv_paint_point::UvPaintPoint::new(&app.session, uv)));
    }

    // Escape cancels the stroke and ends the drag without a primary release, so a cancelled stroke never gets here
    if response.clicked_by(PointerButton::Primary) || response.drag_stopped_by(PointerButton::Primary) {
        let finishing = app.session.stroke_active();
        match app.session.end_stroke() {
            Ok(()) if finishing => {
                app.last_paint_uv = ui.data_mut(|data| {
                    let endpoint = data.get_temp(stroke_end);
                    data.remove::<crate::uv_paint_point::UvPaintPoint>(stroke_end);
                    endpoint
                });
            }
            Ok(()) => {}
            Err(error) => app.toast(error.to_string(), true),
        }
    }
}

/// Paint the skew map under the pointer in the 2D view (a click is one dab, a drag a stroke).
fn bake_paint_input(app: &mut TextureCraftApp, ui: &Ui, resp: &egui::Response, to_uv: &dyn Fn(egui::Pos2) -> Vec2) {
    let last = ui.scope_id().with(("bake_paint_last", app.session.ts(), app.ui.bake_paint_brush.target as u8));
    let brush = app.ui.bake_paint_brush;
    let started = resp.drag_started_by(PointerButton::Primary) || resp.clicked_by(PointerButton::Primary);
    if !(started || resp.dragged_by(PointerButton::Primary)) {
        return;
    }
    let Some(p) = resp.interact_pointer_pos() else { return };
    let uv = to_uv(p);
    let previous: Option<[f32; 2]> = if started { None } else { ui.data(|d| d.get_temp(last)) };
    let points: Vec<[f32; 2]> = previous.into_iter().chain([[uv.x, uv.y]]).collect();
    let command = match brush.target {
        texturecraft_engine::core::BakePaintTarget::Skew => "bake.skew.paint",
        texturecraft_engine::core::BakePaintTarget::Cage => "bake.cage.paint",
    };
    if app.run(command, json!({"points": points, "radius": brush.radius, "value": brush.value, "hardness": brush.hardness, "continue": !started})).is_some() {
        ui.data_mut(|d| d.insert_temp(last, [uv.x, uv.y]));
    }
}

/// The skew map as a translucent orange layer over the UV square from `top_left` to `bottom_right`.
fn bake_paint_overlay(app: &TextureCraftApp, ui: &Ui, painter: &egui::Painter, to_screen: &dyn Fn(f32, f32) -> egui::Pos2) {
    let Some(k) = app.session.doc.bake_paint.get(&(app.session.ts(), app.ui.bake_paint_brush.target)).filter(|k| k.values.len() > 1) else { return };
    let key = (app.session.revision, std::sync::Arc::as_ptr(k) as usize);
    let id = ui.scope_id().with("bake_paint_overlay_texture");
    let cached: Option<((u64, usize), egui::TextureHandle)> = ui.data(|d| d.get_temp(id));
    let tex = match cached {
        Some((c, t)) if c == key => t,
        _ => {
            // At most 256 texels a side: the overlay is a guide, not the map
            let (w, h) = (k.width as usize, k.height as usize);
            let (ow, oh) = (w.min(256), h.min(256));
            let mut rgba = Vec::with_capacity(ow * oh * 4);
            for y in 0..oh {
                for x in 0..ow {
                    let v = k.values.get((y * h / oh) * w + x * w / ow).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                    let v = match app.ui.bake_paint_brush.target {
                        texturecraft_engine::core::BakePaintTarget::Skew => v,
                        texturecraft_engine::core::BakePaintTarget::Cage => 1.0 - v,
                    };
                    rgba.extend_from_slice(&[255, 140, 40, (v * 170.0) as u8]);
                }
            }
            let tex = ui.ctx().load_texture("bake_paint_overlay", egui::ColorImage::from_rgba_unmultiplied([ow, oh], &rgba), egui::TextureOptions::NEAREST);
            ui.data_mut(|d| d.insert_temp(id, (key, tex.clone())));
            tex
        }
    };
    let mut mesh = egui::Mesh::with_texture(tex.id());
    for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        mesh.vertices.push(egui::epaint::Vertex { pos: to_screen(u, v), uv: pos2(u, v), color: Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

/// Hands the base colour on the model at `pixel` to the colour bar `bar` and ends its eyedropper pick; a click off the model ends it too
fn eyedrop(app: &mut TextureCraftApp, ctx: &egui::Context, bar: egui::Id, pixel: Vec2, viewport: Vec2) {
    app.picking_color = None;
    let Ok(values) = app.session.color_at(pixel, viewport) else { return };

    if let Some(color) = values.get(&texturecraft_engine::core::Channel::BaseColor) {
        crate::color_picker::ColorPick::deliver(ctx, bar, *color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn hovering_a_texture_set_row_previews_it_without_selecting_it() {
        use crate::state::Mode;
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        // App creation and frames both set the shared UI language
        let _language_guard = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        let session =
            Session::new_project("Texture set hover", texturecraft_engine::geom::primitives::material_ball(), "builtin:material_ball", None, 128).unwrap();
        let hovered_set = 1;
        let name = session.doc.project.texture_sets[hovered_set].name.clone();
        let selected_set = session.ts();
        assert_ne!(hovered_set, selected_set);

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1600.0, 1500.0))
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|context| TextureCraftApp::new(context, session));
        let preview = |harness: &Harness<'_, TextureCraftApp>| {
            harness.ctx.data(|data| data.get_temp::<Option<usize>>(egui::Id::unique(TEST_SCENE_SET_HOVER))).expect("the 3D view submitted a scene frame")
        };

        for mode in [Mode::Painting, Mode::Baking] {
            harness.state_mut().ui.mode = mode;
            harness.run_steps(4);
            assert_eq!(preview(&harness), None, "{mode:?}: the preview starts clear");

            harness.get_by_label(&name).hover();
            harness.run_steps(3);
            assert_eq!(preview(&harness), Some(hovered_set), "{mode:?}: the hovered set reaches the viewport");
            assert_eq!(harness.state().session.ts(), selected_set, "{mode:?}: hover keeps the selected set");

            let away = harness.state().viewport_rect.center();
            harness.event(egui::Event::PointerMoved(away));
            harness.run_steps(3);
            assert_eq!(preview(&harness), None, "{mode:?}: leaving the row clears the preview");
            assert_eq!(harness.state().session.ts(), selected_set, "{mode:?}: leaving the row keeps the selected set");
        }
    }

    #[test]
    fn the_hover_label_finds_the_tile_under_the_cursor_in_a_turned_view() {
        let session = Session::new_project("Tiles", texturecraft_engine::geom::primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
        let name = session.doc.project.texture_sets[0].name.clone();
        let size = egui::vec2(800.0, 600.0);
        let set = &session.doc.project.texture_sets[0];
        for rotation in [0.0, 0.7, std::f32::consts::FRAC_PI_2] {
            let scale = texturecraft_render::uv_view_scale(size.x / size.y, 1.0, set.size());
            let map = crate::uv_view_map::UvViewMap { rect: Rect::from_min_size(Pos2::ZERO, size), scale, pan: [0.1, 0.0], rotation };

            // The atlas is two tiles wide: 1001 is its left half, 1002 its right half
            let left = map.to_screen(0.25, 0.5);
            let right = map.to_screen(0.75, 0.5);
            assert_eq!(hover_label(&session, true, map, Some(left)), Some(format!("{name} - 1001")), "{rotation}");
            assert_eq!(hover_label(&session, true, map, Some(right)), Some(format!("{name} - 1002")), "{rotation}");
            assert_eq!(hover_label(&session, false, map, Some(left)), None, "tile info off");
        }
    }

    #[test]
    fn the_axis_gizmo_points_along_the_texture_s_u_and_v_edges_at_any_rotation() {
        for rotation in [0.0, 0.6, std::f32::consts::FRAC_PI_2, 2.5, 4.0] {
            let view = crate::uv_view_map::UvViewMap {
                rect: Rect::from_min_size(Pos2::ZERO, egui::vec2(500.0, 300.0)),
                scale: [0.4, 0.6],
                pan: [0.2, -0.1],
                rotation,
            };
            let (u, v) = uv_axis_directions(rotation);
            let u_edge = (view.to_screen(1.0, 1.0) - view.to_screen(0.0, 1.0)).normalized();
            let v_edge = (view.to_screen(0.0, 0.0) - view.to_screen(0.0, 1.0)).normalized();
            assert!((u - u_edge).length() < 1e-4 && (v - v_edge).length() < 1e-4, "{rotation}: {u:?} {u_edge:?} / {v:?} {v_edge:?}");
        }
        let (u, v) = uv_axis_directions(std::f32::consts::FRAC_PI_2);
        assert!((u - egui::vec2(0.0, -1.0)).length() < 1e-4 && (v - egui::vec2(-1.0, 0.0)).length() < 1e-4, "a counter-clockwise quarter turn: U up, V left");
    }

    #[test]
    fn the_hover_label_names_the_set_and_the_tile_under_the_cursor() {
        assert_eq!(hover_label_text("UdimMat", true, true, Some(1001)).as_deref(), Some("UdimMat - 1001"));
        assert_eq!(hover_label_text("UdimMat", true, true, None).as_deref(), Some("UdimMat"), "off the tiles");
        assert_eq!(hover_label_text("MatCube1", false, true, None).as_deref(), Some("MatCube1"), "a set that is not tiled");
    }

    #[test]
    fn tile_info_off_hides_the_label_of_a_tiled_set_only() {
        assert_eq!(hover_label_text("UdimMat", true, false, Some(1001)), None);
        assert_eq!(hover_label_text("MatCube1", false, false, None).as_deref(), Some("MatCube1"), "the button only exists in UV tile projects");
    }

    #[test]
    fn framing_tile_1001_centers_it_and_fits_it_like_a_single_tile_set() {
        let mut set = texturecraft_engine::core::TextureSet::new("Tiles", 0, 128);
        set.uv_tiles = vec![1001, 1002, 1011];
        let size = egui::vec2(800.0, 600.0);
        let (zoom, pan) = framed_tile_view(&set, size);

        // Tile 1001 is the lower-left block of a 2 x 2 atlas
        assert!((zoom - 2.0).abs() < 1e-4, "{zoom}");
        let [scale_x, scale_y] = texturecraft_render::uv_view_scale(size.x / size.y, zoom, set.size());
        let view = crate::uv_view_map::UvViewMap { rect: Rect::from_min_size(Pos2::ZERO, size), scale: [scale_x, scale_y], pan, rotation: 0.0 };
        let center = view.to_screen(0.25, 0.75);
        assert!((center - view.rect.center()).length() < 1e-2, "{center:?}");

        // The tile fills the view as one tile alone would
        let single = texturecraft_render::uv_view_scale(size.x / size.y, 1.0, (128, 128));
        let tile_width = (view.to_screen(0.5, 0.75) - view.to_screen(0.0, 0.75)).length();
        assert!((tile_width - single[0] * size.x).abs() < 1e-2, "{tile_width}");
    }

    #[test]
    fn a_set_without_tiles_frames_the_whole_texture() {
        let set = texturecraft_engine::core::TextureSet::new("Plain", 0, 128);
        assert_eq!(framed_tile_view(&set, egui::vec2(800.0, 600.0)), (1.0, [0.0, 0.0]));
    }

    use texturecraft_engine::paint::{AlphaParams, BrushAlpha, BrushSettings};

    fn extent(points: &[Vec2]) -> Vec2 {
        let (low, high) = points.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(low, high), point| (low.min(*point), high.max(*point)));
        high - low
    }

    #[test]
    fn the_cursor_outline_follows_size_roundness_and_angle() {
        let plain = cursor_outline(&BrushSettings::default(), 1.0);
        assert!(extent(&plain).abs_diff_eq(Vec2::splat(2.0), 1e-3), "a circle at the defaults: {:?}", extent(&plain));
        assert!(plain.iter().all(|point| (point.length() - 1.0).abs() < 1e-3));

        let params = AlphaParams { size_x: 0.5, size_y: 0.25, ..AlphaParams::default() };
        let narrow = BrushSettings { alpha_params: params, ..BrushSettings::default() };
        assert!(extent(&cursor_outline(&narrow, 1.0)).abs_diff_eq(Vec2::new(1.0, 0.5), 1e-3));
        assert!(extent(&cursor_outline(&narrow, 0.5)).abs_diff_eq(Vec2::new(0.5, 0.25), 1e-3), "the hardness ring scales with it");

        let turned = BrushSettings { angle: 90.0, ..narrow.clone() };
        assert!(extent(&cursor_outline(&turned, 1.0)).abs_diff_eq(Vec2::new(0.5, 1.0), 1e-3), "the brush angle turns the extent");

        let square = BrushSettings { alpha_params: AlphaParams { roundness: 0.0, ..AlphaParams::default() }, ..BrushSettings::default() };
        let corner = cursor_outline(&square, 1.0).into_iter().map(Vec2::length).fold(0.0, f32::max);
        assert!((corner - 2f32.sqrt()).abs() < 0.05, "roundness 0 reaches the corners: {corner}");

        let star = BrushSettings { alpha: BrushAlpha::Star, ..square };
        assert!(cursor_outline(&star, 1.0).iter().all(|point| (point.length() - 1.0).abs() < 1e-3), "only the Round alpha has a roundness");
    }

    #[test]
    fn unavailable_tools_finish_strokes_and_box_fills() {
        // Hold the language lock while drawing the harness
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        use texturecraft_engine::{StackTarget, core::LayerId, geom::primitives};

        for tool in [Tool::Paint, Tool::PolygonFill] {
            let session = Session::new_project("t", primitives::plane(2.0, 4), "test", None, 128).unwrap();
            let mut harness = egui_kittest::Harness::builder()
                .with_context_setup(crate::framework::install_headless)
                .build_eframe(|context| TextureCraftApp::new(context, session));
            let app = harness.state_mut();
            app.session.tool.tool = tool;
            let history = app.session.history_labels().len();
            if tool == Tool::Paint {
                app.session.begin_stroke_uv(Vec2::splat(0.5), 1.0).unwrap();

                assert!(app.session.stroke_active());
            } else {
                assert!(app.session.drag_polygon_fill_box_uv(Vec2::ZERO, Vec2::ONE).unwrap() > 0);
            }
            app.session.target = StackTarget::Effect(LayerId(u64::MAX));
            assert!(!tool_acts(app));
            finish_unavailable_tool_gestures(app, None);
            assert!(!app.session.stroke_active());
            assert_eq!(app.session.history_labels().len(), history + 1);
            assert_eq!(app.session.end_polygon_fill_box(), 0, "the interrupted box is already closed");
            finish_unavailable_tool_gestures(app, None);
            assert_eq!(app.session.history_labels().len(), history + 1, "cleanup records the gesture only once");
        }
    }

    #[test]
    fn the_mask_label_hides_while_the_quick_mask_replaces_the_mask_view() {
        use texturecraft_engine::{core::LayerId, geom::primitives};

        // The harness draws the UI, so another test's interface language must not be active
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let session = Session::new_project("t", primitives::plane(2.0, 4), "test", None, 128).unwrap();
        let mut harness = egui_kittest::Harness::builder()
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|context| TextureCraftApp::new(context, session));
        let app = harness.state_mut();
        app.ui.mode = crate::state::Mode::Painting;
        app.session.view = ViewMode::Mask(LayerId(7));

        assert_eq!(mask_view_layer(app), Some(LayerId(7)));

        app.session.quick_mask.editing = true;

        assert!(quick_mask_shown(app));
        assert_eq!(mask_view_layer(app), None, "the quick mask is on screen, not the layer mask");

        app.session.quick_mask.editing = false;

        assert_eq!(mask_view_layer(app), Some(LayerId(7)), "the label returns with the mask view");
    }

    #[test]
    fn picking_shows_the_id_mask_its_color_selection_reads() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        use texturecraft_engine::{StackTarget, execute};

        let mut session = texturecraft_engine::sample_session(256, 64).unwrap();
        execute(&mut session, "layer.add.fill", &json!(null)).unwrap();
        let effect = texturecraft_engine::core::LayerId(execute(&mut session, "mask.add.colorSelection", &json!(null)).unwrap()["effect"].as_u64().unwrap());
        session.target = StackTarget::Effect(effect);
        let mut harness = egui_kittest::Harness::builder()
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|context| TextureCraftApp::new(context, session));
        harness.run_steps(2);

        crate::id_color_pick::IdColorPick::start(harness.state_mut(), effect, None);

        assert_eq!(aux_view(harness.state()), Some(crate::gpu::AuxView::IdMask(effect)), "the pick's own ID mask");

        crate::id_color_pick::IdColorPick::end(harness.state_mut());

        assert_eq!(aux_view(harness.state()), None, "the earlier view");
    }

    /// A click in the 2D view while a colour bar's eyedropper waits paints nothing: the pick keeps waiting, as over the 3D view
    #[test]
    fn a_waiting_eyedropper_keeps_a_2d_view_click_from_painting() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let session = texturecraft_engine::sample_session(256, 64).unwrap();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1600.0, 960.0))
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|context| TextureCraftApp::new(context, session));
        harness.state_mut().ui.layout = ViewLayout::Only2D;
        harness.state_mut().session.tool.tool = Tool::PolygonFill;
        harness.run_steps(4);

        // The 2D view fills the window's middle: there the sample's UV islands are (the shelf drop tests click the same point)
        let on_the_uvs = pos2(700.0, 420.0);
        let steps = harness.state().session.history_labels().len();
        let bar = egui::Id::unique("a colour bar");
        harness.state_mut().picking_color = Some(bar);

        click(&mut harness, on_the_uvs);

        assert_eq!(harness.state().session.history_labels().len(), steps, "the click painted nothing");
        assert_eq!(harness.state().picking_color, Some(bar), "the eyedropper still waits");

        harness.state_mut().picking_color = None;
        click(&mut harness, on_the_uvs);

        assert_eq!(harness.state().session.undo_label(), Some("Polygon fill"), "without a pick the same click paints");
    }

    /// Press and release the primary button at `at`
    fn click(harness: &mut egui_kittest::Harness<'_, TextureCraftApp>, at: Pos2) {
        harness.hover_at(at);
        harness.run_steps(1);
        harness.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        harness.run_steps(1);
        harness.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        harness.run_steps(2);
    }

    #[test]
    fn stored_surfaces_release_after_a_frame_without_paths() {
        // Hold the language lock while drawing the harness
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        use texturecraft_engine::{core::EffectKind, execute, geom::primitives};

        let mut session = Session::new_project("Path cache", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 256).unwrap();
        session.viewport = Vec2::splat(400.0);
        session.camera.yaw = 0.0;
        session.camera.pitch = 0.0;
        let result = execute(&mut session, "tool.path", &json!({"points": [[140, 210], [200, 180], [260, 210]], "commit": true})).unwrap();
        let effect = texturecraft_engine::core::LayerId(result["committed"]["effect"].as_u64().unwrap());
        let EffectKind::Path(paths) = &mut session.doc.project.effect_mut(effect).unwrap().kind else { panic!("path effect") };
        paths.resize(8, paths[0].clone());
        let mut harness = egui_kittest::Harness::builder()
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|context| TextureCraftApp::new(context, session));
        let context = egui::Context::default();
        let mut previous = Vec::<std::sync::Weak<Vec<Vec<Option<Vec3>>>>>::new();

        // Every path in the selection stays warm, and leaving it releases every sampled buffer
        for _ in 0..4 {
            let output = context.run_ui(Default::default(), |ui| {
                for index in 0..8 {
                    let surface = stored_surface(harness.state(), ui, effect, index).unwrap();

                    assert!(surface.iter().map(Vec::len).sum::<usize>() > 10);
                    if let Some(cached) = previous.get(index) {
                        assert!(Arc::ptr_eq(&cached.upgrade().unwrap(), &surface), "unchanged paths must not be resampled");
                    } else {
                        previous.push(Arc::downgrade(&surface));
                    }
                }
            });

            // This cache lifecycle test has no renderer to consume texture updates
            output.drop_without_applying_deltas();
        }

        let EffectKind::Path(paths) = &mut harness.state_mut().session.doc.project.effect_mut(effect).unwrap().kind else { panic!("path effect") };
        for path in paths {
            path.viewport = [800.0, 800.0];
        }
        let output = context.run_ui(Default::default(), |ui| {
            for (index, previous) in previous.iter_mut().enumerate() {
                let surface = stored_surface(harness.state(), ui, effect, index).unwrap();
                assert!(previous.upgrade().is_none(), "editing a path releases its obsolete samples");
                *previous = Arc::downgrade(&surface);
            }
        });
        output.drop_without_applying_deltas();

        for _ in 0..2 {
            context.run_pass(Default::default(), |_| {}).drop_without_applying_deltas();
        }

        assert!(previous.iter().all(|surface| surface.upgrade().is_none()), "unused path surfaces must be freed while the window stays open");
    }

    /// A selected mask decal on the ball, with another texture set available for selection tests
    fn decal_frame_app(render: bool) -> egui_kittest::Harness<'static, TextureCraftApp> {
        let session =
            Session::new_project("Decal selection", texturecraft_engine::geom::primitives::material_ball(), "builtin:material_ball", None, 256).unwrap();
        let builder = egui_kittest::Harness::builder().with_size(vec2(1600.0, 960.0));
        let builder = if render { builder.wgpu() } else { builder };
        let mut harness = builder.with_context_setup(crate::framework::install_headless).build_eframe(|context| TextureCraftApp::new(context, session));
        harness.run_steps(4);
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 255, 255, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let image = harness.state_mut().session.import_resource_bytes("decal.png", png).unwrap();
        harness.state_mut().run("layer.add.decal", json!({"image": image, "channel": "mask", "size": 0.5})).unwrap();
        harness.run_steps(2);
        assert_eq!(harness.state().session.ts(), 0);
        assert!(crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).is_some());
        harness
    }

    /// Move a primary-button drag through enough frames to reach the viewport's decal handles
    fn drag_decal_handle(harness: &mut egui_kittest::Harness<'_, TextureCraftApp>, from: Pos2, to: Pos2) {
        let modifiers = egui::Modifiers::NONE;
        harness.hover_at(from);
        harness.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers });
        harness.run_steps(2);
        for step in 1..=4 {
            harness.hover_at(from.lerp(to, step as f32 / 4.0));
            harness.run_steps(1);
        }
        harness.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers });
        harness.run_steps(2);
    }

    #[test]
    fn decal_frame_rejects_a_stale_effect_after_failed_layer_selection() {
        // Hold the language lock while drawing the harness
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        use texturecraft_engine::{StackTarget, execute};

        for texture_set in [0, 1] {
            let mut harness = decal_frame_app(false);
            let original = crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).unwrap();
            let effect = original.effect.unwrap();
            let session = &mut harness.state_mut().session;
            execute(session, "textureSet.select", &json!({"ts": texture_set})).unwrap();
            let other = execute(session, "layer.add.paint", &json!({"name": "Other layer"})).unwrap()["layer"].clone();
            execute(session, "layer.select", &json!({"layer": original.layer.0, "target": effect.0})).unwrap();

            let selected = execute(session, "layer.select", &json!({"layer": other, "target": effect.0}));

            assert!(selected.is_err());
            assert_eq!(session.ts(), texture_set);
            assert_eq!(session.selected_layer().unwrap().0, other.as_u64().unwrap());
            assert_eq!(session.target, StackTarget::Effect(effect), "the failed selection leaves the previous effect target");
            assert!(crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).is_none(), "a stale effect must not show a decal frame");

            drag_decal_handle(&mut harness, original.corners[1], original.center + (original.corners[1] - original.center) * 1.5);
            drag_decal_handle(&mut harness, original.center, original.center + vec2(40.0, 0.0));

            let effect = &harness.state().session.doc.project.effect(effect).unwrap().1.kind;

            assert!(
                matches!(effect, texturecraft_engine::core::EffectKind::Fill { projection: texturecraft_engine::core::Projection::Planar { placed: Some(decal), .. }, .. } if *decal == original.decal)
            );
        }
    }

    #[test]
    fn decal_frame_keeps_an_instances_inherited_mask_effect_editable() {
        // Hold the language lock while drawing the harness
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        use texturecraft_engine::execute;

        let mut harness = decal_frame_app(false);
        let original = crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).unwrap();
        let effect = original.effect.unwrap();
        let session = &mut harness.state_mut().session;
        let instance = execute(session, "layer.instance", &json!({"layer": original.layer.0, "ts": 1})).unwrap()["layer"].clone();

        // The source effect is a valid inherited target when selection changes to its instance
        execute(session, "layer.select", &json!({"layer": instance, "target": effect.0})).unwrap();
        assert_eq!(session.target, texturecraft_engine::StackTarget::Effect(effect));

        assert_eq!(session.ts(), 1);
        assert_eq!(session.selected_layer().unwrap().0, instance.as_u64().unwrap());

        let inherited = crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).unwrap();

        assert_eq!((inherited.layer, inherited.effect, inherited.decal), (original.layer, original.effect, original.decal));

        drag_decal_handle(&mut harness, inherited.corners[1], inherited.center + (inherited.corners[1] - inherited.center) * 1.5);

        let edited = crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).unwrap();

        assert!(edited.decal.size[0] > original.decal.size[0], "scaling an inherited decal edits its source");
    }

    #[test]
    fn decal_frame_rejects_a_source_mask_overridden_by_the_selected_instance() {
        // Hold the language lock while drawing the harness
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        use texturecraft_engine::execute;

        let mut harness = decal_frame_app(false);
        let original = crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).unwrap();
        let effect = original.effect.unwrap();
        let session = &mut harness.state_mut().session;
        let instance = execute(session, "layer.instance", &json!({"layer": original.layer.0, "ts": 1})).unwrap()["layer"].clone();
        execute(session, "mask.add.black", &json!({"layer": instance})).unwrap();
        execute(session, "layer.select", &json!({"layer": original.layer.0, "target": effect.0})).unwrap();

        assert!(execute(session, "layer.select", &json!({"layer": instance, "target": effect.0})).is_err());

        assert!(crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).is_none(), "an override hides the source's mask decal");
    }

    #[test]
    #[ignore = "requires an offscreen GPU adapter and TC_UI_SNAPSHOTS"]
    fn decal_frame_selection_snapshots() {
        // Hold the language lock while drawing the harness
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        use texturecraft_engine::execute;

        let Some(directory) = std::env::var_os("TC_UI_SNAPSHOTS").map(std::path::PathBuf::from) else { return };

        std::fs::create_dir_all(&directory).unwrap();
        let mut harness = decal_frame_app(true);
        let original = crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).unwrap();
        let effect = original.effect.unwrap();
        let session = &mut harness.state_mut().session;
        let instance = execute(session, "layer.instance", &json!({"layer": original.layer.0, "ts": 1})).unwrap()["layer"].clone();

        assert!(execute(session, "layer.select", &json!({"layer": instance, "target": effect.0})).is_err());

        harness.run_steps(6);

        assert!(crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).is_some());

        harness.render().unwrap().save(directory.join("decal-frame-inherited.png")).unwrap();

        let session = &mut harness.state_mut().session;
        let other = execute(session, "layer.add.paint", &json!({"name": "Other layer"})).unwrap()["layer"].clone();
        execute(session, "layer.select", &json!({"layer": original.layer.0, "target": effect.0})).unwrap();

        assert!(execute(session, "layer.select", &json!({"layer": other, "target": effect.0})).is_err());

        harness.run_steps(6);

        assert!(crate::frame_gizmo::FrameGizmo::of(harness.state(), harness.state().viewport_rect).is_none());

        harness.render().unwrap().save(directory.join("decal-frame-stale.png")).unwrap();
    }

    #[test]
    fn live_bake_preview_hides_setup_overlays_and_restores_their_preferences() {
        // Hold the language lock while drawing the harness
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let session = texturecraft_engine::sample_session(256, 64).unwrap();
        let mut harness = egui_kittest::Harness::builder()
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|context| TextureCraftApp::new(context, session));
        let app = harness.state_mut();
        app.ui.mode = crate::state::Mode::Baking;
        let before = bake_visualization(app).unwrap();

        assert!(before.high_poly.shown && before.cage_shown());

        app.start_bake(vec![0]);

        assert!(bake_visualization(app).is_none(), "setup overlays must not obscure the finished map");
        for show in [texturecraft_engine::PreviewShow::Pin(BakedMap::Normal), texturecraft_engine::PreviewShow::Material] {
            app.session.bake_preview_show(show).unwrap();
            assert!(bake_visualization(app).is_none());
        }

        app.cancel_bake();

        assert_eq!(bake_visualization(app), Some(before));
    }

    #[test]
    fn the_automatic_cage_shows_no_matching_error() {
        let session = texturecraft_engine::sample_session(256, 64).unwrap();
        let mut harness = egui_kittest::Harness::builder()
            .with_context_setup(crate::framework::install_headless)
            .build_eframe(|context| TextureCraftApp::new(context, session));
        let app = harness.state_mut();
        app.ui.mode = crate::state::Mode::Baking;
        assert!(bake_visualization(app).unwrap().matching_error.shown);

        app.session.doc.project.texture_sets[0].bake.cage.kind = texturecraft_engine::core::CageType::Automatic;

        assert!(!bake_visualization(app).unwrap().matching_error.shown);
        assert!(app.ui.bake_visualization.matching_error.shown, "the switch keeps its state for the other cage types");
    }

    const VIEW: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0));
    #[test]
    fn a_far_start_is_clipped_to_the_view_and_preserves_dash_phase() {
        let far_start = pos2(-10_003.0, 50.0);
        let end = pos2(50.0, 50.0);
        let mut dashes = Vec::new();

        visible_dashes(VIEW, &[far_start, end], 6.0, 4.0, |dash| dashes.push(dash));

        assert_eq!(dashes[0], [pos2(0.0, 50.0), pos2(3.0, 50.0)]);
        assert_eq!(dashes[1], [pos2(7.0, 50.0), pos2(13.0, 50.0)]);
        assert!(dashes.iter().flatten().all(|point| VIEW.contains(*point) && point.x <= end.x));
    }

    #[test]
    fn an_outline_keeps_its_dash_pattern_across_segments() {
        let line = [pos2(10.0, 20.0), pos2(14.0, 20.0), pos2(20.0, 20.0)];
        let mut dashes = Vec::new();

        visible_dashes(VIEW, &line, 6.0, 4.0, |dash| dashes.push(dash));

        assert_eq!(dashes, [[line[0], line[1]], [line[1], pos2(16.0, 20.0)]]);
    }

    #[test]
    fn a_segment_outside_the_view_draws_nothing() {
        let mut count = 0;

        visible_dashes(VIEW, &[pos2(-50.0, -10.0), pos2(150.0, -10.0), pos2(120.0, 300.0)], 6.0, 4.0, |_| count += 1);

        assert_eq!(count, 0);
    }
}
