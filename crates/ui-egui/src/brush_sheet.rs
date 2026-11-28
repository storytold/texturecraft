//! The Brush section of a paint tool's Properties: the Size, Opacity, Orientation, Jitter and Projection bands, grouped and named as the reference groups them

use std::ops::RangeInclusive;
use std::sync::Arc;

use egui::{Painter, Rect, Response, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;
use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::paint::{Alignment, BrushSettings};

use crate::TextureCraftApp;
use crate::brush_pictures::{alignment_picture, dial_ring, jitter_image, stamp_image};
use crate::brush_stroke_preview::BrushStrokePreview;
use crate::fill_pictures::backface_picture;
use crate::i18n::tr;
use crate::property_sheet::{PropertySheet, RowLead, StackSlider};
use crate::theme::{Tokens, caption};
use crate::toolbar::{MAX_BRUSH_SIZE, MIN_BRUSH_SIZE};

/// Height of the Jitter band's strip of dabs, in points
const JITTER_STRIP_HEIGHT: f32 = 40.0;

/// Corner radius of the Jitter strip's frame
const JITTER_STRIP_ROUNDING: f32 = 2.0;

/// Width of the Jitter strip's frame, in points; the dabs are drawn inside it
const JITTER_STRIP_OUTLINE_WIDTH: f32 = 1.0;

/// What Angle and each Angle fade may be, in degrees
const ANGLE_RANGE: RangeInclusive<f32> = -180.0..=180.0;
const ANGLE_FADE_RANGE: RangeInclusive<f32> = 0.0..=90.0;

/// Size's slider, in percent of the largest brush
const SIZE_PERCENT_RANGE: RangeInclusive<f32> = 0.05..=100.0;

/// Spacing's slider, in percent of the stamp's width
const SPACING_PERCENT_RANGE: RangeInclusive<f32> = 1.0..=1000.0;

/// Position Jitter's slider, in percent of the stamp's radius
const POSITION_JITTER_PERCENT_RANGE: RangeInclusive<f32> = 0.0..=400.0;

const PERCENT_RANGE: RangeInclusive<f32> = 0.0..=100.0;

/// The ranges of Stamp depth, Gravity, particles per stamp, and a particle's lifetime in steps
const STAMP_DEPTH_RANGE: RangeInclusive<f32> = 0.05..=4.0;
const GRAVITY_RANGE: RangeInclusive<f32> = 0.0..=2.0;
const PARTICLE_COUNT_RANGE: RangeInclusive<f32> = 1.0..=16.0;
const PARTICLE_LIFETIME_RANGE: RangeInclusive<f32> = 1.0..=128.0;

/// Width of the brush presets list
const PRESETS_WIDTH: f32 = 230.0;

/// Height the presets list scrolls past
const PRESETS_MAX_HEIGHT: f32 = 380.0;

/// The Brush section under its caption, whose ≡ button `menu` opens the brush presets
pub(crate) fn brush_section(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, menu: Option<Response>) {
    if let Some(menu) = menu {
        brush_presets_popover(app, sheet.ui, menu);
    }

    let tokens = Tokens::get(sheet.ui.ctx());
    let image = app.session.tool.brush.alpha_image.as_ref().and_then(|name| app.session.images.get(name)).cloned();
    let brush = &mut app.session.tool.brush;
    let defaults = BrushSettings::default();

    // Each band's reset puts back only the fields it shows
    let size_fields = |settings: &BrushSettings| (settings.size, settings.size_pressure, settings.min_size, settings.spacing);
    let band = sheet.band(tr("Size"), true, Some(size_fields(brush) != size_fields(&defaults)), None);
    if band.reset {
        (brush.size, brush.size_pressure, brush.min_size, brush.spacing) = size_fields(&defaults);
    }
    if band.open {
        // Every row keeps the pen button's slot, so the three values line up
        let mut size_percent = brush.size / MAX_BRUSH_SIZE * 100.0;
        if sheet.pen_slider(tr("Size"), &mut size_percent, SIZE_PERCENT_RANGE, 1, Some(&mut brush.size_pressure)).0.is_some() {
            brush.size = (size_percent / 100.0 * MAX_BRUSH_SIZE).clamp(MIN_BRUSH_SIZE, MAX_BRUSH_SIZE);
        }
        let pressure = brush.size_pressure;
        sheet.greyed(pressure, Some(tr("Applies only while pen pressure on Size is on")), |sheet| {
            pen_percent_slider(sheet, tr("Minimum Size (%)"), &mut brush.min_size, PERCENT_RANGE);
        });
        pen_percent_slider(sheet, tr("Spacing"), &mut brush.spacing, SPACING_PERCENT_RANGE);
    }

    let opacity_fields = |settings: &BrushSettings| (settings.flow, settings.opacity);
    let band = sheet.band(tr("Opacity"), true, Some(opacity_fields(brush) != opacity_fields(&defaults)), None);
    if band.reset {
        (brush.flow, brush.opacity) = opacity_fields(&defaults);
    }
    if band.open {
        percent_slider(sheet, tr("Flow"), &mut brush.flow, PERCENT_RANGE);
        percent_slider(sheet, tr("Stroke opacity"), &mut brush.opacity, PERCENT_RANGE);
    }

    let orientation_fields = |settings: &BrushSettings| (settings.angle, settings.follow_path);
    let band = sheet.band(tr("Orientation"), true, Some(orientation_fields(brush) != orientation_fields(&defaults)), None);
    if band.reset {
        (brush.angle, brush.follow_path) = orientation_fields(&defaults);
    }
    if band.open {
        let shown = brush.clone();
        let ctx = sheet.ui.ctx().clone();
        let tint = tokens.text.to_array();
        let dial = |painter: &Painter, rect: Rect| {
            BrushStrokePreview::paint_picture(
                &ctx,
                painter,
                rect,
                "brush_stamp_picture",
                (&shown, image.as_ref()),
                |_| [tint[0], tint[1], tint[2]],
                stamp_image,
            );
            dial_ring(painter, rect, shown.angle, &tokens);
        };
        let angle = StackSlider { label: tr("Angle"), value: &mut brush.angle, range: ANGLE_RANGE, typed: ANGLE_RANGE, decimals: 0, suffix: "°", dial: false };
        sheet.picture_sliders(&dial, &mut [angle]);
        sheet.toggle(tr("Follow Path"), &mut brush.follow_path);
    }

    let jitter_fields = |settings: &BrushSettings| (settings.jitter_size, settings.jitter_flow, settings.jitter_angle, settings.jitter_position);
    let band = sheet.band(tr("Jitter"), true, Some(jitter_fields(brush) != jitter_fields(&defaults)), None);
    if band.reset {
        (brush.jitter_size, brush.jitter_flow, brush.jitter_angle, brush.jitter_position) = jitter_fields(&defaults);
    }
    if band.open {
        jitter_strip(sheet, brush, image.as_ref(), &tokens);
        percent_slider(sheet, tr("Size Jitter"), &mut brush.jitter_size, PERCENT_RANGE);
        percent_slider(sheet, tr("Flow Jitter"), &mut brush.jitter_flow, PERCENT_RANGE);
        percent_slider(sheet, tr("Angle Jitter"), &mut brush.jitter_angle, PERCENT_RANGE);
        percent_slider(sheet, tr("Position Jitter"), &mut brush.jitter_position, POSITION_JITTER_PERCENT_RANGE);
    }

    let projection_fields =
        |settings: &BrushSettings| (settings.alignment, settings.backface_culling, settings.angle_fade, settings.screen_space, settings.depth);
    let band = sheet.band(tr("Projection"), true, Some(projection_fields(brush) != projection_fields(&defaults)), None);
    if band.reset {
        (brush.alignment, brush.backface_culling, brush.angle_fade, brush.screen_space, brush.depth) = projection_fields(&defaults);
    }
    if band.open {
        projection_rows(sheet, brush, &tokens);
    }

    let particles = &mut brush.particles;
    let band = sheet.band(tr("Particles"), false, Some(*particles != defaults.particles), None);
    if band.reset {
        *particles = defaults.particles;
    }
    if band.open {
        sheet.info(tr("Paint slides down the surface"));
        sheet.toggle(tr("Particle brush"), &mut particles.enabled);
        whole_slider(sheet, tr("Count per stamp"), &mut particles.count, PARTICLE_COUNT_RANGE);
        whole_slider(sheet, tr("Lifetime (steps)"), &mut particles.lifetime, PARTICLE_LIFETIME_RANGE);
        sheet.slider(tr("Gravity"), &mut particles.gravity, GRAVITY_RANGE, 2);
    }

    *brush = brush.sanitized();
}

/// Alignment, Backface culling with its Angle fade under it, Size Space and Stamp depth
fn projection_rows(sheet: &mut PropertySheet<'_>, brush: &mut BrushSettings, tokens: &Tokens) {
    let alignment_name = |alignment: Alignment| {
        match alignment {
            Alignment::Tangent => tr("Tangent | Wrap"),
            Alignment::Camera => tr("Camera"),
        }
        .to_owned()
    };
    let picture = |painter: &Painter, rect: Rect, alignment: Alignment| alignment_picture(painter, rect, alignment, tokens);
    let (picked, _) = sheet.dropdown(tr("Alignment"), brush.alignment, &[Alignment::Tangent, Alignment::Camera], alignment_name, Some(&picture));
    if let Some(alignment) = picked {
        brush.alignment = alignment;
    }

    let culled = brush.backface_culling;
    let picture = |painter: &Painter, rect: Rect| backface_picture(painter, rect, culled, tokens);
    sheet.picture_toggle(&picture, tr("Backface culling"), &mut brush.backface_culling);

    // Faces only fade towards their edge while backfaces are culled
    sheet.greyed(brush.backface_culling, Some(tr("Fades only while Backface culling is on")), |sheet| {
        sheet.angle_led(RowLead::Child(tr("Angle fade from")), &mut brush.angle_fade.0, ANGLE_FADE_RANGE);
        sheet.angle_led(RowLead::Child(tr("Angle fade to")), &mut brush.angle_fade.1, ANGLE_FADE_RANGE);
    });

    if let Some(space) = sheet.switch(tr("Size Space"), &[tr("Object"), tr("Viewport")], usize::from(brush.screen_space)) {
        brush.screen_space = space == 1;
    }
    sheet.slider(tr("Stamp depth"), &mut brush.depth, STAMP_DEPTH_RANGE, 2);
}

/// The brush's dabs scattered by its jitters, across the panel above the Jitter sliders
fn jitter_strip(sheet: &mut PropertySheet<'_>, brush: &BrushSettings, image: Option<&Arc<ImageRes>>, tokens: &Tokens) {
    let tint = tokens.text.to_array();
    sheet.embed(|ui: &mut Ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), JITTER_STRIP_HEIGHT), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, tr("Jitter preview")));

        let painter = ui.painter();
        painter.rect_filled(rect, JITTER_STRIP_ROUNDING, tokens.field);
        painter.rect_stroke(rect, JITTER_STRIP_ROUNDING, Stroke::new(JITTER_STRIP_OUTLINE_WIDTH, tokens.field_border), StrokeKind::Inside);
        BrushStrokePreview::paint_picture(
            ui.ctx(),
            painter,
            rect.shrink(JITTER_STRIP_OUTLINE_WIDTH),
            "brush_jitter_picture",
            (brush, image),
            |_| [tint[0], tint[1], tint[2]],
            jitter_image,
        );
    });
}

/// A 0..1 setting shown and typed in whole percent
fn percent_slider(sheet: &mut PropertySheet<'_>, label: &str, fraction: &mut f32, percent: RangeInclusive<f32>) {
    let mut shown = *fraction * 100.0;
    if sheet.slider(label, &mut shown, percent, 0).is_some() {
        *fraction = shown / 100.0;
    }
}

/// [`percent_slider`] in a band whose rows keep the pen button's slot
fn pen_percent_slider(sheet: &mut PropertySheet<'_>, label: &str, fraction: &mut f32, percent: RangeInclusive<f32>) {
    let mut shown = *fraction * 100.0;
    if sheet.pen_slider(label, &mut shown, percent, 0, None).0.is_some() {
        *fraction = shown / 100.0;
    }
}

/// A count on a slider of whole numbers
fn whole_slider(sheet: &mut PropertySheet<'_>, label: &str, count: &mut u32, range: RangeInclusive<f32>) {
    let mut shown = *count as f32;
    if sheet.slider(label, &mut shown, range, 0).is_some() {
        *count = shown.round() as u32;
    }
}

/// The brush presets, listed under the Brush caption's ≡ button
fn brush_presets_popover(app: &mut TextureCraftApp, ui: &Ui, menu: Response) {
    menu.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Brush presets")));
    let menu = menu.on_hover_text(tr("Brush presets"));
    let popup_id = ui.make_persistent_id("brush_presets_popup");
    if menu.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }

    egui::Popup::from_response(&menu).id(popup_id).open_memory(None).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(PRESETS_WIDTH);
        egui::ScrollArea::vertical().max_height(PRESETS_MAX_HEIGHT).show(ui, |ui| {
            let mut category = "";
            for preset in texturecraft_engine::paint::presets::PRESETS.iter() {
                if preset.category != category {
                    category = preset.category;
                    ui.label(caption(category));
                }
                if ui.selectable_label(false, preset.label).on_hover_text(tr("Use this brush (keeps its size)")).clicked() {
                    app.run("brush.preset.apply", json!({"preset": preset.id}));
                }
            }

            ui.separator();
            crate::shelf::user_brushes(app, ui, "");
        });
    });
}
