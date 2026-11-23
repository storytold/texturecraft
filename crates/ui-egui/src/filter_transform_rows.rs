//! The Transform filter's rows: Scaling and Offset beside a preview of the moved tile, Tile Safe Rotation, Rotation, Repeat mode and Flip mode

use std::ops::RangeInclusive;

use egui::{Painter, Rect, pos2};
use serde_json::{Value, json};
use texturecraft_engine::core::{FlipMode, RepeatMode, TileTransform, UvWrap};

use crate::TextureCraftApp;
use crate::fill_pictures::{lattice_preview, wrap_picture};
use crate::filter_properties::DIAL_DEGREES;
use crate::i18n::tr;
use crate::property_sheet::{PreviewLower, PreviewSize, PropertySheet};
use crate::theme::Tokens;

/// What the Scaling track spans (a fill's Tiling track); typing reaches the filter's whole range
const SCALING_TRACK: RangeInclusive<f32> = 0.01..=5.0;

/// What the Offset track spans, in tiles: one tile either way covers every position of a repeating tile
const OFFSET_TRACK: RangeInclusive<f32> = -1.0..=1.0;

/// The rows in the reference's order: Scaling (linked) and Offset beside the preview, Tile Safe Rotation, Rotation, Repeat mode, Flip mode
pub(crate) fn transform_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, id: u64, transform: &TileTransform) {
    let set = |app: &mut TextureCraftApp, key: &str, value: Value, merge: bool| {
        app.run("effect.set", json!({"effect": id, key: value, "merge": merge}));
    };
    let tokens = Tokens::get(sheet.ui.ctx());

    let link_id = sheet.ui.make_persistent_id(("transform_scaling_link", id));
    let mut linked = sheet.ui.data(|data| data.get_temp::<bool>(link_id)).unwrap_or(transform.scaling[0] == transform.scaling[1]);
    let was_linked = linked;
    let (low, high) = TileTransform::SCALING;
    let mut scaling = transform.scaling;
    let size = PreviewSize { label: tr("Scaling"), values: &mut scaling, range: SCALING_TRACK, typed: low..=high, decimals: 2, linked: &mut linked };
    let mut offset = transform.offset;
    let reach = TileTransform::MAX_OFFSET;
    let lower = PreviewLower::Offset { values: &mut offset, range: OFFSET_TRACK, typed: -reach..=reach };
    let (map, repeat) = (transform.affine(), transform.repeat == RepeatMode::HorizontallyAndVertically);
    let picture = |painter: &Painter, rect: Rect| lattice_preview(painter, rect, map, repeat, &tokens);
    let edit = sheet.preview_block(&picture, size, lower);
    if linked != was_linked {
        sheet.ui.data_mut(|data| data.insert_temp(link_id, linked));
    }
    if let Some(merge) = edit.size
        && scaling != transform.scaling
    {
        set(app, "scaling", json!(scaling), merge);
    }
    if let Some(merge) = edit.lower
        && offset != transform.offset
    {
        set(app, "offset", json!(offset), merge);
    }

    let mut tile_safe = transform.tile_safe;
    if sheet.toggle(tr("Tile Safe Rotation"), &mut tile_safe) {
        set(app, "tileSafeRotation", json!(tile_safe), false);
    }
    let mut degrees = transform.rotation;
    if let Some(merge) = sheet.angle(tr("Rotation"), &mut degrees, DIAL_DEGREES) {
        set(app, "rotation", json!(degrees), merge);
    }

    let repeat_picture = |painter: &Painter, rect: Rect, mode: RepeatMode| {
        let wrap = if mode == RepeatMode::None { UvWrap::None } else { UvWrap::Repeat };
        wrap_picture(painter, rect, wrap, &tokens);
    };
    let name = |mode: RepeatMode| tr(mode.label()).to_owned();
    if let (Some(picked), _) = sheet.dropdown(tr("Repeat mode"), transform.repeat, &RepeatMode::ALL, name, Some(&repeat_picture)) {
        set(app, "repeatMode", json!(picked.id()), false);
    }
    let flip_picture = |painter: &Painter, rect: Rect, mode: FlipMode| flip_picture(painter, rect, mode, &tokens);
    let name = |mode: FlipMode| tr(mode.label()).to_owned();
    if let (Some(picked), _) = sheet.dropdown(tr("Flip mode"), transform.flip, &FlipMode::ALL, name, Some(&flip_picture)) {
        set(app, "flipMode", json!(picked.id()), false);
    }
}

/// An L, turned over the way the flip mode turns the tile (no letters, so it reads in any language)
fn flip_picture(painter: &Painter, rect: Rect, mode: FlipMode, tokens: &Tokens) {
    const L_SHAPE: [[f32; 2]; 6] = [[0.3, 0.2], [0.45, 0.2], [0.45, 0.65], [0.75, 0.65], [0.75, 0.8], [0.3, 0.8]];
    let [across, down] = mode.sides();

    // An L is not convex: two rectangles draw it
    let [top, _, joint, _, far, bottom] = L_SHAPE.map(|[u, v]| {
        let (u, v) = (if across { 1.0 - u } else { u }, if down { 1.0 - v } else { v });
        pos2(rect.left() + u * rect.width(), rect.top() + v * rect.height())
    });
    painter.rect_filled(Rect::from_two_pos(top, pos2(joint.x, bottom.y)), 0, tokens.accent);
    painter.rect_filled(Rect::from_two_pos(joint, far), 0, tokens.accent);
}
