//! The Stencil section of a brush tool's Properties: the stencil's slot with its picker, its Tiling mode, and its placement beside a picture of where it sits in the view

use std::ops::RangeInclusive;

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind, TextureId, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::session::StencilState;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::picker_group::PickerGroup;
use crate::picker_tab::PickerTab;
use crate::property_sheet::{PropertySheet, StackSlider};
use crate::resource_picker;
use crate::resource_picker_item::ResourcePickerItem;
use crate::stencil_choice::StencilChoice;
use crate::theme::Tokens;

/// Scale = the image's height as a fraction of the view's
const SCALE_RANGE: RangeInclusive<f32> = 0.05..=4.0;

/// How far the Position sliders move the stencil's center from the middle of the view (NDC; -1..1 is the view)
const POSITION_RANGE: RangeInclusive<f32> = -2.0..=2.0;

/// The Rotation dial's range in degrees
const ROTATION_RANGE: RangeInclusive<f32> = 0.0..=360.0;

/// The view's outline in the placement picture
const VIEW_OUTLINE_WIDTH: f32 = 1.0;

/// Air between the placement picture's edge and the view drawn in it
const VIEW_INSET: f32 = 6.0;

/// A Tiling mode picture's cell: its share of the picture and the air between two cells
const TILE_CELL_FRACTION: f32 = 0.42;
const TILE_CELL_GAP: f32 = 2.0;

/// The Stencil section under its caption, whose reset puts the stencil back in the middle of the view
pub(crate) fn stencil_section(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>) {
    let moved = app.session.tool.stencil.as_ref().is_some_and(|stencil| placement(stencil) != placement(&StencilState::new(&stencil.image)));
    if sheet.section_reset(tr("Stencil"), moved)
        && let Some(stencil) = &app.session.tool.stencil
    {
        let start = StencilState::new(&stencil.image);
        app.run("tool.stencil", json!({"center": start.center, "scale": start.scale, "rotation": start.rotation}));
    }

    stencil_slot(app, sheet);
    let Some(stencil) = app.session.tool.stencil.clone() else { return };

    let tile_name = |tile: bool| if tile { tr("H and V Tiling") } else { tr("No Tiling") }.to_owned();
    let text = Tokens::get(sheet.ui.ctx()).text;
    let tile_picture = |painter: &Painter, rect: Rect, tile: bool| paint_tiling(painter, rect, tile, text);
    let (picked, _) = sheet.dropdown(tr("Tiling mode"), stencil.tile, &[false, true], tile_name, Some(&tile_picture));
    if let Some(tile) = picked {
        app.run("tool.stencil", json!({ "tile": tile }));
    }

    placement_rows(app, sheet, &stencil);

    let mut opacity = stencil.opacity;
    if sheet.slider(tr("Overlay opacity"), &mut opacity, 0.0..=1.0, 2).is_some() {
        app.run("tool.stencil", json!({ "opacity": opacity }));
    }
}

/// What the section's reset puts back: where the stencil sits, its size and its turn
fn placement(stencil: &StencilState) -> ([f32; 2], f32, f32) {
    (stencil.center, stencil.scale, stencil.rotation)
}

/// The slot showing the stencil's image; a click opens the picker of images, × takes the stencil away
fn stencil_slot(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>) {
    let ctx = sheet.ui.ctx().clone();
    let current = app.session.tool.stencil.as_ref().map(|stencil| StencilChoice::Image(stencil.image.clone()));
    let texture = match &current {
        Some(StencilChoice::Image(name)) => app.session.images.get(name).map(|image| app.thumbs.image(&ctx, name, image)),
        _ => None,
    };
    let (name, source) = match &current {
        Some(StencilChoice::Image(name)) => (name.clone(), tr("Imported image")),
        _ => (tr("None").to_owned(), tr("Pick an image to paint through")),
    };
    let tokens = Tokens::get(&ctx);
    let thumbnail = |painter: &Painter, rect: Rect| {
        painter.rect_filled(rect, CornerRadius::ZERO, tokens.field);
        if let Some(texture) = &texture {
            painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    };
    let slot = sheet.slot(&thumbnail, &name, source, None, current.is_some(), false);
    let slot_pick =
        slot.pick.clone().on_hover_text(tr("S + left-drag rotates (with Shift in 15° steps), S + right-drag scales, S + middle-drag moves the stencil"));

    let popup = sheet.ui.make_persistent_id("brush_stencil_source");
    if slot_pick.clicked() {
        resource_picker::toggle(&ctx, popup);
    }
    let tabs = || {
        let textures = ResourcePickerItem::textures(&app.session, StencilChoice::Import, |name| StencilChoice::Image(name.to_owned()));
        vec![PickerTab { icon: ic::FRAME_CORNERS, name: tr("Stencils"), groups: vec![PickerGroup { header: None, items: textures }] }]
    };
    match resource_picker::show(&slot_pick, popup, &mut app.thumbs, current.as_ref(), true, tabs) {
        Some(StencilChoice::Import) => crate::menus::invoke(app, &ctx, "resource.import"),
        Some(choice) if Some(&choice) != current.as_ref() => {
            if let StencilChoice::Image(name) = choice {
                app.run("tool.stencil", json!({ "image": name }));
            }
        }
        _ => {}
    }
    if slot.clear {
        app.run("tool.stencil", json!({"clear": true}));
    }

    // An image from the Assets panel dropped on the slot becomes the stencil
    if let Some(name) = crate::dock::image_drop(sheet.ui, &slot_pick) {
        app.run("tool.stencil", json!({ "image": name }));
    }
    sheet.close_card();
}

/// Scale, Rotation and Position X / Y beside a picture of the view with the stencil placed in it
fn placement_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, stencil: &StencilState) {
    let ctx = sheet.ui.ctx().clone();
    let tokens = Tokens::get(&ctx);
    let image = app.session.images.get(&stencil.image).cloned();
    let texture = image.as_ref().map(|image| app.thumbs.stencil(&ctx, &stencil.image, image));
    let view_aspect = app.session.viewport.x.max(1.0) / app.session.viewport.y.max(1.0);
    let image_aspect = image.as_ref().map_or(1.0, |image| image.width as f32 / image.height.max(1) as f32);
    let picture = |painter: &Painter, rect: Rect| {
        painter.rect_filled(rect, CornerRadius::ZERO, tokens.field);
        let view = fit(rect.shrink(VIEW_INSET), view_aspect);
        painter.rect_filled(view, CornerRadius::ZERO, tokens.panel);
        if let Some(texture) = &texture {
            paint_stencil(&painter.with_clip_rect(view), view, texture.id(), stencil, image_aspect, Color32::WHITE, stencil.tile);
        }
        painter.rect_stroke(view, CornerRadius::ZERO, Stroke::new(VIEW_OUTLINE_WIDTH, tokens.text_dim), StrokeKind::Inside);
    };

    let (mut scale, mut rotation, [mut x, mut y]) = (stencil.scale, stencil.rotation, stencil.center);
    let edits = sheet.picture_sliders(
        &picture,
        &mut [
            StackSlider { label: tr("Scale"), value: &mut scale, range: SCALE_RANGE, typed: SCALE_RANGE, decimals: 2, suffix: "", dial: false },
            StackSlider { label: tr("Rotation"), value: &mut rotation, range: ROTATION_RANGE, typed: ROTATION_RANGE, decimals: 0, suffix: "°", dial: true },
            StackSlider { label: tr("Position X"), value: &mut x, range: POSITION_RANGE, typed: POSITION_RANGE, decimals: 2, suffix: "", dial: false },
            StackSlider { label: tr("Position Y"), value: &mut y, range: POSITION_RANGE, typed: POSITION_RANGE, decimals: 2, suffix: "", dial: false },
        ],
    );
    let [scale_edit, rotation_edit, x_edit, y_edit] = edits.map(|edit| edit.is_some());
    let mut params = serde_json::Map::new();
    if scale_edit {
        params.insert("scale".into(), json!(scale));
    }
    if rotation_edit {
        params.insert("rotation".into(), json!(rotation));
    }
    if x_edit || y_edit {
        params.insert("center".into(), json!([x, y]));
    }
    if !params.is_empty() {
        app.run("tool.stencil", Value::Object(params));
    }
}

/// The largest rect of `aspect` (width / height) centred in `outer`
fn fit(outer: Rect, aspect: f32) -> Rect {
    let width = outer.width().min(outer.height() * aspect);
    Rect::from_center_size(outer.center(), vec2(width, width / aspect.max(f32::EPSILON)))
}

/// Paints the stencil's image where it sits in the view `rect`, `tint`ed: one copy, or every copy that reaches the view when `tiled`; returns the middle copy's corners (top left, top right, bottom right, bottom left)
pub(crate) fn paint_stencil(
    painter: &Painter,
    rect: Rect,
    texture: TextureId,
    stencil: &StencilState,
    image_aspect: f32,
    tint: Color32,
    tiled: bool,
) -> [Pos2; 4] {
    let aspect = rect.width() / rect.height().max(1.0);
    let (center, half_size, rotation) = stencil.placement(image_aspect, aspect);
    let to_screen = |x: f32, y: f32| pos2(rect.left() + (x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - y * 0.5) * rect.height());
    let (sine, cosine) = rotation.sin_cos();

    let square = [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)];
    let corners = square.map(|(x, y)| {
        let (x, y) = (x * half_size[0], y * half_size[1]);
        to_screen(center[0] + x * cosine - y * sine, center[1] + x * sine + y * cosine)
    });

    let mut mesh = egui::Mesh::with_texture(texture);
    for ((x, y), corner) in square.into_iter().zip(corners) {
        let (position, local_x, local_y) = if tiled {
            // Map view corners back into stencil space so the sampler repeats at any scale
            let (dx, dy) = (x - center[0], y - center[1]);
            (to_screen(x, y), (dx * cosine + dy * sine) / half_size[0].max(f32::EPSILON), (dy * cosine - dx * sine) / half_size[1].max(f32::EPSILON))
        } else {
            (corner, x, y)
        };
        let uv = pos2(0.5 + local_x * 0.5, 0.5 - local_y * 0.5);
        mesh.vertices.push(egui::epaint::Vertex { pos: position, uv, color: tint });
    }

    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
    corners
}

/// A Tiling mode's picture: one square for No Tiling, four for H and V Tiling
fn paint_tiling(painter: &Painter, rect: Rect, tile: bool, color: Color32) {
    let cell = rect.width() * TILE_CELL_FRACTION;
    if !tile {
        painter.rect_filled(Rect::from_center_size(rect.center(), vec2(cell, cell)), CornerRadius::ZERO, color);
        return;
    }

    let half_step = (cell + TILE_CELL_GAP) / 2.0;
    for (direction_x, direction_y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let center = rect.center() + vec2(direction_x * half_step, direction_y * half_step);
        painter.rect_filled(Rect::from_center_size(center, vec2(cell, cell)), CornerRadius::ZERO, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tiny_offset_tiled_stencil_repeats_across_the_placement_picture() {
        let ctx = egui::Context::default();
        let view = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0));
        let image = std::sync::Arc::new(texturecraft_engine::compose::ImageRes::new(2, 8, vec![[1.0; 4]; 16], false));
        let stencil = StencilState { center: [2.0, 2.0], scale: 0.05, rotation: 90.0, tile: true, ..StencilState::new("offset") };
        let texture = crate::thumbs::Thumbs::default().stencil(&ctx, "offset", &image);

        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            paint_stencil(ui.painter(), view, texture.id(), &stencil, 0.25, Color32::WHITE, true);
        });
        let options = frame.textures_delta.set[&texture.id()][0].options;
        frame.textures_delta.clear();
        let mesh = frame
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == texture.id() => Some(mesh),
                _ => None,
            })
            .expect("the stencil picture");
        println!("Tile geometry: {} vertices, {} indices", mesh.vertices.len(), mesh.indices.len());

        assert!(mesh.calc_bounds().contains_rect(view), "the rotated tiles reach the entire view");
        if mesh.vertices.iter().any(|vertex| !(0.0..=1.0).contains(&vertex.uv.x) || !(0.0..=1.0).contains(&vertex.uv.y)) {
            assert_eq!(options.wrap_mode, egui::TextureWrapMode::Repeat, "the image repeats outside its first tile");
        }
    }
}
