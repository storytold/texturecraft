//! Properties of a compare mask: its channel, both sources and their operator, constant, tolerance, hardness and histograms

use std::sync::Arc;

use egui::{Align2, Color32, FontId, Mesh, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};
use serde_json::json;
use texturecraft_engine::core::{CompareOp, CompareSource, Layer, LayerId};

use crate::TextureCraftApp;
use crate::compare_settings::CompareSettings;
use crate::histogram_cache::{HISTOGRAM_BUCKETS, cached_histogram};
use crate::i18n::{n_, tr};
use crate::levels_editor::{GRID_FRACTIONS, HISTOGRAM_HEIGHT};
use crate::property_sheet::PropertySheet;
use crate::theme::{AXIS_COLORS, Tokens};

/// Air the tallest bucket keeps under the well's top
const HISTOGRAM_HEADROOM: f32 = 4.0;

/// From the well's bottom to its value labels' centres
const AXIS_LABEL_OFFSET: f32 = 10.0;

/// The room the value labels take under the well
const AXIS_LABEL_ROOM: f32 = 22.0;

/// Size of the value labels under the well
const AXIS_LABEL_SIZE: f32 = 10.0;

/// How much of a source's colour shows over the well; half, so where both sources overlap the one under shows through (levels' single histogram mixes more of its colour)
const HISTOGRAM_TINT: f32 = 0.5;

/// The value labels under the well, at its edges and quarter lines
const AXIS_LABELS: [&str; 5] = ["0", "0.25", "0.5", "0.75", "1"];

/// Width of the constant's line in the well
const CONSTANT_LINE_WIDTH: f32 = 2.0;

/// Gap between the well's top and the top of the constant's line
const CONSTANT_LINE_INSET: f32 = 2.0;

/// The colours that tag the left and the right source
const SOURCE_TAGS: [Color32; 2] = [AXIS_COLORS[1], AXIS_COLORS[2]];

/// The operator switch's words, in [`CompareOp::ALL`] order
const OPERATOR_WORDS: [&str; 3] = ["<", "≈", ">"];

/// The properties of compare mask `effect_id` on `layer`; every edit goes through `effect.set`
pub(crate) fn compare_properties(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer, effect_id: LayerId, settings: CompareSettings) {
    let project = &app.session.doc.project;
    let channels = project.locate(layer.id).and_then(|(set, _, _)| project.texture_set(set).ok()).map(|set| set.channel_list()).unwrap_or_default();
    let id = effect_id.0;
    let CompareSettings { op, mut constant, mut tolerance, left, right, channel, mut hardness } = settings;

    PropertySheet::show(ui, ("compare_sheet", id), |sheet| {
        sheet.section(tr("Compare mask"), false);
        if let (Some(picked), _) = sheet.dropdown(tr("Channel"), channel, &channels, |option| tr(option.label()).to_string(), None) {
            app.run("effect.set", json!({"effect": id, "compareChannel": picked.id()}));
        }

        let operator = CompareOp::ALL.iter().position(|candidate| *candidate == op).unwrap_or_default();
        let picked = sheet.compare_rows((left, right), &CompareSource::ALL, source_name, SOURCE_TAGS, (&OPERATOR_WORDS, operator));
        if let Some(source) = picked.left {
            app.run("effect.set", json!({"effect": id, "left": source.id()}));
        }
        if let Some(source) = picked.right {
            app.run("effect.set", json!({"effect": id, "right": source.id()}));
        }
        if let Some(chosen) = picked.operator.and_then(|index| CompareOp::ALL.get(index)) {
            app.run("effect.set", json!({"effect": id, "op": chosen.id()}));
        }

        if (left == CompareSource::Constant || right == CompareSource::Constant)
            && let Some(merge) = sheet.slider(tr("Constant"), &mut constant, 0.0..=1.0, 2)
        {
            app.run("effect.set", json!({"effect": id, "value": constant, "merge": merge}));
        }
        if op == CompareOp::WithinTolerance
            && let Some(merge) = sheet.slider(tr("Tolerance"), &mut tolerance, 0.0..=1.0, 2)
        {
            app.run("effect.set", json!({"effect": id, "tolerance": tolerance, "merge": merge}));
        }
        if let Some(merge) = sheet.slider(tr("Hardness"), &mut hardness, 0.0..=1.0, 2) {
            app.run("effect.set", json!({"effect": id, "hardness": hardness, "merge": merge}));
        }

        if sheet.band(tr("Source histograms"), true, None, None).open {
            // A constant has no spread: it shows as a line at its value in its side's colour
            let constants = [left, right].map(|side| (side == CompareSource::Constant).then_some(constant));
            let histograms = if constants.contains(&None) { histogram_counts(app, sheet.ui, effect_id) } else { None };
            sheet.embed(|ui| source_histograms(ui, histograms.as_deref(), constants));
        }
    });
}

/// A source's name in its list
fn source_name(source: CompareSource) -> String {
    tr(match source {
        CompareSource::LayersBelow => n_("Layers below"),
        CompareSource::ThisLayer => n_("This layer"),
        CompareSource::ThisMask => n_("This mask"),
        CompareSource::Constant => n_("Constant"),
    })
    .to_string()
}

/// The well with the two sources' histograms (`counts`, left then right; a side reading a constant has `constants` set instead), quarter lines and value labels
fn source_histograms(ui: &mut Ui, counts: Option<&[Vec<u32>; 2]>, constants: [Option<f32>; 2]) {
    let tokens = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HISTOGRAM_HEIGHT + AXIS_LABEL_ROOM), Sense::hover());
    let well = Rect::from_min_size(rect.min, vec2(rect.width(), HISTOGRAM_HEIGHT));
    let painter = ui.painter_at(rect.expand2(vec2(AXIS_LABEL_SIZE, 0.0)));
    painter.rect_filled(well, 0.0, tokens.field);
    let at = |share: f32| well.left() + well.width() * share;
    for quarter in GRID_FRACTIONS {
        painter.line_segment([pos2(at(quarter), well.top()), pos2(at(quarter), well.bottom())], Stroke::new(1.0, tokens.separator));
    }

    // The right source first, the left over it; a constant side draws no spread
    let shown = |side: usize| counts.and_then(|both| both.get(side)).filter(|bins| !bins.is_empty() && constants.get(side).copied().flatten().is_none());
    let tallest = [0, 1].into_iter().filter_map(shown).flatten().copied().max().unwrap_or(0).max(1) as f32;
    for side in [1, 0] {
        if let (Some(tag), Some(bins)) = (SOURCE_TAGS.get(side), shown(side)) {
            painter.add(Shape::mesh(spread_mesh(well, bins, tallest, tag.gamma_multiply(HISTOGRAM_TINT))));
        }
    }
    for (side, constant) in constants.iter().enumerate() {
        if let (Some(value), Some(tag)) = (constant, SOURCE_TAGS.get(side)) {
            let x = at(value.clamp(0.0, 1.0));
            painter.line_segment([pos2(x, well.top() + CONSTANT_LINE_INSET), pos2(x, well.bottom())], Stroke::new(CONSTANT_LINE_WIDTH, *tag));
        }
    }

    for (index, label) in AXIS_LABELS.iter().enumerate() {
        let x = at(index as f32 / (AXIS_LABELS.len() - 1) as f32);
        painter.text(pos2(x, well.bottom() + AXIS_LABEL_OFFSET), Align2::CENTER_CENTER, *label, FontId::proportional(AXIS_LABEL_SIZE), tokens.text_dim);
    }
}

/// One source's spread in `well`: a single `tint` mesh from bucket centre to bucket centre (separate strips would leave seams between them), `tallest` reaching the well's top less its headroom
fn spread_mesh(well: Rect, bins: &[u32], tallest: f32, tint: Color32) -> Mesh {
    let step = well.width() / bins.len() as f32;
    let tops = bins
        .iter()
        .enumerate()
        .map(|(index, count)| (well.left() + (index as f32 + 0.5) * step, well.bottom() - (*count as f32 / tallest) * (HISTOGRAM_HEIGHT - HISTOGRAM_HEADROOM)));
    let outline = std::iter::once((well.left(), well.bottom())).chain(tops).chain(std::iter::once((well.right(), well.bottom())));

    let mut mesh = Mesh::default();
    for (index, (x, top)) in outline.enumerate() {
        mesh.colored_vertex(pos2(x, top), tint);
        mesh.colored_vertex(pos2(x, well.bottom()), tint);
        if let Ok(next) = u32::try_from(index * 2)
            && index > 0
        {
            mesh.add_triangle(next - 2, next - 1, next);
            mesh.add_triangle(next - 1, next + 1, next);
        }
    }
    mesh
}

/// What the two sides of compare mask `effect_id` read
fn histogram_counts(app: &TextureCraftApp, ui: &Ui, effect_id: LayerId) -> Option<Arc<[Vec<u32>; 2]>> {
    let key = egui::Id::unique(("compare_histograms", effect_id.0));
    cached_histogram(ui, "compare_histograms_owner", key, app.session.revision, || app.session.compare_histograms(effect_id, HISTOGRAM_BUCKETS))
}
