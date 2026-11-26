//! A fill's Symmetry section: mirrored or radial copies of its 3D projection

use std::ops::RangeInclusive;

use egui::{Painter, Rect};
use serde_json::{Value, json};
use texturecraft_engine::core::{FillSymmetry, Projection, Sampling, SymmetryKind};

use crate::TextureCraftApp;
use crate::fill_panel::run_projection;
use crate::fill_pictures::ring_picture;
use crate::i18n::{n_, tr};
use crate::property_sheet::{PropertySheet, StackSlider};
use crate::theme::{AXIS_COLORS, AXIS_NAMES, Tokens};
use crate::widgets::press_edit;

/// What the Axis position track spans, in mesh radii from the mesh's centre
const POSITION_RANGE: RangeInclusive<f32> = -FillSymmetry::REACH..=FillSymmetry::REACH;

/// What the Count track spans; a typed count reaches the engine's fewest and most copies, as the symmetry popup's Count does
const COUNT_TRACK: RangeInclusive<f32> = *FillSymmetry::COUNTS.start() as f32..=16.0;

/// What a typed Count may reach
const COUNT_TYPED: RangeInclusive<f32> = *FillSymmetry::COUNTS.start() as f32..=*FillSymmetry::COUNTS.end() as f32;

/// The Symmetry section under the Fill section: its switch on the caption, then Type, Axis, the radial copies beside their picture, Flip copy and Axis position
pub(crate) fn symmetry_section(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, projection: &Projection, sampling: Sampling, base: &Value) {
    let symmetry = sampling.symmetry;
    let run = |app: &mut TextureCraftApp, fields: Value| run_projection(app, base, fields);

    if !projection.takes_symmetry() {
        let reason = match projection {
            Projection::UvTile => tr("Not available with Fill (Match per UV Tile)"),
            _ => tr("Not available with UV projection"),
        };

        sheet.section_unavailable(tr("Symmetry"), reason);
        return;
    }

    let switch = sheet.section_switch(tr("Symmetry"), symmetry.on);
    if switch.clicked() {
        run(app, json!({"symmetry": !symmetry.on}));
    }
    if !symmetry.on {
        return;
    }

    let (picked, _) = sheet.dropdown(tr("Type"), symmetry.kind, &SymmetryKind::ALL, |kind| tr(kind_label(kind)).to_owned(), None);
    if let Some(kind) = picked {
        run(app, json!({"symmetryType": kind.id()}));
    }

    let axis = symmetry.axis_index();
    if let Some(picked) = sheet.switch_tinted(tr("Axis"), &AXIS_NAMES, axis, &AXIS_COLORS) {
        run(app, json!({"symmetryAxis": picked}));
    }

    if symmetry.kind == SymmetryKind::Radial {
        radial_rows(app, sheet, &symmetry, base);
    }

    let mut flip = symmetry.flip;
    if sheet.toggle_pair(tr("Flip copy"), ["U", "V"], &mut flip).is_some() {
        run(app, json!({"flipCopy": flip}));
    }

    // Mirror position moves along its normal and radial position moves across its axis
    let moving: Vec<usize> = match symmetry.kind {
        SymmetryKind::Mirror => vec![axis],
        SymmetryKind::Radial => (0..AXIS_NAMES.len()).filter(|other| *other != axis).collect(),
    };
    let mut parts: Vec<(&str, egui::Color32, f32)> = moving.iter().map(|&moved| (AXIS_NAMES[moved], AXIS_COLORS[moved], symmetry.position[moved])).collect();
    if let Some(merge) = sheet.lettered_sliders(tr("Axis position"), &mut parts, POSITION_RANGE, 2) {
        let mut position = symmetry.position;
        for (axis, (_, _, value)) in moving.into_iter().zip(parts) {
            position[axis] = value;
        }

        run(app, json!({"axisPosition": position, "merge": merge}));
    }
}

/// Count and Angle span beside the ring of copies, seen along the axis
fn radial_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, symmetry: &FillSymmetry, base: &Value) {
    let tokens = Tokens::get(sheet.ui.ctx());
    let axis_color = AXIS_COLORS[symmetry.axis_index()];
    let picture = |painter: &Painter, rect: Rect| ring_picture(painter, rect, symmetry, axis_color, &tokens);
    let count = symmetry.copies();
    let (mut count_value, mut span_value) = (count as f32, symmetry.span);

    let mut sliders = [
        StackSlider { label: tr("Count"), value: &mut count_value, range: COUNT_TRACK, typed: COUNT_TYPED, decimals: 0, suffix: "", dial: false },
        StackSlider {
            label: tr("Angle span"),
            value: &mut span_value,
            range: FillSymmetry::SPANS,
            typed: FillSymmetry::SPANS,
            decimals: 0,
            suffix: "°",
            dial: false,
        },
    ];

    let [count_edit, span_edit] = sheet.picture_sliders(&picture, &mut sliders);

    // Start the count's undo step on the first whole-number change
    let rounded_count = count_value.round() as u32;
    let press = sheet.ui.make_persistent_id("symmetry_count_press");
    let count_merge = press_edit(sheet.ui, press, count_edit.is_some() && rounded_count != count);
    let count_edit = count_edit.zip(count_merge).map(|(track_merge, count_merge)| track_merge && count_merge);

    if let Some(merge) = [count_edit, span_edit].into_iter().flatten().reduce(|first, second| first && second) {
        let mut fields = json!({"merge": merge});
        if count_edit.is_some() {
            fields["symmetryCount"] = json!(rounded_count);
        }
        if span_edit.is_some() {
            fields["angleSpan"] = json!(span_value);
        }

        run_projection(app, base, fields);
    }
}

/// The Type dropdown's name for `kind`
fn kind_label(kind: SymmetryKind) -> &'static str {
    match kind {
        SymmetryKind::Mirror => n_("Mirror symmetry"),
        SymmetryKind::Radial => n_("Radial symmetry"),
    }
}
