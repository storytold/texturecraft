//! The Alpha section of a paint tool's Properties: the alpha's slot with its picker, its Parameters beside a picture of the shape they make, and the Text band

use std::ops::RangeInclusive;

use egui::{Color32, CornerRadius, Painter, Rect, Stroke, Ui, pos2};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::Session;
use texturecraft_engine::core::Channel;
use texturecraft_engine::paint::alpha_params::MAX_ALPHA_CONTRAST;
use texturecraft_engine::paint::{BrushAlpha, BrushSettings, alphas};

use crate::TextureCraftApp;
use crate::alpha_choice::AlphaChoice;
use crate::brush_pictures::alpha_image;
use crate::brush_stroke_preview::BrushStrokePreview;
use crate::i18n::{n_, tr};
use crate::picker_group::PickerGroup;
use crate::picker_tab::PickerTab;
use crate::property_sheet::{PropertySheet, StackSlider};
use crate::resource_picker;
use crate::resource_picker_item::ResourcePickerItem;
use crate::theme::Tokens;
use crate::tile_picture::TilePicture;

/// Hardness, Roundness and Size X / Y, in percent
const HARDNESS_PERCENT_RANGE: RangeInclusive<f32> = 0.0..=99.9;
const ROUNDNESS_PERCENT_RANGE: RangeInclusive<f32> = 0.0..=100.0;
const ALPHA_SIZE_PERCENT_RANGE: RangeInclusive<f32> = 5.0..=100.0;

/// Rows the text box shows before it scrolls
const TEXT_ROWS: usize = 2;

/// Air inside the text box, around its words
const TEXT_BOX_MARGIN: i8 = 6;

/// Corner radius of the text box, as the sheet's fields
const TEXT_BOX_ROUNDING: u8 = 3;

/// Width of the text box's outline
const TEXT_BOX_OUTLINE_WIDTH: f32 = 1.0;

/// The Alpha section: the alpha's slot, its Parameters band and the Text band
pub(crate) fn alpha_section(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>) {
    alpha_slot(app, sheet);
    parameters_band(app, sheet);

    if sheet.band(tr("Text"), false, None, None).open {
        text_rows(app, sheet);
    }
}

/// The slot showing the alpha in use; a click opens the picker of shapes, library alphas and images, × goes back to the Round shape
fn alpha_slot(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>) {
    let ctx = sheet.ui.ctx().clone();
    let current = current_choice(&app.session);
    let brush = &app.session.tool.brush;
    let hardness = brush.hardness;
    let (name, source, texture) = match &current {
        AlphaChoice::Image(name) => {
            let texture = app.session.images.get(name).map(|image| app.thumbs.image(&ctx, name, image));
            (name.clone(), tr("Imported image"), texture)
        }
        AlphaChoice::Library(id) => {
            let alpha = alphas::find(id);
            let texture = alpha.and_then(|alpha| app.thumbs.alpha(&ctx, alpha));
            (alpha.map_or_else(|| (*id).to_owned(), |alpha| tr(&alpha.label).to_owned()), tr("Library alpha"), texture)
        }
        AlphaChoice::Shape(_) | AlphaChoice::Import => (tr(brush.alpha.label()).to_owned(), tr("Built-in shape"), None),
    };
    let shape = brush.alpha;
    let thumbnail = |painter: &Painter, rect: Rect| {
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::BLACK);
        match &texture {
            Some(texture) => {
                painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => crate::thumbs::paint_alpha(painter, rect, shape, hardness),
        }
    };

    // The default Round shape has nothing to clear back to
    let clearable = current != AlphaChoice::Shape(BrushAlpha::default());
    let slot = sheet.slot(&thumbnail, &name, source, None, clearable, false);

    let popup = sheet.ui.make_persistent_id("brush_alpha_source");
    if slot.pick.clicked() {
        resource_picker::toggle(&ctx, popup);
    }
    let tabs = || alpha_tabs(&app.session, hardness);
    match resource_picker::show(&slot.pick, popup, &mut app.thumbs, Some(&current), true, tabs) {
        Some(AlphaChoice::Import) => crate::menus::invoke(app, &ctx, "resource.import"),
        Some(choice) if choice != current => {
            app.run("tool.brush", json!({ "alpha": alpha_value(&choice) }));
        }
        _ => {}
    }
    if slot.clear {
        app.run("tool.brush", json!({ "alpha": BrushAlpha::default().label() }));
    }

    // An alpha from the Assets panel, or an image, dropped on the slot becomes the alpha
    crate::shelf::drop_target(app, sheet.ui, &slot.pick, crate::shelf::DropZone::Brush);
    if let Some(name) = crate::dock::image_drop(sheet.ui, &slot.pick) {
        app.run("tool.brush", json!({ "alpha": { "image": name } }));
    }
    sheet.close_card();
}

/// The picker's choice the brush's alpha is
fn current_choice(session: &Session) -> AlphaChoice {
    let brush = &session.tool.brush;
    match (&brush.alpha_image, brush.alpha_library.as_deref().and_then(alphas::find)) {
        (Some(name), _) => AlphaChoice::Image(name.clone()),
        (None, Some(alpha)) => AlphaChoice::Library(alpha.id.as_str()),
        (None, None) => AlphaChoice::Shape(brush.alpha),
    }
}

/// What `tool.brush` takes as `alpha` for `choice`
fn alpha_value(choice: &AlphaChoice) -> Value {
    match choice {
        AlphaChoice::Shape(shape) => json!(shape.label()),
        AlphaChoice::Library(id) => json!({ "library": id }),
        AlphaChoice::Image(name) => json!({ "image": name }),
        AlphaChoice::Import => Value::Null,
    }
}

/// The picker's one tab: the built-in Shapes at the brush's `hardness`, the library's Alphas in their shelf groups, then Textures with Import... first
fn alpha_tabs(session: &Session, hardness: f32) -> Vec<PickerTab<AlphaChoice>> {
    let shapes = BrushAlpha::ALL
        .into_iter()
        .map(|shape| ResourcePickerItem {
            id: AlphaChoice::Shape(shape),
            label: tr(shape.label()).to_owned(),
            caption: None,
            picture: TilePicture::Shape(shape, hardness),
        })
        .collect();
    let library = alphas::library()
        .iter()
        .map(|alpha| ResourcePickerItem {
            id: AlphaChoice::Library(alpha.id.as_str()),
            label: tr(&alpha.label).to_owned(),
            caption: Some(tr(alpha.category)),
            picture: TilePicture::Alpha(alpha),
        })
        .collect();
    let textures = ResourcePickerItem::textures(session, AlphaChoice::Import, |name| AlphaChoice::Image(name.to_owned()));
    let groups = [(n_("Shapes"), shapes), (n_("Alphas"), library), (n_("Textures"), textures)]
        .into_iter()
        .map(|(header, items)| PickerGroup { header: Some(tr(header)), items })
        .collect();
    vec![PickerTab { icon: ic::CIRCLE_HALF, name: tr("Alphas"), groups }]
}

/// Hardness and the parameters every alpha shares beside the shape they make; the band's reset puts them all back
fn parameters_band(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>) {
    let tokens = Tokens::get(sheet.ui.ctx());
    let image = app.session.tool.brush.alpha_image.as_ref().and_then(|name| app.session.images.get(name)).cloned();
    let brush = &mut app.session.tool.brush;
    let defaults = BrushSettings::default();
    let fields = |settings: &BrushSettings| (settings.hardness, settings.alpha_params);
    let band = sheet.band(tr("Parameters"), true, Some(fields(brush) != fields(&defaults)), None);
    if band.reset {
        (brush.hardness, brush.alpha_params) = fields(&defaults);
    }
    if !band.open {
        return;
    }

    let shown = brush.clone();
    let ctx = sheet.ui.ctx().clone();
    let tint = tokens.text.to_array();
    let picture = |painter: &Painter, rect: Rect| {
        painter.rect_filled(rect, CornerRadius::ZERO, tokens.field);
        BrushStrokePreview::paint_picture(&ctx, painter, rect, "brush_alpha_picture", (&shown, image.as_ref()), |_| [tint[0], tint[1], tint[2]], alpha_image);
    };

    // Only the procedural Round shape has a roundness
    let round = brush.alpha == BrushAlpha::Round && brush.alpha_image.is_none() && brush.alpha_library.is_none();
    let before = [brush.hardness, brush.alpha_params.roundness, brush.alpha_params.size_x, brush.alpha_params.size_y].map(|fraction| fraction * 100.0);
    let [mut hardness, mut roundness, mut size_x, mut size_y] = before;
    let percent = |label: &'static str, value, range: RangeInclusive<f32>| StackSlider {
        label: tr(label),
        value,
        range: range.clone(),
        typed: range,
        decimals: 0,
        suffix: "",
        dial: false,
    };
    if round {
        sheet.picture_sliders(
            &picture,
            &mut [
                percent(n_("Hardness"), &mut hardness, HARDNESS_PERCENT_RANGE),
                percent(n_("Roundness"), &mut roundness, ROUNDNESS_PERCENT_RANGE),
                percent(n_("Size X"), &mut size_x, ALPHA_SIZE_PERCENT_RANGE),
                percent(n_("Size Y"), &mut size_y, ALPHA_SIZE_PERCENT_RANGE),
            ],
        );
    } else {
        sheet.picture_sliders(
            &picture,
            &mut [
                percent(n_("Hardness"), &mut hardness, HARDNESS_PERCENT_RANGE),
                percent(n_("Size X"), &mut size_x, ALPHA_SIZE_PERCENT_RANGE),
                percent(n_("Size Y"), &mut size_y, ALPHA_SIZE_PERCENT_RANGE),
            ],
        );
    }

    // Only what moved is written back, so an untouched value keeps its exact fraction
    for (after, was, fraction) in [
        (hardness, before[0], &mut brush.hardness),
        (roundness, before[1], &mut brush.alpha_params.roundness),
        (size_x, before[2], &mut brush.alpha_params.size_x),
        (size_y, before[3], &mut brush.alpha_params.size_y),
    ] {
        if after != was {
            *fraction = after / 100.0;
        }
    }
    sheet.slider(tr("Contrast"), &mut brush.alpha_params.contrast, 1.0..=MAX_ALPHA_CONTRAST, 2);
    sheet.toggle(tr("Invert"), &mut brush.alpha_params.invert);
    *brush = brush.sanitized();
}

/// Text drawn in a font, as the brush alpha or a decal (`resource.text`)
fn text_rows(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>) {
    if app.ui.text.fonts.is_empty()
        && let Some(fonts) = app.run("text.fonts", Value::Null)
    {
        let installed = fonts["installed"].as_array().into_iter().flatten().map(|font| {
            let styles = font["styles"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            (font["family"].as_str().unwrap_or_default().to_string(), styles)
        });
        let built_in = fonts["builtIn"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|name| (name.to_string(), Vec::new()));
        app.ui.text.fonts = installed.chain(built_in).collect();
    }

    // A decal shows the text in the tool's base color; an alpha must stay white
    let color = app.session.tool.values.get(&Channel::BaseColor).copied().unwrap_or([1.0; 3]);
    let tokens = Tokens::get(sheet.ui.ctx());
    let text = &mut app.ui.text;
    sheet.embed(|ui: &mut Ui| {
        let frame = egui::Frame::NONE
            .fill(tokens.field)
            .stroke(Stroke::new(TEXT_BOX_OUTLINE_WIDTH, tokens.field_border))
            .corner_radius(CornerRadius::same(TEXT_BOX_ROUNDING))
            .inner_margin(egui::Margin::same(TEXT_BOX_MARGIN));
        frame.show(ui, |ui| {
            let edit =
                egui::TextEdit::multiline(&mut text.text).frame(egui::Frame::NONE).desired_rows(TEXT_ROWS).desired_width(f32::INFINITY).hint_text(tr("Text"));
            ui.add(edit);
        });
    });

    // Index 0 is the default font, then the families in the order `text.fonts` lists them
    let family = text.fonts.iter().position(|(family, _)| *family == text.font).map_or(0, |index| index + 1);
    let family_name = |index: usize| if index == 0 { tr("Default").to_owned() } else { text.fonts[index - 1].0.clone() };
    let (picked, _) = sheet.dropdown_built(tr("Font"), family, || 0..=text.fonts.len(), family_name, None);
    if let Some(index) = picked {
        text.font = if index == 0 { String::new() } else { text.fonts[index - 1].0.clone() };
        text.style = String::new();
    }

    let styles = text.fonts.iter().find(|(family, _)| *family == text.font).map(|(_, styles)| styles.clone()).unwrap_or_default();
    if styles.len() > 1 {
        let style = styles.iter().position(|style| *style == text.style).map_or(0, |index| index + 1);
        let style_name = |index: usize| if index == 0 { tr("Default").to_owned() } else { styles[index - 1].clone() };
        let (picked, _) = sheet.dropdown_built(tr("Style"), style, || 0..=styles.len(), style_name, None);
        if let Some(index) = picked {
            text.style = if index == 0 { String::new() } else { styles[index - 1].clone() };
        }
    }

    let mut params =
        json!({"text": text.text, "font": (!text.font.is_empty()).then_some(&text.font), "style": (!text.style.is_empty()).then_some(&text.style)});
    let ready = !text.text.trim().is_empty();
    let alpha = sheet.button_enabled(ic::CIRCLE_HALF, tr("Use as brush alpha"), false, ready).clicked();
    let decal = sheet.button_enabled(ic::STAMP, tr("Place as decal"), false, ready).clicked();
    if decal {
        params["color"] = json!(color);
    }
    if (alpha || decal)
        && let Some(name) = app.run("resource.text", params).and_then(|result| result["name"].as_str().map(str::to_string))
    {
        if alpha {
            app.run("tool.brush", json!({"alpha": {"image": name}}));
        } else {
            app.run("layer.add.decal", json!({"image": name}));
        }
    }
}
