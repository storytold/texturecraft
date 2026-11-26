//! Panel bodies of the right column: Texture Sets, Layers, Texture Set Settings, Properties
//! (hosted as dock tabs by `docking`).

use egui::{Color32, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::compose::ThumbTarget;
use texturecraft_engine::core::{
    BakedMap, BlendMode, Channel, ChannelBlend, Effect, EffectKind, Layer, LayerId, LayerKind, LinkParam, Projection, Sampling, Source,
};
use texturecraft_engine::paint::alpha_params::MAX_ALPHA_CONTRAST;
use texturecraft_engine::paint::{BrushAlpha, BrushSettings, alphas};
use texturecraft_engine::{CloneFrom, StackTarget, Tool};

use crate::TextureCraftApp;
use crate::i18n::{n_, tr, trf};
use crate::layer_panel::{effect_preview_size, effect_row_height, layer_preview_size, layer_row_height};
use crate::past_end::PastEnd;
use crate::property_sheet::{PropertySheet, SectionScope};
use crate::shelf::{Asset, ShelfDrag};
use crate::source_choice::SourceChoice;
use crate::state::PropsTab;
use crate::theme::{Tokens, caption};
use crate::toolbar::{MAX_BRUSH_SIZE, MIN_BRUSH_SIZE};
use crate::widgets::{chip, color_swatch, combo, dnd_release, elided, icon_button, press_edit, section, section_with_reset, slider, slider_edit};

const TAG_COLORS: [Color32; 8] = [
    Color32::from_rgb(0xc0, 0x40, 0x40),
    Color32::from_rgb(0xd0, 0x80, 0x30),
    Color32::from_rgb(0xc8, 0xb0, 0x30),
    Color32::from_rgb(0x58, 0xa0, 0x40),
    Color32::from_rgb(0x30, 0xa0, 0x90),
    Color32::from_rgb(0x40, 0x78, 0xd0),
    Color32::from_rgb(0x80, 0x58, 0xc8),
    Color32::from_rgb(0xb0, 0x50, 0xa0),
];

pub(crate) fn texture_set_list(app: &mut TextureCraftApp, ui: &mut Ui) {
    // The empty state lists nothing
    if !app.session.project_open {
        return;
    }

    let tk = Tokens::get(ui.ctx());
    let sets: Vec<(String, String, bool)> = app
        .session
        .doc
        .project
        .texture_sets
        .iter()
        .map(|t| {
            let res = t.resolution;
            let size = match t.uv_tiles.len() {
                0 => format!("{res}x{res}"),
                n => format!("{res}x{res}, {n} UDIM"),
            };
            (t.name.clone(), size, t.visible)
        })
        .collect();
    let active = app.session.ts();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.menu_button(RichText::new(ic::EYE).size(14.0), |ui| {
            for (label, mode) in [(tr("Show all"), "showAll"), (tr("Hide all"), "hideAll"), (tr("Invert show/hide"), "invert")] {
                if ui.button(label).clicked() {
                    app.run("textureSet.visibility", json!({ "mode": mode }));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(tr("Show or hide texture sets"));
        let solo = sets.iter().enumerate().all(|(i, s)| s.2 == (i == active));
        if icon_button(ui, ic::FRAME_CORNERS, tr("Focus: show only the active texture set"), solo && sets.len() > 1, 22.0).clicked() {
            app.run("textureSet.visibility", json!({ "mode": "solo" }));
        }
    });
    egui::ScrollArea::vertical().id_salt("ts_scroll").show(ui, |ui| {
        for (i, (name, size, visible)) in sets.into_iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
            let p = ui.painter();
            let bg = if i == active {
                tk.row_selected
            } else if resp.hovered() {
                tk.row_hover
            } else {
                tk.panel
            };
            p.rect_filled(rect, 2.0, bg);
            if i == active {
                p.rect_stroke(rect, 2.0, Stroke::new(1.0, tk.accent), egui::StrokeKind::Inside);
            }
            let eye = Rect::from_min_size(rect.min + vec2(4.0, 3.0), vec2(16.0, 16.0));
            let eye_resp = ui.interact(eye, ui.scope_id().with(("ts_eye", i)), Sense::click()).on_hover_text(tr("Show or hide the texture set"));
            p.text(eye.center(), egui::Align2::CENTER_CENTER, if visible { ic::EYE } else { ic::EYE_SLASH }, egui::FontId::proportional(13.0), tk.text);

            // Narrow panels drop the shader label, then the resolution, before cutting the name
            let cy = rect.center().y;
            let mut right = rect.right() - 6.0;
            if rect.width() >= 260.0 {
                let g = p.layout_no_wrap(tr("Main shader").into(), egui::FontId::proportional(11.0), tk.text_dim);
                right -= g.size().x;
                p.galley(pos2(right, cy - g.size().y * 0.5), g, tk.text_dim);
                right -= 10.0;
            }
            if rect.width() >= 170.0 {
                let g = p.layout_no_wrap(size, egui::FontId::proportional(11.0), tk.text_dim);
                let x = (rect.left() + rect.width() * 0.48).min(right - g.size().x);
                p.galley(pos2(x, cy - g.size().y * 0.5), g, tk.text_dim);
                right = x - 8.0;
            }
            let color = if i == active { tk.text_strong } else { tk.text };
            let g = elided(p, name, 12.0, color, right - rect.left() - 26.0);
            p.galley(pos2(rect.left() + 26.0, cy - g.size().y * 0.5), g, color);
            if eye_resp.clicked() {
                app.run("textureSet.visible", json!({"ts": i}));
            } else if resp.clicked() {
                app.run("textureSet.select", json!({"ts": i}));
            }
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Layers

/// Gap between the layer toolbar and the layer rows
const LAYER_TOOLBAR_GAP: f32 = 2.0;

/// Least height of the drop space under the layer rows, so a drop to the bottom always has a target
const BOTTOM_DROP_MIN_HEIGHT: f32 = 24.0;

pub(crate) fn layers(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());

    // The empty state has no layers: the toolbar stays, greyed
    let project_open = app.session.project_open;
    ui.add_enabled_ui(project_open, |ui| crate::layer_toolbar::show(app, ui));
    if !project_open {
        return;
    }

    ui.add_space(LAYER_TOOLBAR_GAP);
    let set = app.session.ts();
    let stack: Vec<LayerId> =
        app.session.doc.project.texture_sets.get(set).map(|texture_set| texture_set.stack.iter().rev().copied().collect()).unwrap_or_default();
    let now = ui.input(|input| input.time);
    app.layer_panel.begin(&app.session, now);
    egui::ScrollArea::vertical().id_salt("layers_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let mut count = 0;
        for id in stack {
            layer_tree(app, ui, id, 0, &mut count);
        }

        // Empty space keeps the selection and drops below the rows
        let free = ui.available_height();
        let drop_height = if free.is_finite() { free.max(BOTTOM_DROP_MIN_HEIGHT) } else { BOTTOM_DROP_MIN_HEIGHT };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), drop_height), Sense::hover());
        if let Some(dragged) = dnd_release::<u64>(&response) {
            let bottom = app.session.doc.project.texture_sets.get(set).and_then(|texture_set| texture_set.stack.first().copied());
            if let Some(layers) = selection_with(app, LayerId(*dragged)) {
                app.run("layer.move", json!({"layers": layers, "index": 0}));
            } else if bottom.map(|bottom| bottom.0) != Some(*dragged) {
                app.run("layer.move", json!({"layer": *dragged, "index": 0}));
            }
        }
        match dnd_release::<ShelfDrag>(&response).map(|drag| drag.asset.clone()) {
            Some(Asset::Material(material)) => {
                app.run("material.apply", json!({"material": material, "index": 0}));
            }
            Some(Asset::Library(path)) => {
                app.run("material.load", json!({"path": path, "index": 0}));
            }
            _ => {}
        }
        let material = response.dnd_hover_payload::<ShelfDrag>().is_some_and(|drag| drag.asset.is_material());
        if response.dnd_hover_payload::<u64>().is_some() || material {
            ui.painter().line_segment([rect.left_top(), rect.right_top()], Stroke::new(DROP_STROKE, tokens.accent));
        }
    });
    select_all_layers(app, ui);
}

/// Ctrl+A (Cmd+A on macOS) with the pointer over the Layers panel selects every row shown in the
/// active texture set (folder rows and the layers of open folders). A focused text field (a
/// rename, a number field) keeps the key for itself, and the key is left alone over other panels.
fn select_all_layers(app: &mut TextureCraftApp, ui: &Ui) {
    let ctx = ui.ctx();
    if ctx.egui_wants_keyboard_input() || !ui.ui_contains_pointer() {
        return;
    }

    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
        app.run("layer.select", json!({"all": true}));
    }
}

/// A click on a layer row or its content thumbnail, as in the reference app: Shift selects the
/// rows from the last clicked layer to this one, Ctrl/Cmd adds or takes out this layer, and a
/// plain click selects it alone (on `target`, when given).
fn click_layer(app: &mut TextureCraftApp, modifiers: egui::Modifiers, layer: u64, target: Option<&str>) {
    let params = if modifiers.shift {
        json!({"layer": layer, "range": true})
    } else if modifiers.command {
        json!({"layer": layer, "toggle": true})
    } else {
        match target {
            Some(target) => json!({"layer": layer, "target": target}),
            None => json!({"layer": layer}),
        }
    };
    app.run("layer.select", params);
}

/// The selected layers when `layer` is one of several selected ones, so an action on its row
/// (drag and drop, the row menu) applies to all of them.
fn selection_with(app: &TextureCraftApp, layer: LayerId) -> Option<Vec<u64>> {
    let selected = app.session.selected_layers();
    (selected.len() > 1 && selected.contains(&layer)).then(|| selected.iter().map(|id| id.0).collect())
}

/// Whether the selection targets the layer content (the content or one of its effects) rather
/// than the mask.
pub(crate) fn targets_content(app: &TextureCraftApp) -> bool {
    let p = &app.session.doc.project;
    match app.session.target {
        StackTarget::Content => true,
        StackTarget::Mask => false,
        StackTarget::Effect(e) => p.effect(e).and_then(|(owner, _)| p.layers.get(&owner)).is_some_and(|l| l.content_effects.iter().any(|x| x.id == e)),
    }
}

/// Layer rows alternate between two shades so neighbours read as separate items.
fn row_shade(tokens: &Tokens, index: usize) -> Color32 {
    if index.is_multiple_of(2) { tokens.row } else { tokens.row.lerp_to_gamma(tokens.row_hover, ALTERNATE_LAYER_SHADE) }
}

/// Fill of the active layer's row: the panel tinted well towards the accent so the row stands out
/// from its neighbours at any scale, while the lettering on it keeps its contrast.
pub fn selected_layer_fill(tokens: &Tokens) -> Color32 {
    tokens.row_selected.lerp_to_gamma(tokens.accent_dim, SELECTED_LAYER_TINT)
}

/// Glyph colour of a layer's eye and the chip behind it: a visible layer shows a bright eye on an
/// accent chip, a hidden one only the dim crossed-out eye, so the two states differ by more than a glyph.
pub fn layer_eye_colors(tokens: &Tokens, visible: bool) -> (Color32, Option<Color32>) {
    if visible { (tokens.text_strong, Some(tokens.accent_dim)) } else { (tokens.text_dim, None) }
}

/// Width of the blend mode / opacity column at the right of a layer row.
const RIGHT_COLUMN: f32 = 58.0;
/// Indent per folder level, and per level of the effect tree.
const INDENT: f32 = 14.0;
/// Edge of the small icon buttons in the properties panel.
const ICON_BUTTON_SIZE: f32 = 22.0;
/// Height of a blend mode menu or opacity field in a stack row, and the rounding of its hover fill.
const ROW_CONTROL_HEIGHT: f32 = 17.0;
const ROW_CONTROL_ROUNDING: f32 = 2.0;
/// Gap between the controls at the right of a stack row, and between them and the row's edge.
const ROW_CONTROL_GAP: f32 = 4.0;
/// Space before an effect row's eye and after its name.
const ROW_LEFT_PADDING: f32 = 7.0;
const ROW_RIGHT_PADDING: f32 = 6.0;
/// Width of the selected row's accent edge.
const SELECTED_EDGE_WIDTH: f32 = 3.0;
/// The same edge on a layer row, wide enough to read on a dense display.
const SELECTED_LAYER_EDGE_WIDTH: f32 = 4.0;
/// How far the active layer's fill moves from the plain selected shade to the accent tint.
const SELECTED_LAYER_TINT: f32 = 0.3;
/// The chip behind a visible layer's eye: its inset from the eye's hit area and its rounding.
const EYE_CHIP_INSET: f32 = 1.0;
const EYE_CHIP_ROUNDING: f32 = 4.0;
/// An effect thumbnail's corner rounding and border, the strike across a hidden effect's, and the line marking where a drag drops.
const THUMBNAIL_ROUNDING: f32 = 2.0;
const THUMBNAIL_BORDER_WIDTH: f32 = 1.0;
const HIDDEN_STRIKE_WIDTH: f32 = 1.5;
const DROP_LINE_WIDTH: f32 = 2.0;
/// The eye's size and the column it sits in (the eye and a gap before the thumbnail).
const EYE_SIZE: f32 = 16.0;
const EYE_COLUMN_WIDTH: f32 = 22.0;
/// Gap between an effect's thumbnail and its name, and the narrowest name worth showing.
const THUMBNAIL_GAP: f32 = 8.0;
const MIN_NAME_WIDTH: f32 = 20.0;
/// Font sizes in an effect row: the eye, the name, the blend mode and its caret.
const EYE_FONT_SIZE: f32 = 12.0;
const EFFECT_NAME_FONT_SIZE: f32 = 11.5;
/// Height available to an effect name while it is edited.
const EFFECT_NAME_EDIT_HEIGHT: f32 = 20.0;
const ROW_SMALL_FONT_SIZE: f32 = 11.0;
const CARET_FONT_SIZE: f32 = 8.0;
/// Room for the caret at the right of the blend mode's name.
const CARET_WIDTH: f32 = 10.0;
/// The blend mode menu's width, wide enough for the longest mode name.
const BLEND_MENU_MIN_WIDTH: f32 = 190.0;
/// Padding inside a row's opacity field, and the percent it moves per point dragged.
const ROW_FIELD_PADDING: f32 = 4.0;
const OPACITY_DRAG_SPEED: f64 = 0.5;
/// A layer row's eye: its hit area, its glyph size and the column it sits in.
const LAYER_EYE_SIZE: f32 = 18.0;
const LAYER_EYE_FONT_SIZE: f32 = 14.0;
/// The wand on a mask thumbnail that has effects.
const LAYER_WAND_FONT_SIZE: f32 = 13.0;
const LAYER_EYE_COLUMN_WIDTH: f32 = 20.0;
/// A folder's caret: its size (the space it takes before the thumbnail) and its center from where it starts.
const FOLDER_CARET_SIZE: f32 = 14.0;
const FOLDER_CARET_CENTER: f32 = 6.0;
/// A thumbnail's border while it is the layer's target, and how far outside it the mask-view and drop outlines sit.
const TARGET_BORDER_WIDTH: f32 = 1.5;
const HIGHLIGHT_OUTSET: f32 = 2.0;
/// The gap between a layer row's stacked blend mode and opacity, either side of the row's middle.
const ROW_STACK_GAP: f32 = 1.0;
/// Width of a layer's colour tag beside the accent edge.
const TAG_WIDTH: f32 = 3.0;
/// Space between a layer's content and mask thumbnails, and the size of the empty mask slot.
const THUMBNAIL_SPACING: f32 = 4.0;
const MASK_SLOT_SIZE: f32 = 18.0;
/// Grays of a white or black mask before its thumbnail arrives.
const WHITE_MASK_GRAY: u8 = 225;
const BLACK_MASK_GRAY: u8 = 18;
/// A layer's name and subtitle font sizes, and their distance from the row's middle.
const LAYER_NAME_FONT_SIZE: f32 = 12.5;
const LAYER_SUBTITLE_FONT_SIZE: f32 = 10.0;
const NAME_LINE_GAP: f32 = 2.0;
/// Folder lines: their width, how faded they are, how far an indent must reach past one, and where the branch starts.
const FOLDER_LINE_WIDTH: f32 = 1.0;
const FOLDER_LINE_FADE: f32 = 0.6;
const FOLDER_LINE_CLEARANCE: f32 = 8.0;
const FOLDER_BRANCH_OFFSET: f32 = 15.0;
/// The share of a folder row's height, either side of its middle, where a drop goes inside it.
const FOLDER_DROP_ZONE: f32 = 0.25;
/// How far each alternating effect row shade goes from the panel towards the hover colour.
const EFFECT_ROW_SHADES: [f32; 2] = [0.2, 0.5];
/// Width of the blend mode and opacity pair at the right of an effect row (a gap between them), and of its opacity part.
const EFFECT_BLEND_WIDTH: f32 = 82.0;
const EFFECT_OPACITY_WIDTH: f32 = 36.0;
/// Height of the "Content effects" / "Mask effects" header rows.
const GROUP_HEADER_H: f32 = 20.0;

// Group header spacing leaves room for the label and effect count
const GROUP_HEADER_CARET_OFFSET: f32 = 15.0;
const GROUP_HEADER_LABEL_GAP: f32 = 10.0;
const GROUP_HEADER_LABEL_RESERVE: f32 = 28.0;
const GROUP_HEADER_COUNT_PADDING: f32 = 8.0;
const GROUP_HEADER_LABEL_FONT_SIZE: f32 = 10.5;
/// Hover tint of alternating layer rows
const ALTERNATE_LAYER_SHADE: f32 = 0.45;
/// Maximum layer-tree nesting shown by the dock
const MAX_LAYER_TREE_DEPTH: usize = 12;

/// Width of the drop indicators: the line above a row and the outline round it or its mask
const DROP_STROKE: f32 = 2.0;
/// Checkerboard cells along each thumbnail edge
const THUMB_CHECKER_CELLS: usize = 4;
/// Keep the group label and count readable when the panel is narrow
const GROUP_HEADER_MIN_REMAINING_WIDTH: f32 = 90.0;

/// Keep both thumbnail targets inside the row before spending space on indentation
fn layer_thumbnail_layout(width: f32, depth: usize, folder: bool, preferred_size: f32) -> (f32, f32) {
    let controls = ROW_LEFT_PADDING + LAYER_EYE_COLUMN_WIDTH + if folder { FOLDER_CARET_SIZE } else { 0.0 };
    let thumbnail_space = (width - controls - THUMBNAIL_SPACING - ROW_CONTROL_GAP).max(0.0);
    let size = preferred_size.min(thumbnail_space * 0.5);
    let indent = (depth as f32 * INDENT).min((thumbnail_space - size * 2.0).max(0.0));
    (indent, size)
}

/// Drop secondary controls before allowing them to cover thumbnail targets or the name
fn layer_name_layout(rect: Rect, left: f32) -> (Rect, bool) {
    let right = rect.right() - ROW_CONTROL_GAP;
    let show_blend = right - RIGHT_COLUMN - ROW_CONTROL_GAP - left >= MIN_NAME_WIDTH;
    let name_right = if show_blend { right - RIGHT_COLUMN - ROW_CONTROL_GAP } else { right };
    (Rect::from_min_max(pos2(left, rect.top() + 4.0), pos2(name_right.max(left), rect.bottom() - 4.0)), show_blend)
}

/// Whether `text` at font `size` is wider than `width`, so a row shows it shortened.
fn text_is_cut(painter: &egui::Painter, text: &str, size: f32, width: f32) -> bool {
    painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(size), Color32::WHITE).size().x > width
}

/// Preserve the effect eye, preview and a readable name before spending space on nesting.
fn effect_row_indent(width: f32, level: usize, preview: f32) -> f32 {
    (level as f32 * INDENT).min((width - ROW_LEFT_PADDING - EYE_COLUMN_WIDTH - preview - THUMBNAIL_GAP - MIN_NAME_WIDTH - ROW_RIGHT_PADDING).max(0.0))
}

/// X of the centre of the eye column at indent `level` in a row spanning from `left`.
fn level_x(left: f32, level: usize) -> f32 {
    left + ROW_LEFT_PADDING + level as f32 * INDENT + EYE_SIZE * 0.5
}

fn layer_tree(app: &mut TextureCraftApp, ui: &mut Ui, id: LayerId, depth: usize, count: &mut usize) {
    let Some(layer) = app.session.doc.project.layers.get(&id).cloned() else { return };

    let index = *count;
    *count += 1;
    layer_row(app, ui, &layer, depth, index);
    let mask_effects = layer.mask.as_ref().map_or(0, |mask| mask.effects.len());
    if !layer.content_effects.is_empty() {
        effect_group(app, ui, &layer, depth, false, mask_effects > 0);
    }
    if mask_effects > 0 {
        effect_group(app, ui, &layer, depth, true, false);
    }

    // A line under the layer and its effects separates it from the next layer
    let tokens = Tokens::get(ui.ctx());
    let available = ui.available_rect_before_wrap();
    ui.painter().hline(available.x_range(), available.top() - 0.5, Stroke::new(1.0, tokens.separator));
    if let LayerKind::Group { children, collapsed, .. } = &layer.kind
        && !collapsed
        && depth < MAX_LAYER_TREE_DEPTH
    {
        for child in children.iter().rev() {
            layer_tree(app, ui, *child, depth + 1, count);
        }
    }
}

/// The picture for `target` inside `rect` (drawn over the well), whether there is one yet.
fn thumb_box(ui: &Ui, app: &mut TextureCraftApp, rect: Rect, target: ThumbTarget, dim: bool) -> bool {
    let tokens = Tokens::get(ui.ctx());
    let painter = ui.painter();
    painter.rect_filled(rect, THUMBNAIL_ROUNDING, tokens.field);

    // A checkerboard shows through where the layer has nothing (transparent texels).
    let cell_size = rect.width() / THUMB_CHECKER_CELLS as f32;
    let check = tokens.field.lerp_to_gamma(tokens.row_hover, 0.6);
    for y in 0..THUMB_CHECKER_CELLS {
        for x in (0..THUMB_CHECKER_CELLS).filter(|x| (x + y) % 2 == 0) {
            painter.rect_filled(Rect::from_min_size(rect.min + vec2(x as f32 * cell_size, y as f32 * cell_size), vec2(cell_size, cell_size)), 0.0, check);
        }
    }

    // Only rows on screen ask for a picture.
    if ui.is_rect_visible(rect)
        && let Some(texture) = app.layer_panel.texture(ui.ctx(), &app.session, target)
    {
        let tint = if dim { Color32::WHITE.gamma_multiply(0.4) } else { Color32::WHITE };
        painter.image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        return true;
    }

    false
}

/// The row of `layer` at folder depth `depth`, the `index`th row of the stack (rows alternate shades).
fn layer_row(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer, depth: usize, index: usize) {
    let tokens = Tokens::get(ui.ctx());
    let selected = app.session.selected_layer() == Some(layer.id);

    // One of several selected layers (Shift/Ctrl+click) is highlighted like the active one
    let highlighted = selected || app.session.is_layer_selected(layer.id);
    let content_target = selected && targets_content(app);
    let preferred_size = layer_preview_size(app);
    let row_height = layer_row_height(preferred_size);
    let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click_and_drag());
    row.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListItem, true, highlighted, &layer.name));

    let painter = ui.painter().clone();

    // The active layer gets the strong tint; the other selected rows the plain selected shade
    let background = if selected {
        selected_layer_fill(&tokens)
    } else if highlighted {
        tokens.row_selected
    } else if row.hovered() {
        tokens.row_hover
    } else {
        row_shade(&tokens, index)
    };
    painter.rect_filled(rect, 0.0, background);
    let solo = app.session.solo.filter(|solo| solo.layer == layer.id);
    let (indent, thumbnail_size) = layer_thumbnail_layout(rect.width(), depth, layer.is_group(), preferred_size);

    // Folder contents hang off lines down the left without crossing compact row controls
    let line = Stroke::new(FOLDER_LINE_WIDTH, tokens.button_border.gamma_multiply(FOLDER_LINE_FADE));
    for level in 0..depth {
        if level as f32 * INDENT + FOLDER_LINE_CLEARANCE < indent {
            painter.vline(level_x(rect.left(), level), rect.y_range(), line);
        }
    }
    if depth > 0 && indent >= INDENT {
        let x = rect.left() + FOLDER_BRANCH_OFFSET + indent - INDENT;
        painter.hline(x..=x + INDENT * 0.5, rect.center().y, line);
    }
    if highlighted {
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(SELECTED_LAYER_EDGE_WIDTH, row_height)), 0.0, tokens.accent);
    }
    if let Some(tag) = layer.color_tag.and_then(|tag| TAG_COLORS.get(tag as usize)) {
        painter.rect_filled(Rect::from_min_size(rect.min + vec2(SELECTED_LAYER_EDGE_WIDTH, 0.0), vec2(TAG_WIDTH, row_height)), 0.0, *tag);
    }

    // A soloed layer: an outline round the row and a lit eye
    if solo.is_some() {
        painter.rect_stroke(rect.shrink(SOLO_OUTLINE_WIDTH * 0.5), 0.0, Stroke::new(SOLO_OUTLINE_WIDTH, tokens.warning), egui::StrokeKind::Inside);
    }

    // The eye
    let content_left = rect.left() + ROW_LEFT_PADDING + indent;
    let eye = Rect::from_center_size(pos2(content_left + EYE_SIZE * 0.5, rect.center().y), vec2(LAYER_EYE_SIZE, LAYER_EYE_SIZE));
    let eye_response = ui.interact(eye, ui.scope_id().with(("eye", layer.id.0)), Sense::click()).on_hover_ui(|ui| {
        ui.label(tr("Show or hide the layer"));
        ui.label(tr("Alt+click: show only this layer. Alt+Shift+click: without its mask."));
    });
    let (eye_color, eye_chip) = layer_eye_colors(&tokens, layer.visible);
    if let Some(chip) = eye_chip {
        painter.rect_filled(eye.shrink(EYE_CHIP_INSET), EYE_CHIP_ROUNDING, chip);
    }
    painter.text(
        eye.center(),
        egui::Align2::CENTER_CENTER,
        if layer.visible || solo.is_some() { ic::EYE } else { ic::EYE_SLASH },
        egui::FontId::proportional(LAYER_EYE_FONT_SIZE),
        if solo.is_some() { tokens.warning } else { eye_color },
    );

    // The eye shows or hides its own layer only, even on one of several selected rows
    if eye_response.clicked() {
        let modifiers = ui.input(|input| input.modifiers);
        if modifiers.alt {
            toggle_solo(app, layer.id, modifiers.shift);
        } else {
            app.run("layer.visible", json!({"layer": layer.id.0}));
        }
    }

    // A folder's caret
    let mut x = content_left + LAYER_EYE_COLUMN_WIDTH;
    if let LayerKind::Group { collapsed, .. } = layer.kind {
        let caret = Rect::from_center_size(pos2(x + FOLDER_CARET_CENTER, rect.center().y), vec2(FOLDER_CARET_SIZE, FOLDER_CARET_SIZE));
        let caret_response = ui.interact(caret, ui.scope_id().with(("caret", layer.id.0)), Sense::click()).on_hover_text(tr("Expand or collapse the folder"));
        painter.text(
            caret.center(),
            egui::Align2::CENTER_CENTER,
            if collapsed { ic::CARET_RIGHT } else { ic::CARET_DOWN },
            egui::FontId::proportional(ROW_SMALL_FONT_SIZE),
            tokens.text_dim,
        );
        if caret_response.clicked()
            && let Some(LayerKind::Group { collapsed, .. }) = app.session.doc.project.layers.get_mut(&layer.id).map(|edited| &mut edited.kind)
        {
            *collapsed = !*collapsed;
        }

        x += FOLDER_CARET_SIZE;
    }

    // Content thumbnail: the layer's own result in the displayed channel
    let channel = app.ui.layer_channel;
    let thumbnail = Rect::from_min_size(pos2(x, rect.center().y - thumbnail_size * 0.5), vec2(thumbnail_size, thumbnail_size));
    if !thumb_box(ui, app, thumbnail, ThumbTarget::Content { layer: layer.id, channel }, !layer.visible) {
        content_glyph(&painter, &tokens, thumbnail, layer);
    }

    let content_label = tr("Select the layer content");
    let content_response = ui.interact(thumbnail, ui.scope_id().with(("content", layer.id.0)), Sense::click()).on_hover_text(content_label);
    painter.rect_stroke(thumbnail, THUMBNAIL_ROUNDING, target_stroke(&tokens, content_target), egui::StrokeKind::Outside);
    if content_response.clicked() {
        let modifiers = ui.input(|input| input.modifiers);
        click_layer(app, modifiers, layer.id.0, Some("content"));

        if !modifiers.alt && matches!(app.session.view, texturecraft_engine::ViewMode::Mask(_)) {
            app.run("viewport.view", json!({"mode": "material"}));
        }
    }

    thumbnail_context_menu(ui, &content_response, content_label, &layer.name, content_target, |ui| layer_context_menu(app, ui, layer, true));

    x = thumbnail.right() + THUMBNAIL_SPACING;

    // An instance draws its source's content, blending and (unless it has its own) mask
    let project = &app.session.doc.project;
    let source = layer.instance_source().map(|source| {
        let set = project.locate(source).and_then(|(set, _, _)| project.texture_sets.get(set)).map(|set| set.name.as_str()).unwrap_or("?");
        project.layers.get(&source).map_or_else(|| tr("missing source").to_string(), |source_layer| format!("{} · {set}", source_layer.name))
    });

    // Read just the visible mask summary to avoid cloning every row
    let mask =
        project.layers.get(&project.mask_owner(layer.id)).and_then(|owner| owner.mask.as_ref()).map(|mask| (mask.enabled, mask.background, mask.effects.len()));
    let blend = project.content_layer(layer.id).and_then(|content| content.blend(channel));

    // Mask thumbnail, selectable on its own; or the slot that adds one
    let mask_rect;
    if let Some((mut mask_enabled, mask_background, mask_effect_count)) = mask {
        let mask_target = selected && !targets_content(app);
        mask_rect = Rect::from_min_size(pos2(x, rect.center().y - thumbnail_size * 0.5), vec2(thumbnail_size, thumbnail_size));
        let mask_label = tr("Select the mask (Alt+click: show it in the viewport, Shift+click: switch it on or off)");
        let mask_response = ui.interact(mask_rect, ui.scope_id().with(("mask", layer.id.0)), Sense::click()).on_hover_text(mask_label);
        if mask_response.clicked() {
            let modifiers = ui.input(|input| input.modifiers);
            if modifiers.shift && !modifiers.alt {
                if app.run("mask.toggle", json!({"layer": layer.id.0})).is_some() {
                    mask_enabled = !mask_enabled;
                    mask_response.request_focus();
                }
            } else {
                app.run("layer.select", json!({"layer": layer.id.0, "target": "mask"}));

                if modifiers.alt {
                    let showing = app.session.view == texturecraft_engine::ViewMode::Mask(layer.id);
                    let view = if showing { json!({"mode": "material"}) } else { json!({"mode": "mask", "layer": layer.id.0}) };
                    app.run("viewport.view", view);
                }
            }
        }

        if !thumb_box(ui, app, mask_rect, ThumbTarget::Mask { layer: layer.id }, !layer.visible || !mask_enabled) {
            let white = mask_background == texturecraft_engine::core::MaskBackground::White;
            painter.rect_filled(mask_rect, THUMBNAIL_ROUNDING, Color32::from_gray(if white { WHITE_MASK_GRAY } else { BLACK_MASK_GRAY }));
            if mask_effect_count > 0 {
                painter.text(
                    mask_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    ic::MAGIC_WAND,
                    egui::FontId::proportional(LAYER_WAND_FONT_SIZE),
                    tokens.text_dim,
                );
            }
        }
        painter.rect_stroke(mask_rect, THUMBNAIL_ROUNDING, target_stroke(&tokens, mask_target), egui::StrokeKind::Outside);
        if !mask_enabled {
            painter.line_segment([mask_rect.left_top(), mask_rect.right_bottom()], Stroke::new(HIDDEN_STRIKE_WIDTH, tokens.danger));
        }

        thumbnail_context_menu(ui, &mask_response, mask_label, &layer.name, mask_target, |ui| mask_management_menu(app, ui, layer, true));
        ui.ctx().accesskit_node_builder(mask_response.id, |node| node.set_description(if mask_enabled { tr("Mask on") } else { tr("Mask off") }));

        if app.session.view == texturecraft_engine::ViewMode::Mask(layer.id) {
            painter.rect_stroke(
                mask_rect.expand(HIGHLIGHT_OUTSET),
                THUMBNAIL_ROUNDING,
                Stroke::new(TARGET_BORDER_WIDTH, tokens.warning),
                egui::StrokeKind::Outside,
            );
        }
    } else {
        // The empty slot adds a mask to this layer
        let slot_size = thumbnail_size.min(MASK_SLOT_SIZE);
        mask_rect = Rect::from_min_size(pos2(x, rect.center().y - slot_size * 0.5), vec2(slot_size, slot_size));
        let slot = ui.interact(mask_rect, ui.scope_id().with(("mask_slot", layer.id.0)), Sense::click()).on_hover_text(tr("Add mask"));
        let slot_color = if slot.hovered() { tokens.accent } else { tokens.field_border };
        painter.rect_stroke(mask_rect, THUMBNAIL_ROUNDING, Stroke::new(THUMBNAIL_BORDER_WIDTH, slot_color), egui::StrokeKind::Inside);
        if slot.hovered() {
            painter.text(mask_rect.center(), egui::Align2::CENTER_CENTER, ic::PLUS, egui::FontId::proportional(EYE_FONT_SIZE), tokens.text);
        }
        let picker = ui.make_persistent_id(("bitmap_mask_picker", layer.id.0));
        egui::Popup::menu(&slot).show(|ui| crate::layer_toolbar::mask_slot_menu(app, ui, layer, picker));
        crate::layer_toolbar::bitmap_picker(app, &row, picker, layer.id);
    }

    x = mask_rect.right() + THUMBNAIL_GAP;

    // Name (or rename editor) with the quieter subtitle under it
    let (name_rect, show_blend) = layer_name_layout(rect, x);
    let show_name = name_rect.width() >= MIN_NAME_WIDTH;
    if !show_name {
        row.clone().on_hover_text(&layer.name);
        if app.ui.rename.as_ref().is_some_and(|(id, _)| *id == layer.id.0) {
            app.ui.rename = None;
        }
    }
    let renaming = app.ui.rename.as_ref().is_some_and(|(id, _)| *id == layer.id.0);
    if renaming {
        let mut editor_ui = ui.new_child(egui::UiBuilder::new().max_rect(name_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
        if let Some((_, text)) = app.ui.rename.as_mut() {
            // Escape reaches us instead of just unfocusing the field, so it can cancel
            let filter = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, escape: true, ..Default::default() };
            let editor = editor_ui.add(egui::TextEdit::singleline(text).desired_width(name_rect.width()).event_filter(filter));
            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                app.ui.rename = None;
            } else if editor.lost_focus() {
                // Enter or a click elsewhere
                let name = text.trim().to_string();
                app.ui.rename = None;
                if !name.is_empty() && name != layer.name {
                    app.run("layer.rename", json!({"layer": layer.id.0, "name": name}));
                }
            } else if !editor.has_focus() {
                // Only until the field has focus: asking every frame trapped the editor open and blocked every shortcut
                editor.request_focus();
            }
        }
    } else if show_name {
        let name_color = if layer.visible { tokens.text_strong } else { tokens.text_dim };
        let galley = elided(&painter, layer.name.clone(), LAYER_NAME_FONT_SIZE, name_color, name_rect.width());
        painter.galley(pos2(name_rect.left(), rect.center().y - NAME_LINE_GAP - galley.size().y), galley, tokens.text);
        let kind = tr(match &layer.kind {
            LayerKind::Paint => "Paint layer",
            LayerKind::Fill(_) => "Fill layer",
            LayerKind::Group { .. } => "Folder",
            LayerKind::Instance { .. } => "Instance",
        });
        let effects = layer.content_effects.len() + mask.map_or(0, |(_, _, count)| count);
        let subtitle = match (source, effects) {
            (Some(source), _) => format!("{kind} · {} {source}", ic::LINK_SIMPLE),
            (None, 0) => kind.to_string(),
            (None, 1) => format!("{kind} · {}", tr("1 effect")),
            (None, count) => format!("{kind} · {}", trf("{count} effects", &[("count", &count)])),
        };

        // A name or subtitle cut short by a narrow panel is shown in full on hover
        if text_is_cut(&painter, &layer.name, LAYER_NAME_FONT_SIZE, name_rect.width())
            || text_is_cut(&painter, &subtitle, LAYER_SUBTITLE_FONT_SIZE, name_rect.width())
        {
            row.clone().on_hover_text(format!("{}\n{subtitle}", layer.name));
        }
        let subtitle = elided(&painter, subtitle, LAYER_SUBTITLE_FONT_SIZE, tokens.text_dim, name_rect.width());
        painter.galley(pos2(name_rect.left(), rect.center().y + NAME_LINE_GAP), subtitle, tokens.text_dim);
    }

    // Blend mode and opacity of the displayed channel, stacked at the right
    let mode_rect = Rect::from_min_size(
        pos2(rect.right() - RIGHT_COLUMN - ROW_CONTROL_GAP, rect.center().y - ROW_CONTROL_HEIGHT - ROW_STACK_GAP),
        vec2(RIGHT_COLUMN, ROW_CONTROL_HEIGHT),
    );
    let opacity_rect =
        Rect::from_min_size(pos2(rect.right() - RIGHT_COLUMN - ROW_CONTROL_GAP, rect.center().y + ROW_STACK_GAP), vec2(RIGHT_COLUMN, ROW_CONTROL_HEIGHT));
    if let Some(channel_blend) = blend.filter(|_| show_blend) {
        let id = ui.scope_id().with(("blend", layer.id.0));
        let (mode, opacity) = blend_controls(
            ui,
            id,
            (mode_rect, opacity_rect),
            (channel_blend.mode, channel_blend.opacity),
            (tr("Blending mode of the displayed channel"), &layer.name),
        );
        if let Some(mode) = mode {
            app.run("layer.blend", json!({"layer": layer.id.0, "channel": channel.id(), "mode": format!("{mode:?}")}));
        }
        if let Some((opacity, merge)) = opacity {
            app.run("layer.blend", json!({"layer": layer.id.0, "channel": channel.id(), "opacity": opacity, "merge": merge}));
        }
    } else if show_blend {
        painter.text(mode_rect.right_center(), egui::Align2::RIGHT_CENTER, tr("off"), egui::FontId::proportional(ROW_SMALL_FONT_SIZE), tokens.text_dim);
    }

    // Interaction: select, rename, drag & drop, context menu
    let on_control = eye_response.clicked() || content_response.clicked();

    // Shift or Ctrl/Cmd clicks in quick succession pick layers; only a plain double click renames
    let modifiers = ui.input(|input| input.modifiers);
    if row.clicked() && !on_control && !renaming {
        click_layer(app, modifiers, layer.id.0, None);
    }
    if row.double_clicked() && !on_control && !renaming && show_name && !modifiers.shift && !modifiers.command {
        app.ui.rename = Some((layer.id.0, layer.name.clone()));
    }
    if row.drag_started() && !renaming {
        row.dnd_set_drag_payload(layer.id.0);
    }

    // A folder midpoint accepts child drops; a smart mask replaces the current mask
    let inside = layer.is_group()
        && ui.input(|input| input.pointer.hover_pos()).is_some_and(|pointer| (pointer.y - rect.center().y).abs() < row_height * FOLDER_DROP_ZONE);
    let shelf = row.dnd_hover_payload::<ShelfDrag>().map(|drag| drag.asset.clone());

    // Several selected layers move together, so none of them is a place to drop them
    let in_drag = |app: &TextureCraftApp, dragged: u64| {
        dragged == layer.id.0 || (app.session.is_layer_selected(layer.id) && app.session.is_layer_selected(LayerId(dragged)))
    };
    let moving = row.dnd_hover_payload::<u64>().is_some_and(|dragged| !in_drag(app, *dragged));
    let drop_stroke = Stroke::new(DROP_LINE_WIDTH, tokens.accent);
    let shelf_material = shelf.as_ref().is_some_and(Asset::is_material);
    if moving || shelf_material {
        if inside {
            painter.rect_stroke(rect, THUMBNAIL_ROUNDING, drop_stroke, egui::StrokeKind::Inside);
        } else if shelf_material {
            painter.line_segment([rect.left_bottom(), rect.right_bottom()], drop_stroke);
        } else {
            painter.line_segment([rect.left_top(), rect.right_top()], drop_stroke);
        }
    } else if matches!(shelf, Some(Asset::SmartMask(_) | Asset::SavedMask(_))) {
        painter.rect_stroke(mask_rect.expand(HIGHLIGHT_OUTSET), THUMBNAIL_ROUNDING, drop_stroke, egui::StrokeKind::Outside);
    } else if matches!(shelf, Some(Asset::Filter(_))) && !layer.is_group() {
        painter.rect_stroke(rect, THUMBNAIL_ROUNDING, drop_stroke, egui::StrokeKind::Inside);
    }
    if let Some(dragged) = dnd_release::<u64>(&row)
        && !in_drag(app, *dragged)
    {
        match selection_with(app, LayerId(*dragged)) {
            Some(layers) => app.run("layer.move", json!({"layers": layers, "above": layer.id.0, "inside": inside})),
            None => app.run("layer.move", json!({"layer": *dragged, "above": layer.id.0, "inside": inside})),
        };
    }
    match dnd_release::<ShelfDrag>(&row).map(|drag| drag.asset.clone()) {
        Some(Asset::Material(material)) => {
            app.run("material.apply", shelf_drop_place(json!({"material": material}), layer.id, inside));
        }
        Some(Asset::Library(path)) => {
            app.run("material.load", shelf_drop_place(json!({"path": path}), layer.id, inside));
        }
        Some(Asset::SmartMask(smart_mask)) => {
            app.run("mask.smart.apply", json!({"name": smart_mask, "layer": layer.id.0}));
        }
        Some(Asset::SavedMask(path)) => {
            app.run("mask.smart.apply", json!({"path": path, "layer": layer.id.0}));
        }
        Some(Asset::Filter(filter)) => {
            app.run("layer.select", json!({"layer": layer.id.0}));
            app.run("effect.add.filter", json!({"layer": layer.id.0, "filter": filter, "target": "content"}));
        }
        Some(Asset::Alpha(_) | Asset::Texture(_)) | None => {}
    }
    row.context_menu(|ui| layer_context_menu(app, ui, layer, false));
}

/// Expose the thumbnail as a focusable control and use the same popup for pointer and keyboard.
fn thumbnail_context_menu(ui: &Ui, response: &egui::Response, label: &str, layer_name: &str, selected: bool, contents: impl FnOnce(&mut Ui)) {
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, format!("{label} ({layer_name})")));
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_has_popup(egui::accesskit::HasPopup::Menu));
    let mut popup = egui::Popup::context_menu(response).anchor(response);
    let return_focus = popup.is_open() && ui.input(|input| input.key_pressed(egui::Key::Escape));
    if response.has_focus() {
        ui.painter().rect_stroke(response.rect.shrink(1.0), THUMBNAIL_ROUNDING, ui.visuals().widgets.active.fg_stroke, egui::StrokeKind::Inside);
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::F10) || input.consume_key(egui::Modifiers::NONE, egui::Key::Menu)) {
            popup = popup.open_memory(Some(egui::SetOpenCommand::Bool(true)));
        }
    }
    popup.show(contents);
    if return_focus {
        response.request_focus();
    }
}

/// `params` of a shelf material dropped on the row of `layer`: inside a folder when dropped on its middle, else directly below the row
fn shelf_drop_place(mut params: serde_json::Value, layer: LayerId, inside: bool) -> serde_json::Value {
    params[if inside { "above" } else { "below" }] = json!(layer.0);
    if inside {
        params["inside"] = json!(true);
    }
    params
}

/// The border of a content or mask thumbnail: the accent while it is the layer's target.
fn target_stroke(tokens: &Tokens, target: bool) -> Stroke {
    if target { Stroke::new(TARGET_BORDER_WIDTH, tokens.accent) } else { Stroke::new(THUMBNAIL_BORDER_WIDTH, tokens.button_border) }
}

/// What a content thumbnail shows until its picture arrives (or where the layer has none).
fn content_glyph(p: &egui::Painter, tk: &Tokens, thumb: Rect, layer: &Layer) {
    match &layer.kind {
        LayerKind::Fill(f) => {
            let c = match f.source(Channel::BaseColor) {
                Source::Uniform(v) => v,
                Source::Procedural { low, high, .. } => [(low[0] + high[0]) * 0.5, (low[1] + high[1]) * 0.5, (low[2] + high[2]) * 0.5],
                Source::Image { .. } | Source::Anchor { .. } => [0.5; 3],
            };
            crate::thumbs::paint_sphere(p, thumb.center(), thumb.width() * 0.34, c);
        }
        LayerKind::Paint => {
            p.text(thumb.center(), egui::Align2::CENTER_CENTER, ic::PAINT_BRUSH, egui::FontId::proportional(thumb.width() * 0.45), tk.text_dim);
        }
        LayerKind::Group { .. } => {
            p.text(thumb.center(), egui::Align2::CENTER_CENTER, ic::FOLDER_SIMPLE, egui::FontId::proportional(thumb.width() * 0.5), tk.text);
        }
        LayerKind::Instance { .. } => {
            p.text(thumb.center(), egui::Align2::CENTER_CENTER, ic::LINK, egui::FontId::proportional(thumb.width() * 0.45), tk.accent);
        }
    }
}

/// The "Content effects" or "Mask effects" header of `layer` and, unless folded, its effect rows
/// hung off tree lines. `more_below`: a second group follows, so the tree line goes on.
fn effect_group(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer, depth: usize, mask: bool, more_below: bool) {
    let tokens = Tokens::get(ui.ctx());
    let stack: &[Effect] = if mask { layer.mask.as_ref().map_or(&[], |layer_mask| &layer_mask.effects) } else { &layer.content_effects };
    let key = (layer.id.0, mask);
    let folded = app.layer_panel.folded.contains(&key);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), GROUP_HEADER_H), Sense::click());
    let label = if mask { tr("Mask effects") } else { tr("Content effects") };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 0.0, if response.hovered() { tokens.row_hover } else { tokens.panel });
    let indent = ((depth + 1) as f32 * INDENT).min((rect.width() - GROUP_HEADER_MIN_REMAINING_WIDTH).max(0.0));
    tree_lines(&painter, &tokens, rect, indent, &[], more_below);
    let x = rect.left() + GROUP_HEADER_CARET_OFFSET + indent;
    painter.text(
        pos2(x, rect.center().y),
        egui::Align2::CENTER_CENTER,
        if folded { ic::CARET_RIGHT } else { ic::CARET_DOWN },
        egui::FontId::proportional(LAYER_SUBTITLE_FONT_SIZE),
        tokens.text_dim,
    );
    let galley = elided(
        &painter,
        label.to_string(),
        GROUP_HEADER_LABEL_FONT_SIZE,
        tokens.text_dim,
        (rect.right() - GROUP_HEADER_LABEL_RESERVE - x - GROUP_HEADER_LABEL_GAP).max(0.0),
    );
    painter.galley(pos2(x + GROUP_HEADER_LABEL_GAP, rect.center().y - galley.size().y * 0.5), galley, tokens.text_dim);
    painter.text(
        pos2(rect.right() - GROUP_HEADER_COUNT_PADDING, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        stack.len().to_string(),
        egui::FontId::proportional(LAYER_SUBTITLE_FONT_SIZE),
        tokens.text_dim,
    );
    if response.on_hover_text(tr("Fold or unfold these effects")).clicked() && !app.layer_panel.folded.remove(&key) {
        app.layer_panel.folded.insert(key);
    }
    if folded {
        return;
    }

    let trunks: Vec<usize> = if more_below { vec![depth] } else { Vec::new() };
    for (row_index, effect) in stack.iter().rev().enumerate() {
        effect_row(app, ui, layer, stack, effect, row_index, depth + 2, &trunks, row_index + 1 == stack.len(), mask);
    }
}

/// Tree connector of a row at the clamped `indent`: a line down from the parent's column (to the middle
/// of the row when `more` is false, the row's end otherwise) and a tick towards the row, with
/// lines straight through for the `trunks` of ancestors that continue.
fn tree_lines(p: &egui::Painter, tk: &Tokens, rect: Rect, indent: f32, trunks: &[usize], more: bool) {
    if indent < INDENT {
        return;
    }
    let stroke = Stroke::new(1.0, tk.button_border);
    for t in trunks {
        if *t as f32 * INDENT + 8.0 < indent {
            p.vline(level_x(rect.left(), *t), rect.y_range(), stroke);
        }
    }
    let x = rect.left() + 15.0 + indent - INDENT;
    let end = if more { rect.bottom() } else { rect.center().y };
    p.vline(x, rect.top()..=end, stroke);
    p.hline(x..=x + INDENT * 0.6, rect.center().y, stroke);
}

/// The drag-and-drop payload of an effect row (the effect id).
struct EffectDrag(u64);

/// A stack row's blend mode menu and opacity field (percent, named after row `row_name`): the mode picked, and the opacity (0..1) set this frame with whether it continues a drag.
fn blend_controls(
    ui: &mut Ui,
    id: egui::Id,
    (mode_rect, opacity_rect): (Rect, Rect),
    (mode, opacity): (BlendMode, f32),
    (hover, row_name): (&str, &str),
) -> (Option<BlendMode>, Option<(f32, bool)>) {
    let tokens = Tokens::get(ui.ctx());
    let painter = ui.painter().clone();

    let mut picked = None;
    let menu = ui.interact(mode_rect, id, Sense::click()).on_hover_text(hover);
    if menu.hovered() {
        painter.rect_filled(mode_rect, ROW_CONTROL_ROUNDING, tokens.row_hover);
    }
    let mode_font = egui::FontId::proportional(ROW_SMALL_FONT_SIZE);
    painter.text(mode_rect.right_center() - vec2(CARET_WIDTH, 0.0), egui::Align2::RIGHT_CENTER, mode.short(), mode_font, tokens.text);
    painter.text(mode_rect.right_center(), egui::Align2::RIGHT_CENTER, ic::CARET_DOWN, egui::FontId::proportional(CARET_FONT_SIZE), tokens.text_dim);
    egui::Popup::menu(&menu).show(|ui| {
        ui.set_min_width(BLEND_MENU_MIN_WIDTH);
        for item in BlendMode::MENU {
            match item {
                Some(item) => {
                    if ui.selectable_label(item == mode, tr(item.label())).clicked() {
                        picked = Some(item);
                        ui.close();
                    }
                }
                None => {
                    ui.separator();
                }
            }
        }
    });

    let mut percent = (opacity * 100.0).round();
    let mut field_ui = ui.new_child(egui::UiBuilder::new().max_rect(opacity_rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
    // As tall as its slot in the row (the style's fields are taller), in the row's small print
    field_ui.spacing_mut().interact_size.y = opacity_rect.height();
    field_ui.spacing_mut().button_padding = vec2(ROW_FIELD_PADDING, 0.0);
    field_ui.style_mut().override_text_style = Some(egui::TextStyle::Small);

    // Plain text like the blend mode beside it: no field box, only the hover fill
    let widgets = &mut field_ui.visuals_mut().widgets;
    for state in [&mut widgets.inactive, &mut widgets.noninteractive] {
        state.weak_bg_fill = Color32::TRANSPARENT;
        state.bg_fill = Color32::TRANSPARENT;
        state.bg_stroke = Stroke::NONE;
    }
    for state in [&mut widgets.hovered, &mut widgets.active] {
        state.weak_bg_fill = tokens.row_hover;
        state.bg_stroke = Stroke::NONE;
    }
    let field = field_ui.add(egui::DragValue::new(&mut percent).range(0.0..=100.0).speed(OPACITY_DRAG_SPEED).max_decimals(0).suffix("%"));
    field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::SpinButton, true, trf("Opacity of {name}", &[("name", &row_name)])));

    // A drag's first change starts an undo step and its later changes continue it (a drag can start without a change)
    let drag_changed_id = id.with("opacity_drag_changed");
    let drag_changed = !field.drag_started() && ui.data(|data| data.get_temp::<bool>(drag_changed_id)).unwrap_or(false);
    let merge = field.dragged() && drag_changed;
    if field.changed() || field.drag_started() {
        ui.data_mut(|data| data.insert_temp(drag_changed_id, field.changed() && field.dragged()));
    }
    (picked, field.changed().then_some((percent / 100.0, merge)))
}

/// Effect `effect` of `stack` (a layer's content or, with `in_mask`, mask effects) at indent `level`.
#[allow(clippy::too_many_arguments)]
fn effect_row(
    app: &mut TextureCraftApp,
    ui: &mut Ui,
    layer: &Layer,
    stack: &[Effect],
    effect: &Effect,
    row_index: usize,
    level: usize,
    trunks: &[usize],
    last: bool,
    in_mask: bool,
) {
    let tokens = Tokens::get(ui.ctx());
    let (effect_id, visible) = (effect.id, effect.visible);
    let selected = app.session.selected_layer() == Some(layer.id) && app.session.target == StackTarget::Effect(effect_id);
    let layer_preview = layer_preview_size(app);
    let preview_size = effect_preview_size(layer_preview);
    let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), effect_row_height(layer_preview)), Sense::click_and_drag());
    row.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListItem, true, selected, &effect.name));

    // Background, tree lines and the selected row's accent edge
    let painter = ui.painter().clone();

    // Effect rows alternate two shades just above the panel, counted from the top of their list
    let background = if selected {
        tokens.row_selected
    } else if row.hovered() {
        tokens.row_hover
    } else {
        tokens.panel.lerp_to_gamma(tokens.row_hover, EFFECT_ROW_SHADES[row_index % EFFECT_ROW_SHADES.len()])
    };
    painter.rect_filled(rect, 0.0, background);
    let indent = effect_row_indent(rect.width(), level, preview_size);
    tree_lines(&painter, &tokens, rect, indent, trunks, !last);
    if selected {
        painter.rect_filled(Rect::from_min_size(rect.min, vec2(SELECTED_EDGE_WIDTH, rect.height())), 0.0, tokens.accent);
    }

    // The eye, then the stack as it is up to and including this effect (a hidden one shows what it bypasses)
    let content_left = rect.left() + ROW_LEFT_PADDING + indent;
    let eye = Rect::from_center_size(pos2(content_left + EYE_SIZE * 0.5, rect.center().y), vec2(EYE_SIZE, EYE_SIZE));
    let eye_response = ui.interact(eye, ui.scope_id().with(("eff_eye", effect_id.0)), Sense::click()).on_hover_text(tr("Enable or disable the effect"));
    let eye_glyph = if visible { ic::EYE } else { ic::EYE_SLASH };
    painter.text(eye.center(), egui::Align2::CENTER_CENTER, eye_glyph, egui::FontId::proportional(EYE_FONT_SIZE), tokens.text_dim);
    let thumbnail = Rect::from_min_size(pos2(content_left + EYE_COLUMN_WIDTH, rect.center().y - preview_size * 0.5), vec2(preview_size, preview_size));
    let channel = app.ui.layer_channel;

    // Content rows edit the displayed channel's blend
    let row_channel = (!in_mask && !app.ui.layer_material_view).then_some(channel);
    if !thumb_box(ui, app, thumbnail, ThumbTarget::Stage { effect: effect_id, channel }, false) {
        let glyph = if in_mask { ic::MAGIC_WAND } else { ic::SPHERE };
        painter.text(thumbnail.center(), egui::Align2::CENTER_CENTER, glyph, egui::FontId::proportional(preview_size * 0.5), tokens.text_dim);
    }
    painter.rect_stroke(thumbnail, THUMBNAIL_ROUNDING, Stroke::new(THUMBNAIL_BORDER_WIDTH, tokens.button_border), egui::StrokeKind::Outside);
    if !visible {
        painter.line_segment([thumbnail.left_bottom(), thumbnail.right_top()], Stroke::new(HIDDEN_STRIKE_WIDTH, tokens.text_dim));
    }

    // The name, then the blend mode and opacity side by side at the right when the name keeps room to be read
    let name_left = thumbnail.right() + THUMBNAIL_GAP;
    let controls_left = rect.right() - ROW_CONTROL_GAP - EFFECT_BLEND_WIDTH;
    let show_blend = controls_left - ROW_CONTROL_GAP - name_left >= MIN_NAME_WIDTH;
    let name_right = if show_blend { controls_left - ROW_CONTROL_GAP } else { rect.right() - ROW_RIGHT_PADDING };
    let name_color = if visible { tokens.text } else { tokens.text_dim };
    let renaming = app.ui.rename.as_ref().is_some_and(|(id, _)| *id == effect_id.0);
    if renaming {
        let half_height = EFFECT_NAME_EDIT_HEIGHT * 0.5;
        let name_rect = Rect::from_min_max(pos2(name_left, rect.center().y - half_height), pos2(name_right, rect.center().y + half_height));
        effect_rename_editor(app, ui, name_rect, effect);
    } else {
        let name_width = (name_right - name_left).max(MIN_NAME_WIDTH);
        if text_is_cut(&painter, &effect.name, EFFECT_NAME_FONT_SIZE, name_width) {
            row.clone().on_hover_text(&effect.name);
        }
        let galley = elided(&painter, effect.name.clone(), EFFECT_NAME_FONT_SIZE, name_color, name_width);
        painter.galley(pos2(name_left, rect.center().y - galley.size().y * 0.5), galley, tokens.text);
    }
    if show_blend {
        let mode_width = EFFECT_BLEND_WIDTH - ROW_CONTROL_GAP - EFFECT_OPACITY_WIDTH;
        let mode_rect = Rect::from_center_size(pos2(controls_left + mode_width * 0.5, rect.center().y), vec2(mode_width, ROW_CONTROL_HEIGHT));
        let opacity_center = pos2(mode_rect.right() + ROW_CONTROL_GAP + EFFECT_OPACITY_WIDTH * 0.5, rect.center().y);
        let opacity_rect = Rect::from_center_size(opacity_center, vec2(EFFECT_OPACITY_WIDTH, ROW_CONTROL_HEIGHT));
        let id = ui.scope_id().with(("eff_blend", effect_id.0));
        let shown = effect.blend_of(row_channel);
        let tip = if row_channel.is_some() { tr("Blending mode of the displayed channel") } else { tr("Blending mode of the effect") };
        let (mode, opacity) = blend_controls(ui, id, (mode_rect, opacity_rect), (shown.mode, shown.opacity), (tip, &effect.name));
        let channel_id = row_channel.map(Channel::id);
        if let Some(mode) = mode {
            app.run("effect.set", json!({"effect": effect_id.0, "channel": channel_id, "blend": format!("{mode:?}")}));
        }
        if let Some((opacity, merge)) = opacity {
            app.run("effect.set", json!({"effect": effect_id.0, "channel": channel_id, "opacity": opacity, "merge": merge}));
        }
    }

    if eye_response.clicked() {
        app.run("effect.set", json!({"effect": effect_id.0, "visible": !visible}));
    } else if row.double_clicked() && !renaming {
        app.ui.rename = Some((effect_id.0, effect.name.clone()));
    } else if row.clicked() {
        app.run("layer.select", json!({"layer": layer.id.0, "target": effect_id.0}));
    }

    // Drag to reorder within the stack: a drop lands above this row, or below it from its lower half
    if row.drag_started() {
        row.dnd_set_drag_payload(EffectDrag(effect_id.0));
    }
    let position = |id: u64| stack.iter().position(|other| other.id.0 == id);
    let below = ui.input(|input| input.pointer.hover_pos()).is_some_and(|pointer| pointer.y > rect.center().y);
    if row.dnd_hover_payload::<EffectDrag>().is_some_and(|dragged| dragged.0 != effect_id.0 && position(dragged.0).is_some()) {
        let y = if below { rect.bottom() } else { rect.top() };
        painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(DROP_LINE_WIDTH, tokens.accent));
    }
    if let Some(dragged) = dnd_release::<EffectDrag>(&row)
        && let (Some(from), Some(to)) = (position(dragged.0), position(effect_id.0))
        && from != to
    {
        // Rows list the stack top first, so above this row is the next index up
        let index = match (from < to, below) {
            (true, false) | (false, true) => to,
            (true, true) => to - 1,
            (false, false) => to + 1,
        };
        app.run("effect.move", json!({"effect": dragged.0, "index": index}));
    }

    row.context_menu(|ui| {
        if ui.button(tr("Rename effect")).clicked() {
            app.ui.rename = Some((effect_id.0, effect.name.clone()));
            ui.close();
        }
        let keymap = app.ui.keymap.clone();

        // The row is selected before the paste runs, so a copied effect is all it needs
        let paste = texturecraft_engine::effect_clipboard::clipboard_effect(&app.session).is_some();

        // The clipboard items show the layer clipboard's shortcuts, which act on a selected effect
        for (label, command, keys, enabled) in [
            (tr("Copy effect"), "effect.copy", Some("layer.copy"), true),
            (tr("Cut effect"), "effect.cut", Some("layer.cut"), true),
            (tr("Paste effect"), "effect.paste", Some("layer.paste"), paste),
            (tr("Duplicate effect"), "effect.duplicate", None, true),
            (tr("Remove effect"), "effect.remove", None, true),
        ] {
            let mut button = egui::Button::new(label);
            if let Some(shortcut) = keys.and_then(|key| crate::keymap::shortcut_of(&keymap, key)) {
                button = button.shortcut_text(shortcut);
            }
            if ui.add_enabled(enabled, button).clicked() {
                // Selected first: a paste lands above it, a duplicate takes the selection
                app.run("layer.select", json!({"layer": layer.id.0, "target": effect_id.0}));
                app.run(command, json!({"effect": effect_id.0}));
                ui.close();
            }
        }

        // Blending options can apply across channels or effects
        ui.separator();
        let blends = effect.kind.blends();
        let copied = app.session.blending_clipboard.is_some();
        ui.add_enabled_ui(blends, |ui| {
            ui.menu_button(tr("Blending options"), |ui| {
                let channel = row_channel.map(Channel::id);
                let mut item = |ui: &mut Ui, label: &str, command: &str, enabled: bool| {
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        app.run("layer.select", json!({"layer": layer.id.0, "target": effect_id.0}));
                        app.run(command, json!({"effect": effect_id.0, "channel": channel}));
                        ui.close();
                    }
                };

                // Mask and Material views have no individual channel to propagate
                let lends = channel.is_some();
                item(ui, tr("Use this blend mode on every channel"), "effect.blending.modeToEveryChannel", lends);
                item(ui, tr("Use this opacity on every channel"), "effect.blending.opacityToEveryChannel", lends);
                item(ui, tr("Use both on every channel"), "effect.blending.bothToEveryChannel", lends);
                ui.separator();
                item(ui, tr("Copy channel blending"), "effect.blending.copy", true);
                item(ui, tr("Paste channel blending"), "effect.blending.paste", copied);
            });
        });
    });
}

/// The name editor of `effect` in `rect`: Enter or a click elsewhere renames, Escape cancels.
fn effect_rename_editor(app: &mut TextureCraftApp, ui: &mut Ui, rect: Rect, effect: &Effect) {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let Some((_, name_buffer)) = app.ui.rename.as_mut() else { return };

    // Escape reaches us instead of just unfocusing the field, so it can cancel
    let filter = egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, escape: true, ..Default::default() };
    let response = child
        .add(egui::TextEdit::singleline(name_buffer).desired_width(rect.width()).font(egui::FontId::proportional(EFFECT_NAME_FONT_SIZE)).event_filter(filter));
    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        app.ui.rename = None;
    } else if response.lost_focus() {
        let name = name_buffer.trim().to_string();
        app.ui.rename = None;
        if !name.is_empty() && name != effect.name {
            app.run("effect.set", json!({"effect": effect.id.0, "name": name}));
        }
    } else if !response.has_focus() {
        // Only until the field has focus (see the layer rename editor)
        response.request_focus();
    }
}

/// The Geometry mask tool: what a click does, and the selected layer's parts.
fn geometry_tool_section(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    ui.label(caption(tr("Geometry mask")));
    ui.label(RichText::new(tr("Click a mesh part to show or hide the selected layer on it.")).color(tk.text_dim));
    let Some(layer) = app.session.selected_layer().and_then(|id| app.session.doc.project.layers.get(&id).cloned()) else { return };
    let ts = app.session.doc.project.locate(layer.id).map_or(app.session.ts(), |(ts, _, _)| ts);
    let parts = texturecraft_engine::set_cmds::parts(&app.session, ts);
    for part in &parts {
        let on = layer.geometry_mask.as_ref().is_none_or(|m| m.contains(part));
        ui.label(RichText::new(format!("{} {part}", if on { ic::EYE } else { ic::EYE_SLASH })).color(if on { tk.text } else { tk.text_dim }));
    }
    if ui.add_enabled(layer.geometry_mask.is_some(), egui::Button::new(tr("Show on all parts"))).clicked() {
        app.run("layer.set", json!({"layer": layer.id.0, "geometryMask": null}));
    }
}

/// Geometry mask: the mesh parts of the layer's texture set it shows on (when the set has
/// several parts).
fn geometry_mask_menu(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer) {
    let ts = app.session.doc.project.locate(layer.id).map_or(app.session.ts(), |(ts, _, _)| ts);
    let parts = texturecraft_engine::set_cmds::parts(&app.session, ts);
    if parts.len() < 2 {
        return;
    }
    let lid = layer.id.0;
    let shown = layer.geometry_mask.clone().unwrap_or_else(|| parts.clone());
    ui.menu_button(tr("Geometry mask"), |ui| {
        for part in &parts {
            let mut on = shown.contains(part);
            if ui.checkbox(&mut on, part.as_str()).changed() {
                let next: Vec<&String> = parts.iter().filter(|q| if *q == part { on } else { shown.contains(q) }).collect();
                let value = if next.len() == parts.len() { Value::Null } else { json!(next) };
                app.run("layer.set", json!({"layer": lid, "geometryMask": value}));
            }
        }
        ui.separator();
        if ui.add_enabled(layer.geometry_mask.is_some(), egui::Button::new(tr("Show on all parts"))).clicked() {
            app.run("layer.set", json!({"layer": lid, "geometryMask": null}));
            ui.close();
        }
    });
}

/// Ask for a file and write `layer`'s evaluated mask there as a 16-bit grayscale PNG (one file per
/// UV tile for a tiled set).
#[cfg(not(target_arch = "wasm32"))]
fn export_mask_file(app: &mut TextureCraftApp, layer: &Layer) {
    use crate::file_dialogs::{Bound, FileDialog};
    let lid = layer.id.0;
    let name = format!("{}_mask.png", texturecraft_engine::formats::export::sanitize_file_stem(&layer.name));
    app.ask_file(FileDialog::save().filter(tr("PNG image"), &["png"]).file_name(name), Bound::Project, move |app, paths| {
        let Some(path) = paths.first() else { return };
        let Some(done) = app.run("mask.export", json!({"layer": lid, "path": path.display().to_string()})) else { return };
        let files = done.get("files").and_then(Value::as_array).map(|files| files.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "));
        app.toast(trf("Saved {path}", &[("path", &files.unwrap_or_default())]), false);
    });
}

/// Put the evaluated mask of layer `id` on the system clipboard as an 8-bit grayscale image (the
/// whole texture set; a UV-tiled set copies its tile atlas).
fn copy_mask(app: &mut TextureCraftApp, ctx: &egui::Context, id: texturecraft_engine::core::LayerId) {
    match app.session.mask_bitmaps(id, false) {
        Ok(bitmaps) => {
            let Some(bitmap) = bitmaps.first() else { return };
            ctx.copy_image(egui::ColorImage::from_gray([bitmap.width as usize, bitmap.height as usize], &bitmap.gray8()));
            app.toast(tr("Mask copied to the clipboard"), false);
        }
        Err(e) => app.toast(e.to_string(), true),
    }
}

/// Width of the outline round a soloed layer's row
const SOLO_OUTLINE_WIDTH: f32 = 2.0;

/// Show `layer` alone (its mask ignored with `ignore_mask`), or end the solo when it is the one shown.
fn toggle_solo(app: &mut TextureCraftApp, layer: texturecraft_engine::core::LayerId, ignore_mask: bool) {
    if app.session.solo.is_some_and(|solo| solo.layer == layer) {
        app.run("view.solo", json!({"layer": null}));
    } else {
        app.run("view.solo", json!({"layer": layer.0, "ignoreMask": ignore_mask}));
    }
}

/// The context menu's solo entries: a view-only preview, so they never select the layer or edit the document.
fn solo_menu(app: &mut TextureCraftApp, ui: &mut Ui, layer: texturecraft_engine::core::LayerId) {
    let solo = app.session.solo.filter(|solo| solo.layer == layer);
    if ui.add(egui::Button::new(tr("Solo layer")).selected(solo.is_some())).clicked() {
        toggle_solo(app, layer, false);
        ui.close();
    }
    let ignoring = solo.is_some_and(|solo| solo.ignore_mask);
    if ui.add(egui::Button::new(tr("Solo layer without its mask")).selected(ignoring)).clicked() {
        app.run("view.solo", json!({"layer": layer.0, "ignoreMask": !ignoring}));
        ui.close();
    }
}

/// Layer menu ▸ Blending options: the displayed channel's mode and/or opacity for every channel of the layer, and copy / paste of all channels' blending.
fn blending_options_menu(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer) {
    let lid = layer.id.0;
    let project = &app.session.doc.project;
    let content = project.content_layer(layer.id).unwrap_or(layer);
    let shown = content.blend(app.ui.layer_channel);
    let copied: Vec<(Channel, ChannelBlend)> = content.channels.iter().map(|(c, b)| (*c, *b)).collect();
    let can_paste = app.ui.blend_clipboard.is_some() && !content.channels.is_empty();
    ui.menu_button(tr("Blending options"), |ui| {
        let apply = |app: &mut TextureCraftApp, ui: &mut Ui, label: &str, mode: bool, opacity: bool| {
            let Some(shown) = shown else {
                ui.add_enabled(false, egui::Button::new(label));
                return;
            };

            if ui.button(label).clicked() {
                let mut params = json!({"layer": lid, "channel": "all"});
                if mode {
                    params["mode"] = json!(format!("{:?}", shown.mode));
                }
                if opacity {
                    params["opacity"] = json!(shown.opacity);
                }
                app.run("layer.blend", params);
                ui.close();
            }
        };
        apply(app, ui, tr("Apply blending mode to all channels"), true, false);
        apply(app, ui, tr("Apply opacity to all channels"), false, true);
        apply(app, ui, tr("Apply both to all channels"), true, true);
        ui.separator();
        if ui.button(tr("Copy channel blending settings")).clicked() {
            app.ui.blend_clipboard = Some(copied);
            ui.close();
        }
        if ui.add_enabled(can_paste, egui::Button::new(tr("Paste channel blending settings"))).clicked() {
            if let Some(saved) = app.ui.blend_clipboard.clone() {
                let channels: serde_json::Map<String, Value> =
                    saved.iter().map(|(c, b)| (c.id().to_string(), json!({"mode": format!("{:?}", b.mode), "opacity": b.opacity}))).collect();
                app.run("layer.blending.set", json!({"layer": lid, "channels": channels}));
            }
            ui.close();
        }
    });
}
/// Least width of a layer row's right-click menu, wide enough for its longest item and shortcut
const CONTEXT_MENU_MIN_WIDTH: f32 = 230.0;

/// Side and corner radius of a colour tag square in the layer row's right-click menu
const TAG_SWATCH_SIZE: f32 = 16.0;
const TAG_SWATCH_RADIUS: f32 = 2.0;

fn layer_context_menu(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer, content_thumbnail: bool) {
    let tokens = Tokens::get(ui.ctx());
    ui.set_min_width(CONTEXT_MENU_MIN_WIDTH);
    let layer_id = layer.id.0;
    let keymap = app.ui.keymap.clone();

    // The row menu can act on several selected layers. A thumbnail identifies one layer.
    let several = if content_thumbnail { None } else { selection_with(app, layer.id) };
    let run = |app: &mut TextureCraftApp, ui: &mut Ui, label: &str, id: &str, params: Value, enabled: bool| {
        let mut button = egui::Button::new(label);
        if let Some(shortcut) = crate::keymap::shortcut_of(&keymap, id) {
            button = button.shortcut_text(shortcut);
        }
        if ui.add_enabled(enabled, button).clicked() {
            let batch = matches!(id, "layer.remove" | "layer.duplicate" | "layer.group");
            match several.as_ref().filter(|_| batch) {
                Some(layers) => app.run(id, json!({"layers": layers})),
                None => {
                    app.run("layer.select", json!({"layer": layer_id}));
                    app.run(id, params)
                }
            };
            ui.close();
        }
    };

    run(app, ui, tr("Add white mask"), "mask.add.white", json!({"layer": layer_id}), layer.mask.is_none());
    run(app, ui, tr("Add black mask"), "mask.add.black", json!({"layer": layer_id}), layer.mask.is_none());
    run(app, ui, tr("Add mask with color selection"), "mask.add.colorSelection", json!({"layer": layer_id}), true);
    if !content_thumbnail {
        ui.separator();
        mask_management_menu(app, ui, layer, false);
    }
    geometry_mask_menu(app, ui, layer);

    ui.separator();
    solo_menu(app, ui, layer.id);
    ui.separator();
    run(app, ui, tr("Flatten layer"), "layer.flatten", json!({"layer": layer_id}), !layer.is_group());
    #[cfg(not(target_arch = "wasm32"))]
    if ui.button(tr("Create smart material")).clicked() {
        let file = crate::library_dir().join(format!("{}.tcmat", texturecraft_engine::formats::sanitize_resource_name(&layer.name)));
        if let Some(saved) = app.run("material.save", json!({"layer": layer_id, "path": file.display().to_string(), "name": layer.name})) {
            let path = saved.get("path").and_then(|path| path.as_str()).unwrap_or_default().to_string();
            app.toast(trf("Smart material saved to {path}", &[("path", &path)]), false);
            app.library = None;
        }
        ui.close();
    }
    ui.separator();
    run(app, ui, tr("Cut layer(s)"), "layer.cut", json!({"layer": layer_id}), true);
    run(app, ui, tr("Copy layer(s)"), "layer.copy", json!({"layer": layer_id}), true);
    let paste_layer = crate::menus::enabled(app, "layer.paste");
    run(app, ui, tr("Paste layer(s)"), "layer.paste", json!({}), paste_layer);
    let has_content_stack = app.session.doc.project.content_layer(layer.id).is_some_and(|owner| !owner.is_group());
    let (params, paste_effect) = if content_thumbnail {
        let compatible =
            app.session.clipboard.as_ref().and_then(|(doc, id)| doc.project.effect(*id)).is_some_and(|(_, effect)| effect.kind.content_only() != Some(false));
        (json!({"layer": layer_id, "target": "content"}), has_content_stack && compatible)
    } else {
        (json!({}), texturecraft_engine::effect_clipboard::clipboard_effect(&app.session).is_some())
    };
    run(app, ui, tr("Paste effect"), "effect.paste", params, paste_effect);
    let paste_instance = crate::menus::enabled(app, "layer.pasteInstance");
    run(app, ui, tr("Paste as instance"), "layer.pasteInstance", json!({}), paste_instance);
    run(app, ui, tr("Duplicate layer(s)"), "layer.duplicate", json!({"layer": layer_id}), true);
    let many_sets = app.session.set_count() > 1;
    run(app, ui, tr("Instantiate across texture sets"), "layer.instantiateAll", json!({"layer": layer_id}), many_sets);
    run(app, ui, tr("Copy to all texture sets"), "layer.instantiateAll", json!({"layer": layer_id, "copy": true}), many_sets);
    run(app, ui, tr("Remove layer(s)"), "layer.remove", json!({"layer": layer_id}), true);
    run(app, ui, tr("Group layer(s)"), "layer.group", json!({"layer": layer_id}), true);
    blending_options_menu(app, ui, layer);

    // Colour tags: none (crossed out), then each tag colour
    ui.separator();
    ui.horizontal(|ui| {
        let swatch_size = vec2(TAG_SWATCH_SIZE, TAG_SWATCH_SIZE);
        let (swatch, response) = ui.allocate_exact_size(swatch_size, Sense::click());
        ui.painter().rect_stroke(swatch, TAG_SWATCH_RADIUS, Stroke::new(1.0, tokens.text_dim), egui::StrokeKind::Inside);
        ui.painter().line_segment([swatch.left_bottom(), swatch.right_top()], Stroke::new(1.0, tokens.danger));
        if response.clicked() {
            app.run("layer.colorTag", json!({"layer": layer_id}));
        }
        for (tag, color) in TAG_COLORS.iter().enumerate() {
            let (swatch, response) = ui.allocate_exact_size(swatch_size, Sense::click());
            ui.painter().rect_filled(swatch, TAG_SWATCH_RADIUS, *color);
            if response.clicked() {
                app.run("layer.colorTag", json!({"layer": layer_id, "tag": tag}));
            }
        }
    });
    if !content_thumbnail || has_content_stack {
        ui.separator();
        crate::layer_toolbar::effect_menu_items(app, ui, layer, content_thumbnail);
    }
}

/// Mask management is available from the row and directly from the mask thumbnail.
fn mask_management_menu(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer, mask_thumbnail: bool) {
    // An instance inherits its source mask until it has its own.
    let project = &app.session.doc.project;
    let mask = project.layers.get(&project.mask_owner(layer.id)).and_then(|owner| owner.mask.as_ref());
    let (has_mask, mask_on) = (mask.is_some(), mask.is_some_and(|mask| mask.enabled));
    let layer_id = layer.id.0;
    let target = if mask_thumbnail { "mask" } else { "content" };
    for (label, command) in [
        (tr("Toggle mask"), "mask.toggle"),
        (tr("Invert mask background"), "mask.invertBackground"),
        (tr("Invert mask"), "mask.invert"),
        (tr("Clear mask"), "mask.clear"),
        (tr("Remove mask"), "mask.remove"),
    ] {
        let mut button = egui::Button::new(label).selected(command == "mask.toggle" && mask_on);
        if let Some(shortcut) = crate::keymap::shortcut_of(&app.ui.keymap, command) {
            button = button.shortcut_text(shortcut);
        }
        if ui.add_enabled(has_mask, button).clicked() {
            app.run("layer.select", json!({"layer": layer_id, "target": target}));
            app.run(command, json!({"layer": layer_id}));
            ui.close();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if ui.add_enabled(has_mask, egui::Button::new(tr("Create smart mask"))).clicked() {
        app.run("layer.select", json!({"layer": layer_id, "target": target}));
        crate::smart_mask_menu::save(app, layer, &crate::library_dir());
        ui.close();
    }
    #[cfg(not(target_arch = "wasm32"))]
    if ui.add_enabled(mask_on, egui::Button::new(tr("Export mask to file..."))).clicked() {
        export_mask_file(app, layer);
        ui.close();
    }
    if ui.add_enabled(mask_on, egui::Button::new(tr("Copy mask to clipboard"))).clicked() {
        copy_mask(app, ui.ctx(), layer.id);
        ui.close();
    }
}

// ---------------------------------------------------------------------------------------------
// Texture set settings

/// Size of a channel's remove button, and of the room Base color keeps for one
const CHANNEL_REMOVE_SIZE: f32 = 18.0;

/// The sizes a texture set and each of its UV tiles can take
const TEXTURE_SET_RESOLUTIONS: [u32; 6] = [256, 512, 1024, 2048, 4096, 8192];

/// Air between the General properties grid's columns and rows
const GENERAL_GRID_SPACING: egui::Vec2 = vec2(8.0, 6.0);

/// Air above the Channels and Mesh maps captions
const SECTION_GAP: f32 = 8.0;

/// Air between the channel list and the height and normal settings
const CHANNEL_SETTINGS_GAP: f32 = 6.0;

/// Range of the Height to normal strength slider
const HEIGHT_STRENGTH_RANGE: std::ops::RangeInclusive<f32> = 0.0..=10.0;

/// A mesh map row's colour square, its corner radius, the size of its file name and of its menu's dots
const MESH_MAP_SWATCH_SIZE: f32 = 26.0;
const MESH_MAP_SWATCH_RADIUS: f32 = 2.0;
const MESH_MAP_FILE_TEXT_SIZE: f32 = 10.5;
const MESH_MAP_MENU_ICON_SIZE: f32 = 13.0;

pub(crate) fn texture_set_settings(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    let set_index = app.session.ts();
    let Some(set) = app.session.doc.project.texture_sets.get(set_index).cloned() else { return };
    egui::ScrollArea::vertical().id_salt("tss").show(ui, |ui| {
        ui.label(caption(tr("General properties")));
        egui::Grid::new("tss_grid").num_columns(2).spacing(GENERAL_GRID_SPACING).show(ui, |ui| {
            ui.label(tr("Name"));
            ui.label(RichText::new(&set.name).color(tokens.text_strong));
            ui.end_row();

            ui.label(tr("Size"));
            egui::ComboBox::from_id_salt("ts_size").selected_text(format!("{}", set.resolution)).show_ui(ui, |ui| {
                for resolution in TEXTURE_SET_RESOLUTIONS {
                    if ui.selectable_label(resolution == set.resolution, format!("{resolution}")).clicked() {
                        app.start_resize(set_index, resolution);
                    }
                }
            });
            ui.end_row();

            ui.label(tr("UV tiles (UDIM)"));
            let mut tiled = !set.uv_tiles.is_empty();
            if ui.checkbox(&mut tiled, "").changed() {
                app.run("textureSet.uvTiles", json!({"ts": set_index, "enabled": tiled}));
            }
            ui.end_row();

            // Each tile's own size
            for &udim in &set.uv_tiles {
                let tile_resolution = set.tile_resolution(udim);
                ui.label(RichText::new(format!("   {udim}")).color(tokens.text_dim));
                egui::ComboBox::from_id_salt(("ts_tile", udim)).selected_text(format!("{tile_resolution}")).show_ui(ui, |ui| {
                    for resolution in TEXTURE_SET_RESOLUTIONS {
                        if ui.selectable_label(resolution == tile_resolution, format!("{resolution}")).clicked() {
                            app.run("textureSet.tile.resolution", json!({"ts": set_index, "tile": udim, "resolution": resolution}));
                        }
                    }
                });
                ui.end_row();
            }

            ui.label(tr("Shader instance"));
            ui.label(tr("Main shader"));
            ui.end_row();
        });

        ui.add_space(SECTION_GAP);
        ui.horizontal(|ui| {
            ui.label(caption(tr("Channels")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(tr("Add channel"), |ui| {
                    for channel in Channel::ALL {
                        if !set.has_channel(channel) && ui.button(tr(channel.label())).clicked() {
                            app.run("textureSet.channel.add", json!({"ts": set_index, "channel": channel.id()}));
                            ui.close();
                        }
                    }
                });
            });
        });

        for (channel, format) in &set.channels {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr(channel.label())).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Base color can't be removed: its × room stays, so every format ends on one column
                    if *channel == Channel::BaseColor {
                        ui.allocate_exact_size(vec2(CHANNEL_REMOVE_SIZE, CHANNEL_REMOVE_SIZE), Sense::hover());
                    } else if icon_button(ui, ic::X, tr("Remove channel"), false, CHANNEL_REMOVE_SIZE).clicked() {
                        app.run("textureSet.channel.remove", json!({"ts": set_index, "channel": channel.id()}));
                    }
                    ui.label(RichText::new(format.label()).color(tokens.text_dim));
                });
            });
        }

        ui.add_space(CHANNEL_SETTINGS_GAP);
        let mut height_strength = set.height_strength;
        if let Some(merge) = slider_edit(ui, tr("Height to normal strength"), &mut height_strength, HEIGHT_STRENGTH_RANGE, 2) {
            app.run("textureSet.settings", json!({"ts": set_index, "heightStrength": height_strength, "merge": merge}));
        }

        let directx = app.session.doc.project.normal_format == texturecraft_engine::core::NormalFormat::DirectX;
        combo(ui, tr("Normal map format"), if directx { "DirectX" } else { "OpenGL" }, |ui| {
            for (id, label, on) in [("openGl", "OpenGL", !directx), ("directX", "DirectX", directx)] {
                if ui.selectable_label(on, label).clicked() && !on {
                    app.run("textureSet.settings", json!({"ts": set_index, "normalFormat": id}));
                }
            }
        })
        .on_hover_text(tr(
            "For the whole project: how normal map images set as mesh maps are read (set them again after a change), and the default for export",
        ));

        ui.add_space(SECTION_GAP);
        ui.horizontal(|ui| {
            ui.label(caption(tr("Mesh maps")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tr("Bake Mesh Maps")).clicked() {
                    app.ui.mode = crate::state::Mode::Baking;
                }
            });
        });
        for map in BakedMap::ALL {
            let present = app.session.doc.baked.get(&set_index).is_some_and(|baked| baked.maps.contains_key(&map));
            let row = ui.horizontal(|ui| {
                let (swatch, _) = ui.allocate_exact_size(vec2(MESH_MAP_SWATCH_SIZE, MESH_MAP_SWATCH_SIZE), Sense::hover());
                ui.painter().rect_filled(swatch, MESH_MAP_SWATCH_RADIUS, if present { mesh_map_color(map) } else { tokens.button });
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr(crate::i18n::baked_map_label(map))).strong());
                    ui.label(
                        RichText::new(if present { format!("{}_{}", set.name, map.id()) } else { tr("Missing (bake to create)").into() })
                            .size(MESH_MAP_FILE_TEXT_SIZE)
                            .color(tokens.text_dim),
                    );
                });

                // An image resource instead of a bake, or no map
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new(ic::DOTS_THREE).size(MESH_MAP_MENU_ICON_SIZE), |ui| {
                        if let Some(name) = image_menu(app, ui) {
                            app.run("textureSet.meshMap.set", json!({"ts": set_index, "map": map.id(), "image": name}));
                        }
                        ui.separator();
                        if ui.add_enabled(present, egui::Button::new(tr("Clear"))).clicked() {
                            app.run("textureSet.meshMap.clear", json!({"ts": set_index, "map": map.id()}));
                            ui.close();
                        }
                    });
                });
            });

            // A shelf image dropped on the row becomes this mesh map
            let response = row.response.interact(Sense::hover()).on_hover_text(tr("Drop an image from the Assets panel here to use it as this mesh map"));
            if let Some(name) = image_drop(ui, &response) {
                app.run("textureSet.meshMap.set", json!({"ts": set_index, "map": map.id(), "image": name}));
            }
        }
    });
}

/// Menu listing the decoded image resources with a name filter and thumbnails, then Import...: the
/// image picked, if any. A resource that did not decode is not offered, as the commands that take
/// an image refuse it.
pub(crate) fn image_menu(app: &mut TextureCraftApp, ui: &mut Ui) -> Option<String> {
    const LIST_HEIGHT_POINTS: f32 = 240.0;
    const THUMBNAIL_SIZE_POINTS: f32 = 24.0;

    ui.label(caption(tr("Images")));

    // Keep the name filter across menu frames
    let search_id = ui.make_persistent_id("image_search");
    let mut search_text: String = ui.data(|data| data.get_temp(search_id)).unwrap_or_default();
    if !app.session.images.is_empty() {
        ui.add(egui::TextEdit::singleline(&mut search_text).hint_text(tr("Search")).desired_width(f32::INFINITY));
    }
    let search_query = search_text.trim().to_lowercase();
    ui.data_mut(|data| data.insert_temp(search_id, search_text));

    // Release the resource-list borrow before drawing rows that update the preview cache
    let matching_images: Vec<String> = app
        .session
        .doc
        .resources
        .keys()
        .filter(|name| app.session.images.contains_key(name.as_str()) && (search_query.is_empty() || name.to_lowercase().contains(&search_query)))
        .cloned()
        .collect();

    let mut picked = None;

    // Bounded so Import... and later items stay on screen with many images
    egui::ScrollArea::vertical().id_salt("image_menu").max_height(LIST_HEIGHT_POINTS).show(ui, |ui| {
        for name in &matching_images {
            ui.horizontal(|ui| {
                let (rect, thumbnail_response) = ui.allocate_exact_size(egui::Vec2::splat(THUMBNAIL_SIZE_POINTS), egui::Sense::click());

                // Offscreen rows keep their labels without resampling or uploading a preview
                if ui.is_rect_visible(rect)
                    && let Some(image) = app.session.images.get(name)
                {
                    let thumbnail = app.thumbs.image(ui.ctx(), name, image);
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    ui.painter().image(thumbnail.id(), rect, uv, egui::Color32::WHITE);
                }

                // The thumbnail and name select the same image
                let thumbnail_response = thumbnail_response.on_hover_cursor(egui::CursorIcon::PointingHand);
                if thumbnail_response.clicked() || ui.button(name.as_str()).clicked() {
                    picked = Some(name.clone());
                }
            });
        }
    });

    if ui.button(tr("Import...")).clicked() {
        crate::menus::invoke(app, ui.ctx(), "resource.import");
        ui.close();
    }
    if picked.is_some() {
        ui.close();
    }

    picked
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod image_menu_tests {
    use std::sync::Arc;

    use egui_kittest::{Harness, kittest::Queryable};
    use texturecraft_engine::compose::ImageRes;

    use super::{TextureCraftApp, image_menu};

    fn image_menu_harness(image_count: usize) -> texturecraft_engine::Result<Harness<'static, (TextureCraftApp, Option<String>)>> {
        let mut session = texturecraft_engine::sample_session(128, 16)?;

        // The menu reads decoded images; fixture bytes are never imported or decoded
        for index in 0..image_count {
            let name = format!("image-{index:04}.png");
            let image = ImageRes::new(32, 32, vec![[index as f32 / image_count as f32, 0.5, 0.25, 1.0]; 32 * 32], false);
            session.doc.resources.insert(name.clone(), Arc::new(vec![0]));
            session.images.insert(name, Arc::new(image));
        }

        let creation_context = eframe::CreationContext::_new_kittest(egui::Context::default());
        let app = TextureCraftApp::new(&creation_context, session);

        // Translated menu labels may need fonts this preview-cache test does not load
        Ok(Harness::builder().with_size(egui::vec2(320.0, 400.0)).with_context_setup(crate::theme::install).allow_missing_glyphs().build_ui_state(
            |ui, (app, picked)| {
                if let Some(name) = image_menu(app, ui) {
                    *picked = Some(name);
                }
            },
            (app, None),
        ))
    }

    fn preview_stats(context: &egui::Context) -> (usize, usize) {
        context
            .tex_manager()
            .read()
            .allocated()
            .filter(|(_, metadata)| metadata.name.starts_with("res_image-"))
            .fold((0, 0), |(count, bytes), (_, metadata)| (count + 1, bytes + metadata.bytes_used()))
    }

    #[test]
    fn image_menu_previews_only_visible_rows_and_keeps_later_images_selectable() {
        // The harness draws the UI, and the i18n tests switch the process-wide language to ones its fonts cannot draw
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        const IMAGE_COUNT: usize = 128;
        let mut harness = image_menu_harness(IMAGE_COUNT).unwrap();
        let (initial_previews, _) = preview_stats(&harness.ctx);

        assert!(initial_previews > 0);
        assert!(initial_previews < IMAGE_COUNT, "offscreen images must not be resampled: {initial_previews}");

        let initial_ids: Vec<_> =
            harness.ctx.tex_manager().read().allocated().filter(|(_, metadata)| metadata.name.starts_with("res_image-")).map(|(id, _)| *id).collect();
        harness.run_steps(2);

        assert_eq!(preview_stats(&harness.ctx).0, initial_previews);
        assert!(initial_ids.iter().all(|id| harness.ctx.tex_manager().read().meta(*id).is_some()), "warm frames reuse the same previews");

        let last_name = format!("image-{:04}.png", IMAGE_COUNT - 1);
        harness.get_by_label(&last_name).scroll_to_me();
        harness.run_steps(30);

        assert!(preview_stats(&harness.ctx).0 > initial_previews, "scrolling creates newly visible previews");

        harness.get_by_label(&last_name).click();
        harness.run_steps(2);

        assert_eq!(harness.state().1.as_deref(), Some(last_name.as_str()));
    }
}

/// An image from the Assets panel dropped on `resp`, outlined while one hovers it; other assets are left for the drop targets around it
pub(crate) fn image_drop(ui: &Ui, resp: &egui::Response) -> Option<String> {
    if !resp.dnd_hover_payload::<ShelfDrag>().is_some_and(|drag| matches!(drag.asset, Asset::Texture(_))) {
        return None;
    }

    ui.painter().rect_stroke(resp.rect.shrink(1.0), 3.0, Stroke::new(2.0, Tokens::get(ui.ctx()).accent), egui::StrokeKind::Inside);
    match dnd_release::<ShelfDrag>(resp).map(|d| d.asset.clone()) {
        Some(Asset::Texture(name)) => Some(name),
        _ => None,
    }
}

pub(crate) fn mesh_map_color(map: BakedMap) -> Color32 {
    match map {
        BakedMap::Normal => Color32::from_rgb(128, 128, 255),
        BakedMap::WorldNormal => Color32::from_rgb(120, 200, 140),
        BakedMap::Id => Color32::from_rgb(220, 60, 60),
        BakedMap::AmbientOcclusion => Color32::from_gray(200),
        BakedMap::Curvature => Color32::from_gray(128),
        BakedMap::Position => Color32::from_rgb(120, 180, 90),
        BakedMap::Thickness => Color32::from_gray(160),
        BakedMap::Height => Color32::from_gray(128),
        BakedMap::BentNormal => Color32::from_rgb(140, 170, 230),
        BakedMap::Opacity => Color32::from_gray(230),
    }
}

// ---------------------------------------------------------------------------------------------
// Properties

/// The Properties tab title: what the panel edits
pub(crate) fn properties_title(app: &TextureCraftApp) -> String {
    if !app.session.project_open {
        return tr("Properties").to_string();
    }

    let project = &app.session.doc.project;
    let tool = app.session.tool.tool;
    let tool_usable = app.session.tool_usable(tool);
    let what = match (app.session.selected_layer().and_then(|id| project.layers.get(&id)), app.session.target) {
        // Polygon fill keeps its own properties wherever it can fill
        _ if tool == Tool::PolygonFill && tool_usable => tr(tool.label()).to_string(),

        // Filters share a title even when the effect is renamed
        (Some(_), StackTarget::Effect(effect)) => project.effect(effect).map(|(_, found)| tr(effect_type_label(&found.kind)).to_string()).unwrap_or_default(),
        (Some(layer), StackTarget::Content) if matches!(layer.kind, LayerKind::Fill(_)) => tr("Fill").to_string(),
        (Some(layer), StackTarget::Content) if layer.is_group() => tr("Folder").to_string(),
        (Some(layer), StackTarget::Content) if layer.instance_source().is_some() => tr("Instance").to_string(),
        (Some(_), StackTarget::Mask) if !tool_usable => tr("Mask").to_string(),
        (None, _) if !tool_usable => return tr("Properties").to_string(),
        _ => tr(tool.label()).to_string(),
    };
    format!("{} - {what}", tr("Properties"))
}

/// The name of effect type `kind` in the Properties title
fn effect_type_label(kind: &EffectKind) -> &'static str {
    match kind {
        EffectKind::Filter(_) => n_("Filter"),
        other => other.name(),
    }
}

pub(crate) fn properties(app: &mut TextureCraftApp, ui: &mut Ui) {
    // A floating scroll bar would cover the right-aligned slider values and section reset icons
    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
    let scope = SectionScope::Docked;
    if let Some(tabs) = properties_fill_tabs(app) {
        crate::property_sheet::section_tabs(ui, scope, 0, &tabs);
    }

    let selection = properties_selection(app);
    crate::property_sheet::in_scope(ui, scope, |ui| {
        let area = egui::ScrollArea::vertical().id_salt("props").auto_shrink([false, false]);
        let area = app.properties_scroll.prepare(area, ui.scope_id(), selection);
        let offset_points = properties_scrolled(app, ui, area, PastEnd::Always);
        app.properties_scroll.remember(selection, offset_points);
    });
}

/// The layer and target whose scroll offset is shown, or the polygon fill tool's own offset
fn properties_selection(app: &TextureCraftApp) -> crate::property_sheet::PropertiesSelection {
    // Disabled polygon fill shows the selected layer's settings
    if app.session.tool.tool == Tool::PolygonFill && app.session.tool_usable(Tool::PolygonFill) {
        return None;
    }

    app.session.selected_layer().map(|layer| (layer, app.session.target))
}

/// Draw the sections, reserve room for the scrollbar and return the clamped vertical offset
pub(crate) fn properties_scrolled(app: &mut TextureCraftApp, ui: &mut Ui, area: egui::ScrollArea, past_end: PastEnd) -> f32 {
    let sheet_width = (ui.available_width() - ui.spacing().scroll.allocated_width()).max(0.0);
    let output = area.show(ui, |ui| {
        ui.set_max_width(sheet_width);
        crate::property_sheet::forget_sections(ui);
        properties_contents(app, ui);

        // Leave room to align the last section with the top of the list
        let scrolls = ui.min_rect().height() > ui.clip_rect().height();
        if past_end == PastEnd::Always || scrolls {
            ui.add_space(crate::property_sheet::room_past_end(ui));
        }
    });

    output.state.clamped_offset().y
}

/// The tabs over the panel when it shows a fill layer's or a mask fill effect's sections, which they jump between
pub(crate) fn properties_fill_tabs(app: &TextureCraftApp) -> Option<[(&'static str, &'static str); 3]> {
    if app.session.tool.tool == Tool::PolygonFill && app.session.tool_usable(Tool::PolygonFill) {
        return None;
    }

    let project = &app.session.doc.project;
    match app.session.target {
        StackTarget::Content => {
            let layer = app.session.selected_layer().and_then(|id| project.layers.get(&id))?;
            matches!(&layer.kind, LayerKind::Fill(fill) if fill.projection.placed().is_none()).then(|| crate::fill_panel::fill_tabs(false))
        }
        StackTarget::Effect(effect) => mask_fill_of(project, effect).map(|_| crate::fill_panel::fill_tabs(true)),
        _ => None,
    }
}

/// Effect `effect`'s source, projection and sampling when it is a mask fill the Properties panel lays out like a fill (a decal's placed fill keeps its own rows)
fn mask_fill_of(project: &texturecraft_engine::core::Project, effect: LayerId) -> Option<(&Source, &Projection, Sampling)> {
    match &project.effect(effect)?.1.kind {
        EffectKind::Fill { source, projection, sampling } if projection.placed().is_none() => Some((source, projection, *sampling)),
        _ => None,
    }
}

/// Room around the Properties rows not yet laid out by the property sheet: the sheet's side padding, so both start under the panel's name
const LEGACY_BODY_MARGIN: egui::Margin = egui::Margin::symmetric(crate::property_sheet::SIDE_PADDING_POINTS as i8, 4);

/// Lays out `body` with the other panels' margin
fn legacy_body(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    egui::Frame::new().inner_margin(LEGACY_BODY_MARGIN).show(ui, body);
}

/// What the Properties panel edits: polygon fill's own settings, the selected effect, fill, folder, instance or mask, else the tool (or a hint to select a layer when it can't act)
pub(crate) fn properties_contents(app: &mut TextureCraftApp, ui: &mut Ui) {
    if !app.session.project_open {
        crate::widgets::empty_note(ui, tr("No properties found"));
        return;
    }

    let selected = app.session.selected_layer();
    let tool = app.session.tool.tool;
    let tool_usable = app.session.tool_usable(tool);
    let tokens = Tokens::get(ui.ctx());

    match (selected.and_then(|id| app.session.project().layers.get(&id)), app.session.target) {
        // Polygon fill keeps its own settings on any target it can fill
        _ if tool == Tool::PolygonFill && tool_usable => legacy_body(ui, |ui| polygon_fill_properties(app, ui)),
        (Some(layer), StackTarget::Effect(effect)) if mask_fill_of(&app.session.doc.project, effect).is_some() => {
            let layer_id = layer.id;
            mask_fill_properties(app, ui, layer_id, effect);
        }
        (Some(layer), StackTarget::Effect(effect)) => {
            let snapshot = layer.clone();
            effect_properties(app, ui, &snapshot, effect);
            let painted = app.session.doc.project.effect(effect).is_some_and(|(_, effect)| matches!(effect.kind, EffectKind::Paint | EffectKind::Path(_)));
            if painted && grayscale_tab_open(app) {
                crate::mask_paint_panel::grayscale_sheet(app, ui);
            }
        }
        (Some(layer), StackTarget::Content) if matches!(layer.kind, LayerKind::Fill(_)) => {
            let snapshot = layer.clone();
            fill_properties(app, ui, &snapshot);
        }
        (Some(layer), StackTarget::Content) if layer.is_group() => crate::folder_properties::folder_properties(app, ui, layer.id),
        (Some(layer), StackTarget::Content) if layer.instance_source().is_some() => {
            let snapshot = layer.clone();
            crate::instance_properties::instance_properties(app, ui, &snapshot);
        }
        (Some(layer), StackTarget::Mask) => {
            let layer_id = layer.id;
            legacy_body(ui, |ui| {
                // Saved height masks can still be turned off here
                let height_shown = app.session.doc.project.content_layer(layer_id).is_some_and(|content| content.height_mask.enabled);
                if height_shown {
                    height_mask_section(app, ui, layer_id);
                }
                if tool_usable {
                    if height_shown {
                        ui.separator();
                    }
                    tool_properties(app, ui);
                }
            });
            if tool_usable && grayscale_tab_open(app) {
                crate::mask_paint_panel::grayscale_sheet(app, ui);
            }
        }
        (None, _) if !tool_usable => legacy_body(ui, |ui| {
            ui.label(RichText::new(tr("Select a layer to paint on")).color(tokens.text_dim));
        }),
        _ => legacy_body(ui, |ui| tool_properties(app, ui)),
    }
}

/// Whether the tool panel's third tab is open while a mask is painted: its Grayscale tab, drawn under the tool's rows
fn grayscale_tab_open(app: &TextureCraftApp) -> bool {
    app.ui.props_tab == PropsTab::Material && !targets_content(app)
}

/// Mask with height: the layer's height shapes its mask transitions.
fn height_mask_section(app: &mut TextureCraftApp, ui: &mut Ui, layer: LayerId) {
    let height_mask = app.session.project().content_layer(layer).map(|source| source.height_mask).unwrap_or_default();

    section(ui, "sec_height_mask", tr("Mask with height"), true, |ui| {
        let mut on = height_mask.enabled;
        if ui
            .checkbox(&mut on, tr("Combine with the layer's height"))
            .on_hover_text(tr("Partial mask values follow the layer's height: high parts show first"))
            .changed()
        {
            app.run("layer.set", json!({"layer": layer.0, "heightMask": {"enabled": on}}));
        }

        ui.add_enabled_ui(height_mask.enabled, |ui| {
            let (mut contrast, mut offset) = (height_mask.contrast, height_mask.offset);
            if let Some(merge) = slider_edit(ui, tr("Contrast"), &mut contrast, 0.0..=16.0, 2) {
                app.run("layer.set", json!({"layer": layer.0, "heightMask": {"contrast": contrast}, "merge": merge}));
            }
            if let Some(merge) = slider_edit(ui, tr("Height offset"), &mut offset, -1.0..=1.0, 2) {
                app.run("layer.set", json!({"layer": layer.0, "heightMask": {"offset": offset}, "merge": merge}));
            }
        });
    });
}

/// Height of the brush preview over the tool's properties, and its corner radius
const BRUSH_PREVIEW_HEIGHT: f32 = 88.0;
const BRUSH_PREVIEW_CORNER_RADIUS: f32 = 2.0;

/// The preview stroke: its segments, its margin from the preview's left (each side of its half), its wave's phase span in radians and its swing
const PREVIEW_STROKE_STEPS: usize = 40;
const PREVIEW_STROKE_MARGIN: f32 = 14.0;
const PREVIEW_STROKE_WAVE: f32 = 9.0;
const PREVIEW_STROKE_SWING: f32 = 18.0;

/// The preview stroke's width: [`PREVIEW_STROKE_WIDTH_SCALE`] points for each [`PREVIEW_SIZE_UNIT`] of brush size, kept inside [`PREVIEW_STROKE_WIDTH`]
const PREVIEW_SIZE_UNIT: f32 = 0.25;
const PREVIEW_STROKE_WIDTH_SCALE: f32 = 60.0;
const PREVIEW_STROKE_WIDTH: std::ops::RangeInclusive<f32> = 2.0..=30.0;

/// Faintest the preview stroke draws, so a low flow or opacity still shows
const PREVIEW_STROKE_MIN_ALPHA: u8 = 30;

/// Radius of the preview's material sphere, and the colour it shows when the tool has no base colour
const PREVIEW_SPHERE_RADIUS: f32 = 36.0;
const PREVIEW_FALLBACK_BASE_COLOR: [f32; 3] = [0.8; 3];

/// Gap between the brush preview and the tab buttons
const BRUSH_PREVIEW_GAP: f32 = 4.0;

fn tool_properties(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    let content = targets_content(app);

    // Brush preview: a stroke and a sphere with the tool's material
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), BRUSH_PREVIEW_HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, BRUSH_PREVIEW_CORNER_RADIUS, tokens.viewport);
    let half_width = rect.width() * 0.5;
    let brush = &app.session.tool.brush;
    let mut points = Vec::new();
    for step in 0..=PREVIEW_STROKE_STEPS {
        let fraction = step as f32 / PREVIEW_STROKE_STEPS as f32;
        let x = rect.left() + PREVIEW_STROKE_MARGIN + fraction * (half_width - 2.0 * PREVIEW_STROKE_MARGIN);
        points.push(pos2(x, rect.center().y + (fraction * PREVIEW_STROKE_WAVE).sin() * PREVIEW_STROKE_SWING));
    }

    let stroke_width = (brush.size / PREVIEW_SIZE_UNIT * PREVIEW_STROKE_WIDTH_SCALE).clamp(*PREVIEW_STROKE_WIDTH.start(), *PREVIEW_STROKE_WIDTH.end());
    let alpha = (brush.flow * brush.opacity * 255.0) as u8;

    let preview_stroke = |gray: f32| {
        let gray = (gray.clamp(0.0, 1.0) * 255.0).round() as u8;
        Stroke::new(stroke_width, Color32::from_rgba_unmultiplied(gray, gray, gray, alpha.max(PREVIEW_STROKE_MIN_ALPHA)))
    };

    // Sample image-backed masks along the preview stroke
    let gray = if content { texturecraft_engine::compose::MaskGray::Value(1.0) } else { app.session.mask_gray() };
    match gray {
        texturecraft_engine::compose::MaskGray::Value(value) => {
            painter.add(egui::Shape::line(points, preview_stroke(value)));
        }
        texturecraft_engine::compose::MaskGray::Image { .. } => {
            let size = PREVIEW_STROKE_STEPS as u32;
            for (index, segment) in points.windows(2).enumerate() {
                let value = gray.at(index as u32, size / 2, (size, size));
                painter.line_segment([segment[0], segment[1]], preview_stroke(value));
            }
        }
    }

    if content {
        let base_color = app.session.tool.values.get(&Channel::BaseColor).copied().unwrap_or(PREVIEW_FALLBACK_BASE_COLOR);

        // The sphere sits in the middle of the preview's right half
        crate::thumbs::paint_sphere(painter, pos2(rect.left() + half_width * 1.5, rect.center().y), PREVIEW_SPHERE_RADIUS, base_color);
    }
    ui.add_space(BRUSH_PREVIEW_GAP);

    ui.horizontal(|ui| {
        let button_size = ui.spacing().interact_size.y;

        // Painting a mask, the material tab becomes the Grayscale tab
        let material = if content { (ic::SPHERE, tr("Material")) } else { (ic::SQUARE_HALF, tr("Grayscale")) };
        for (tab, icon, tip) in
            [(PropsTab::Brush, ic::PAINT_BRUSH, tr("Brush")), (PropsTab::Alpha, ic::CIRCLE_HALF, tr("Alpha")), (PropsTab::Material, material.0, material.1)]
        {
            if icon_button(ui, icon, tip, app.ui.props_tab == tab, button_size).clicked() {
                app.ui.props_tab = tab;
            }
        }
    });
    ui.separator();

    if app.session.quick_mask.editing {
        ui.label(RichText::new(tr("Editing the quick mask: strokes paint where painting is allowed. Press U to finish.")).color(tokens.text_dim));
        ui.separator();
    }

    let tool_section: Option<fn(&mut TextureCraftApp, &mut Ui)> = match app.session.tool.tool {
        Tool::Projection => Some(stencil_section),
        Tool::Clone => Some(clone_section),
        Tool::Smudge => Some(smudge_section),
        Tool::Path => Some(path_section),
        Tool::GeometryMask => Some(geometry_tool_section),
        _ => None,
    };
    if let Some(section) = tool_section {
        section(app, ui);
        ui.separator();
    }

    match app.ui.props_tab {
        PropsTab::Brush => brush_section(app, ui),
        PropsTab::Alpha => alpha_section(app, ui),
        PropsTab::Material if content => material_section(app, ui),
        // A mask's Grayscale tab is a property sheet, drawn by the caller past this panel's margins
        PropsTab::Material => {}
    }
}

/// How far the Position sliders move the stencil's center from the middle of the view (NDC; -1..1 is the view)
const STENCIL_POSITION_RANGE: f32 = 2.0;

fn stencil_section(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    ui.label(caption(tr("Stencil")));
    let names: Vec<String> = app.session.doc.resources.keys().cloned().collect();
    let current = app.session.tool.stencil.as_ref().map(|s| s.image.clone()).unwrap_or_else(|| tr("None").into());
    let row = ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("stencil_img").width(170.0).selected_text(current.clone()).show_ui(ui, |ui| {
            if ui.selectable_label(app.session.tool.stencil.is_none(), tr("None")).clicked() {
                app.run("tool.stencil", json!({"clear": true}));
            }
            for n in &names {
                if ui.selectable_label(*n == current, n).clicked() {
                    app.run("tool.stencil", json!({"image": n}));
                }
            }
        });
        import_button(app, ui);
    });

    // An image from the Assets panel dropped on the row becomes the stencil
    let row_response = row.response.interact(Sense::hover()).on_hover_text(tr("Drop an image from the Assets panel here to use it as the stencil"));
    if let Some(name) = image_drop(ui, &row_response) {
        app.run("tool.stencil", json!({"image": name}));
    }
    if names.is_empty() {
        ui.label(RichText::new(tr("Import an image (File > Import resources) to project it onto the mesh.")).color(tk.text_dim));
        return;
    }

    if let Some(st) = app.session.tool.stencil.as_mut() {
        slider(ui, tr("Scale"), &mut st.scale, 0.05..=4.0, 2);
        slider(ui, tr("Rotation"), &mut st.rotation, 0.0..=360.0, 0);
        slider(ui, tr("Position X"), &mut st.center[0], -STENCIL_POSITION_RANGE..=STENCIL_POSITION_RANGE, 2);
        slider(ui, tr("Position Y"), &mut st.center[1], -STENCIL_POSITION_RANGE..=STENCIL_POSITION_RANGE, 2);
        if ui.button(tr("Reset stencil")).on_hover_text(tr("Put the stencil back in the middle of the view at its starting size and rotation")).clicked() {
            st.reset_placement();
        }
        slider(ui, tr("Overlay opacity"), &mut st.opacity, 0.0..=1.0, 2);
        ui.checkbox(&mut st.tile, tr("Tile"));
        ui.label(
            RichText::new(tr("S + left-drag rotates (with Shift in 15° steps), S + right-drag scales, S + middle-drag moves; or use Position X / Y."))
                .size(10.5)
                .color(tk.text_dim),
        );
    }
}

fn import_button(app: &mut TextureCraftApp, ui: &mut Ui) {
    if ui.small_button(tr("Import...")).clicked() {
        let ctx = ui.ctx().clone();
        crate::menus::invoke(app, &ctx, "resource.import");
    }
}

fn clone_section(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    ui.label(caption(tr("Clone")));
    ui.horizontal(|ui| {
        ui.label(tr("Copy from"));
        for (mode, label, tip, id) in [
            (CloneFrom::Composite, tr("All layers"), tr("Copy what the texture set shows"), "composite"),
            (CloneFrom::Layer, tr("Current layer"), tr("Copy the selected layer alone (its empty parts copy nothing)"), "layer"),
        ] {
            if chip(ui, label, app.session.tool.clone_from == mode).on_hover_text(tip).clicked() {
                app.run("tool.brush", json!({"cloneSource": id}));
            }
        }
    });
    let mut aligned = app.session.tool.clone_aligned;
    if ui
        .checkbox(&mut aligned, tr("Aligned"))
        .on_hover_text(tr("On: the offset from the first stroke is kept. Off: every stroke starts at the source point."))
        .changed()
    {
        app.run("tool.brush", json!({"cloneAligned": aligned}));
    }
    let Some(src) = app.session.tool.clone_source else {
        ui.label(RichText::new(tr("Hold V and click the mesh to set the source, then paint elsewhere to copy it.")).color(tk.text_dim));
        return;
    };

    ui.horizontal(|ui| {
        ui.label(trf("Source  U {u}  V {v}", &[("u", &format!("{:.3}", src.uv[0])), ("v", &format!("{:.3}", src.uv[1]))]));
        if ui.small_button(tr("Clear")).clicked() {
            app.run("tool.cloneSource", json!({"clear": true}));
        }
    });
    let hint = if !app.session.tool.clone_aligned {
        tr("Not aligned: every stroke starts at the source. V + click sets a new source.")
    } else if src.offset.is_some() {
        tr("Aligned: the offset set by the first stroke is kept. V + click sets a new source.")
    } else {
        tr("The first stroke sets the offset from the source.")
    };
    ui.label(RichText::new(hint).size(10.5).color(tk.text_dim));
}

fn smudge_section(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    ui.label(caption(tr("Smudge")));
    let mut v = app.session.tool.smudge_strength * 100.0;
    if slider(ui, tr("Strength"), &mut v, 0.0..=100.0, 0).changed() {
        app.session.tool.smudge_strength = (v / 100.0).clamp(0.0, 1.0);
    }
    ui.label(RichText::new(tr("Drags the layer stack's values along the stroke into the selected layer.")).size(10.5).color(tk.text_dim));
    ui.add_space(4.0);
    ui.label(tr("Channels dragged"));
    material_channel_chips(app, ui);
    ui.label(RichText::new(tr("These are the brush's material channels, the same ones Paint writes and the Material tab shows. A channel the selected layer does not have is skipped.")).size(10.5).color(tk.text_dim));
}

fn path_section(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    ui.label(caption(tr("Path")));
    ui.horizontal(|ui| crate::toolbar::path_controls(app, ui));
    let n = app.session.tool.path.points.len();
    let hint = trf(
        "{n} control points. Click the mesh (or the 2D view) to add points and drag them to adjust; click the first point to close the path. Enter paints it with the brush (a closed, filled path fills its inside), Esc cancels.",
        &[("n", &n)],
    );
    ui.label(RichText::new(hint).size(10.5).color(tk.text_dim));
}

/// Popover for built-in and saved brush presets
fn brush_presets_popover(app: &mut TextureCraftApp, ui: &mut Ui) {
    let response = ui.button(format!("{}  {}", ic::PAINT_BRUSH_BROAD, tr("Brush presets")));
    let popup_id = ui.make_persistent_id("brush_presets_popup");
    if response.clicked() {
        egui::Popup::toggle_id(ui.ctx(), popup_id);
    }

    egui::Popup::from_response(&response).id(popup_id).open_memory(None).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(230.0);
        egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
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

/// Width of the label column in polygon fill's properties
const POLYGON_FILL_LABEL_WIDTH: f32 = 108.0;

/// Polygon fill's properties: how it selects and fills, its blending, then what it fills with (material, mask gray, or why it can't fill)
fn polygon_fill_properties(app: &mut TextureCraftApp, ui: &mut Ui) {
    use texturecraft_engine::FillHitTest;

    let button_size = ui.spacing().interact_size.y;
    ui.label(caption(&tr("Polygon fill").to_uppercase()));
    property_row(ui, tr("Selection mode"), tr("A click fills what is under it; a dragged rectangle fills what it selects as it grows"), |ui| {
        icon_button(ui, ic::SELECTION, tr("Rectangle fill"), true, button_size);
    });
    property_row(
        ui,
        tr("Fill mode"),
        tr("Triangle fills single triangles, Polygon whole quads and n-gons, Mesh the connected geometry, UV island the UV islands"),
        |ui| crate::toolbar::fill_mode_buttons(app, ui),
    );
    property_row(ui, tr("Hit-test"), tr("How a rectangle selects polygons"), |ui| {
        for (hit_test, icon, tooltip) in [
            (FillHitTest::Intersection, ic::INTERSECT_SQUARE, tr("Intersection: fill polygons the rectangle touches")),
            (FillHitTest::ContainedCenter, ic::SELECTION_FOREGROUND, tr("Contained center: fill polygons whose center is in the rectangle")),
        ] {
            if icon_button(ui, icon, tooltip, app.session.tool.fill_hit_test == hit_test, button_size).clicked() {
                app.session.tool.fill_hit_test = hit_test;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.session.tool.fill_visible_only, tr("Visible polygons only"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| info_icon(ui, tr("Fill only polygons the camera sees (3D view)")));
    });
    section(ui, "sec_polygon_fill_blending", tr("Blending"), false, |ui| {
        let mut opacity = app.session.tool.brush.opacity * 100.0;
        if slider(ui, tr("Opacity"), &mut opacity, 0.0..=100.0, 0).changed() {
            app.session.tool.brush.opacity = (opacity / 100.0).clamp(0.0, 1.0);
        }
    });
    ui.separator();

    match polygon_fill_paints_mask(app) {
        Ok(false) => {
            ui.label(caption(&tr("Material").to_uppercase()));
            material_controls(app, ui);
        }
        Ok(true) => {
            ui.label(caption(&tr("Grayscale").to_uppercase()));
            let mut gray = app.session.tool.mask_value;
            if slider(ui, tr("Value"), &mut gray, 0.0..=1.0, 2).changed() {
                app.run("tool.material", json!({"maskValue": gray}));
            }
        }
        Err(note) => {
            ui.label(RichText::new(note).color(Tokens::get(ui.ctx()).text_dim));
        }
    }
}

/// One polygon fill setting: its label in a fixed column, its `controls`, and an info icon with `info` at the right edge
fn property_row(ui: &mut Ui, label: &str, info: &str, controls: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let label_width = ui.label(label).rect.width();
        ui.add_space((POLYGON_FILL_LABEL_WIDTH - label_width - ui.spacing().item_spacing.x).max(0.0));
        controls(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| info_icon(ui, info));
    });
}

/// A dim info icon explaining a setting on hover
pub(crate) fn info_icon(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(ic::INFO).color(Tokens::get(ui.ctx()).text_dim)).on_hover_text(text);
}

/// What a polygon fill paints into now: Ok(true) a mask (gray), Ok(false) layer content (material), or a note on why it can't fill
fn polygon_fill_paints_mask(app: &TextureCraftApp) -> Result<bool, String> {
    if app.session.quick_mask.editing {
        return Err(tr("Editing the quick mask: fills paint where painting is allowed. Press U to finish.").to_string());
    }

    let project = &app.session.doc.project;
    let Some(layer) = app.session.selected_layer().and_then(|id| project.layers.get(&id)) else {
        return Err(tr("Select a paint layer or a layer's mask to fill").to_string());
    };

    match app.session.target {
        StackTarget::Mask => Ok(true),
        StackTarget::Effect(_) => Ok(!targets_content(app)),
        StackTarget::Content => {
            let content = project.content_layer(layer.id).unwrap_or(layer);
            if matches!(content.kind, LayerKind::Paint) {
                Ok(false)
            } else {
                Err(trf("\"{layer}\" can't be filled: select a paint layer or a layer's mask", &[("layer", &layer.name)]))
            }
        }
    }
}

/// Font size of the dim hint line under the Brush caption
const HINT_TEXT_SIZE: f32 = 10.5;

fn brush_section(app: &mut TextureCraftApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(caption(tr("Brush")));
        brush_presets_popover(app, ui);
    });

    let brush = &mut app.session.tool.brush;
    let defaults = BrushSettings::default();
    let hint = tr("Ctrl+right-drag in a view sets the size and hardness, Ctrl+left-drag the flow and angle.");
    ui.label(RichText::new(hint).size(HINT_TEXT_SIZE).color(Tokens::get(ui.ctx()).text_dim));
    ui.label(RichText::new(tr("Ctrl+Shift+right-drag: size and hardness in fine steps.")).size(HINT_TEXT_SIZE).color(Tokens::get(ui.ctx()).text_dim));

    // Each section's reset puts back only the fields it shows
    let size_fields = |settings: &BrushSettings| (settings.size, settings.min_size, settings.screen_space);
    if section_with_reset(ui, "sec_size", tr("Size"), true, size_fields(brush) == size_fields(&defaults), |ui| {
        let mut size_percent = brush.size / MAX_BRUSH_SIZE * 100.0;
        if slider(ui, tr("Size"), &mut size_percent, 0.05..=100.0, 1).changed() {
            brush.size = (size_percent / 100.0 * MAX_BRUSH_SIZE).clamp(MIN_BRUSH_SIZE, MAX_BRUSH_SIZE);
        }

        let mut min_size_percent = brush.min_size * 100.0;
        if slider(ui, tr("Minimum size (%)"), &mut min_size_percent, 0.0..=100.0, 0).changed() {
            brush.min_size = min_size_percent / 100.0;
        }

        ui.checkbox(&mut brush.screen_space, tr("Size in screen space"));
    }) {
        (brush.size, brush.min_size, brush.screen_space) = size_fields(&defaults);
    }

    let flow_fields = |settings: &BrushSettings| (settings.flow, settings.opacity, settings.spacing);
    if section_with_reset(ui, "sec_flow", tr("Flow and opacity"), true, flow_fields(brush) == flow_fields(&defaults), |ui| {
        let mut flow_percent = brush.flow * 100.0;
        if slider(ui, tr("Flow"), &mut flow_percent, 0.0..=100.0, 0).changed() {
            brush.flow = flow_percent / 100.0;
        }

        let mut opacity_percent = brush.opacity * 100.0;
        if slider(ui, tr("Stroke opacity"), &mut opacity_percent, 0.0..=100.0, 0).changed() {
            brush.opacity = opacity_percent / 100.0;
        }

        let mut spacing_percent = brush.spacing * 100.0;
        if slider(ui, tr("Spacing"), &mut spacing_percent, 1.0..=1000.0, 0).changed() {
            brush.spacing = spacing_percent / 100.0;
        }
    }) {
        (brush.flow, brush.opacity, brush.spacing) = flow_fields(&defaults);
    }

    let angle_fields = |settings: &BrushSettings| (settings.angle, settings.follow_path);
    if section_with_reset(ui, "sec_angle", tr("Angle"), false, angle_fields(brush) == angle_fields(&defaults), |ui| {
        slider(ui, tr("Angle"), &mut brush.angle, -180.0..=180.0, 0);
        ui.checkbox(&mut brush.follow_path, tr("Follow path"));
    }) {
        (brush.angle, brush.follow_path) = angle_fields(&defaults);
    }

    let jitter_fields = |settings: &BrushSettings| (settings.jitter_size, settings.jitter_angle, settings.jitter_flow, settings.jitter_position);
    if section_with_reset(ui, "sec_jitter", tr("Jitters"), false, jitter_fields(brush) == jitter_fields(&defaults), |ui| {
        let jitters = [(tr("Size jitter"), &mut brush.jitter_size), (tr("Angle jitter"), &mut brush.jitter_angle), (tr("Flow jitter"), &mut brush.jitter_flow)];
        for (label, jitter) in jitters {
            let mut jitter_percent = *jitter * 100.0;
            if slider(ui, label, &mut jitter_percent, 0.0..=100.0, 0).changed() {
                *jitter = jitter_percent / 100.0;
            }
        }

        let mut position_jitter_percent = brush.jitter_position * 100.0;
        if slider(ui, tr("Position jitter"), &mut position_jitter_percent, 0.0..=400.0, 0).changed() {
            brush.jitter_position = position_jitter_percent / 100.0;
        }
    }) {
        (brush.jitter_size, brush.jitter_angle, brush.jitter_flow, brush.jitter_position) = jitter_fields(&defaults);
    }

    let alignment_fields = |settings: &BrushSettings| (settings.alignment, settings.backface_culling, settings.angle_fade, settings.depth);
    if section_with_reset(ui, "sec_align", tr("Alignment and culling"), false, alignment_fields(brush) == alignment_fields(&defaults), |ui| {
        use texturecraft_engine::paint::Alignment;
        let alignments = [(Alignment::Tangent, tr("Tangent | Wrap")), (Alignment::Camera, tr("Camera"))];
        let shown = alignments.iter().find(|(alignment, _)| *alignment == brush.alignment).map_or("", |(_, label)| label);
        combo(ui, tr("Alignment"), shown, |ui| {
            for (alignment, label) in alignments {
                ui.selectable_value(&mut brush.alignment, alignment, label);
            }
        });

        ui.checkbox(&mut brush.backface_culling, tr("Backface culling"));
        slider(ui, tr("Angle fade from"), &mut brush.angle_fade.0, 0.0..=90.0, 0);
        slider(ui, tr("Angle fade to"), &mut brush.angle_fade.1, 0.0..=90.0, 0);
        slider(ui, tr("Stamp depth"), &mut brush.depth, 0.05..=4.0, 2);
    }) {
        (brush.alignment, brush.backface_culling, brush.angle_fade, brush.depth) = alignment_fields(&defaults);
    }

    if section_with_reset(ui, "sec_particles", tr("Particles"), false, brush.particles == defaults.particles, |ui| {
        let particles = &mut brush.particles;
        ui.checkbox(&mut particles.enabled, tr("Particle brush (slides down the surface)"));

        let mut count = particles.count as f32;
        if slider(ui, tr("Count per stamp"), &mut count, 1.0..=16.0, 0).changed() {
            particles.count = count as u32;
        }

        let mut lifetime_steps = particles.lifetime as f32;
        if slider(ui, tr("Lifetime (steps)"), &mut lifetime_steps, 1.0..=128.0, 0).changed() {
            particles.lifetime = lifetime_steps as u32;
        }

        slider(ui, tr("Gravity"), &mut particles.gravity, 0.0..=2.0, 2);
    }) {
        brush.particles = defaults.particles;
    }

    if section_with_reset(ui, "sec_lazy", tr("Lazy mouse"), false, brush.lazy == defaults.lazy, |ui| {
        slider(ui, tr("Lazy mouse distance"), &mut brush.lazy, 0.0..=200.0, 0);
    }) {
        brush.lazy = defaults.lazy;
    }

    *brush = brush.sanitized();
}

/// One alpha in the picker: a 52×64 tile with the shape drawn by `paint` and a label.
fn alpha_tile(ui: &mut Ui, on: bool, label: &str, paint: impl FnOnce(&egui::Painter, Rect)) -> egui::Response {
    let tk = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(52.0, 64.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, tk.viewport);
    if on {
        p.rect_stroke(rect, 3.0, Stroke::new(1.0, tk.accent), egui::StrokeKind::Inside);
    }
    paint(p, Rect::from_center_size(rect.center() - vec2(0.0, 7.0), vec2(40.0, 40.0)));
    let name = elided(p, label, 9.5, tk.viewport_text, rect.width() - 4.0);
    p.galley(rect.center_bottom() - vec2(name.size().x * 0.5, 6.0 + name.size().y * 0.5), name, tk.viewport_text);
    resp.on_hover_text(label)
}

fn alpha_section(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(caption(tr("Alpha")));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| import_button(app, ui));
    });
    let brush = &app.session.tool.brush;
    let (current, image, hardness) = (brush.alpha, brush.alpha_image.clone(), brush.hardness);
    let library = if image.is_none() { brush.alpha_library.as_deref().and_then(alphas::find) } else { None };
    let mut pick = None;
    let zone = ui.horizontal_wrapped(|ui| {
        for a in BrushAlpha::ALL {
            let on = image.is_none() && library.is_none() && current == a;
            if alpha_tile(ui, on, tr(a.label()), |p, r| crate::thumbs::paint_alpha(p, r, a, hardness)).clicked() {
                pick = Some(json!(a.label()));
            }
        }

        // The library alpha in use (picked from the shelf's Alphas tab)
        if let Some(a) = library {
            let tex = app.thumbs.alpha(ui.ctx(), a);
            alpha_tile(ui, true, &a.label, |p, r| {
                if let Some(t) = tex {
                    p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
            });
        }

        // Imported images, sampled as grayscale
        for name in app.session.doc.resources.keys() {
            let Some(img) = app.session.images.get(name) else { continue };
            let tex = app.thumbs.image(ui.ctx(), name, img);
            let on = image.as_deref() == Some(name.as_str());
            if alpha_tile(ui, on, name, |p, r| {
                p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            })
            .clicked()
            {
                pick = Some(json!({ "image": name }));
            }
        }
    });
    if let Some(alpha) = pick {
        app.run("tool.brush", json!({ "alpha": alpha }));
    }
    ui.label(RichText::new(tr("More in Assets > Alphas: click one, or drag it here")).size(10.0).color(tk.text_dim));
    crate::shelf::drop_target(app, ui, &zone.response, crate::shelf::DropZone::Brush);
    alpha_parameters(&mut app.session.tool.brush, ui);
    section(ui, "sec_text", tr("Text"), true, |ui| text_section(app, ui));
}

/// Hardness and the parameters every alpha shares, under a header whose reset puts them back
fn alpha_parameters(brush: &mut BrushSettings, ui: &mut Ui) {
    let defaults = BrushSettings::default();
    let at_defaults = brush.hardness == defaults.hardness && brush.alpha_params == defaults.alpha_params;
    let round = brush.alpha == BrushAlpha::Round && brush.alpha_image.is_none() && brush.alpha_library.is_none();
    let percent_slider = |ui: &mut Ui, label: &str, value: &mut f32, min: f32, max: f32| {
        let mut percent = *value * 100.0;
        if slider(ui, label, &mut percent, min..=max, 0).changed() {
            *value = percent / 100.0;
        }
    };

    let reset = section_with_reset(ui, "sec_alpha_params", tr("Parameters"), true, at_defaults, |ui| {
        percent_slider(ui, tr("Hardness"), &mut brush.hardness, 0.0, 99.9);
        percent_slider(ui, tr("Size X"), &mut brush.alpha_params.size_x, 5.0, 100.0);
        percent_slider(ui, tr("Size Y"), &mut brush.alpha_params.size_y, 5.0, 100.0);

        // Only the procedural Round shape has a roundness
        ui.add_enabled_ui(round, |ui| percent_slider(ui, tr("Roundness"), &mut brush.alpha_params.roundness, 0.0, 100.0));
        slider(ui, tr("Contrast"), &mut brush.alpha_params.contrast, 1.0..=MAX_ALPHA_CONTRAST, 2);
        ui.checkbox(&mut brush.alpha_params.invert, tr("Invert"));
    });
    if reset {
        (brush.hardness, brush.alpha_params) = (defaults.hardness, defaults.alpha_params);
    }
    *brush = brush.sanitized();
}

/// Text drawn in a font, as the brush alpha or a decal (`resource.text`).
fn text_section(app: &mut TextureCraftApp, ui: &mut Ui) {
    if app.ui.text.fonts.is_empty()
        && let Some(f) = app.run("text.fonts", Value::Null)
    {
        let installed = f["installed"].as_array().into_iter().flatten().map(|v| {
            let styles = v["styles"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            (v["family"].as_str().unwrap_or_default().to_string(), styles)
        });
        let built_in = f["builtIn"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|n| (n.to_string(), Vec::new()));
        app.ui.text.fonts = installed.chain(built_in).collect();
    }
    // A decal shows the text in the tool's base color; an alpha must stay white.
    let color = app.session.tool.values.get(&Channel::BaseColor).copied().unwrap_or([1.0; 3]);
    let t = &mut app.ui.text;
    ui.add(egui::TextEdit::multiline(&mut t.text).desired_rows(2).desired_width(f32::INFINITY).hint_text(tr("Text")));
    let shown = if t.font.is_empty() { tr("Default").to_string() } else { t.font.clone() };
    combo(ui, tr("Font"), shown, |ui| {
        if ui.selectable_label(t.font.is_empty(), tr("Default")).clicked() {
            (t.font, t.style) = (String::new(), String::new());
        }
        for (family, _) in &t.fonts {
            if ui.selectable_label(t.font == *family, family).clicked() {
                (t.font, t.style) = (family.clone(), String::new());
            }
        }
    });
    let styles = t.fonts.iter().find(|(f, _)| *f == t.font).map(|(_, s)| s.clone()).unwrap_or_default();
    if styles.len() > 1 {
        let shown = if t.style.is_empty() { tr("Default").to_string() } else { t.style.clone() };
        combo(ui, tr("Style"), shown, |ui| {
            for s in styles {
                ui.selectable_value(&mut t.style, s.clone(), s);
            }
        });
    }
    let mut params = json!({"text": t.text, "font": (!t.font.is_empty()).then_some(&t.font), "style": (!t.style.is_empty()).then_some(&t.style)});
    let ready = !t.text.trim().is_empty();
    ui.horizontal_wrapped(|ui| {
        let alpha = ui.add_enabled(ready, egui::Button::new(tr("Use as brush alpha"))).clicked();
        let decal = ui.add_enabled(ready, egui::Button::new(tr("Place as decal"))).clicked();
        if decal {
            params["color"] = json!(color);
        }
        if (alpha || decal)
            && let Some(name) = app.run("resource.text", params).and_then(|r| r["name"].as_str().map(str::to_string))
        {
            match alpha {
                true => app.run("tool.brush", json!({"alpha": {"image": name}})),
                false => app.run("layer.add.decal", json!({"image": name})),
            };
        }
    });
}

/// Tooltip of the material channel chips.
pub(crate) const SOLO_HINT: &str = "Click: turn this channel on or off. Alt+click: this channel only";

/// The tool's material under a "Material" heading
fn material_section(app: &mut TextureCraftApp, ui: &mut Ui) {
    ui.label(caption(tr("Material")));
    material_controls(app, ui);
}

/// Space between the tool's channel chips and the first channel value
const CHIPS_GAP: f32 = 4.0;

/// Room right of the tool's colour swatch, like a linked material's
const TOOL_SWATCH_RIGHT_MARGIN: f32 = LINK_SWATCH_RIGHT_MARGIN;

/// One chip per channel of the texture set, on while the tool's material has it enabled; returns the channels.
/// The Material tab and the Smudge panel both draw these, so they share one set of flags.
fn material_channel_chips(app: &mut TextureCraftApp, ui: &mut Ui) -> Vec<Channel> {
    let channels = app.session.doc.project.texture_set(app.session.ts()).map(|set| set.channel_list()).unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        for channel in &channels {
            let on = app.session.tool.enabled.contains(channel);
            if chip(ui, tr(channel.short()), on).on_hover_text(tr(SOLO_HINT)).clicked() {
                let solo = ui.input(|input| input.modifiers.alt);
                app.run("tool.material", json!({"channel": channel.id(), "enabled": solo || !on, "solo": solo}));
            }
        }
    });
    channels
}

/// The tool's material: channel chips, then each enabled channel's value
fn material_controls(app: &mut TextureCraftApp, ui: &mut Ui) {
    use texturecraft_engine::core::ChannelKind;

    let tokens = Tokens::get(ui.ctx());
    let channels = material_channel_chips(app, ui);
    ui.add_space(CHIPS_GAP);

    for channel in channels {
        if !app.session.tool.enabled.contains(&channel) {
            continue;
        }

        let mut value = app.session.tool.values.get(&channel).copied().unwrap_or(channel.default_value());
        match channel.kind() {
            ChannelKind::Color => {
                ui.horizontal(|ui| {
                    ui.label(tr(channel.label()));
                    if color_swatch(ui, &format!("tool_{}", channel.id()), &mut value, ui.available_width() - TOOL_SWATCH_RIGHT_MARGIN) {
                        app.session.tool.values.insert(channel, value);
                        app.session.tool.material = None;
                    }
                });
            }
            ChannelKind::NormalMap => {
                ui.label(RichText::new(tr("Normal: painting uses the flat normal")).color(tokens.text_dim));
            }
            ChannelKind::Signed => {
                if slider(ui, tr(channel.label()), &mut value[0], -1.0..=1.0, 2).changed() {
                    app.session.tool.values.insert(channel, [value[0]; 3]);
                    app.session.tool.material = None;
                }
            }
            ChannelKind::Scalar => {
                if slider(ui, tr(channel.label()), &mut value[0], 0.0..=1.0, 2).changed() {
                    app.session.tool.values.insert(channel, [value[0]; 3]);
                    app.session.tool.material = None;
                }
            }
        }
    }

    if app.session.tool.tool == Tool::Eraser {
        ui.label(RichText::new(tr("The eraser removes paint from the enabled channels.")).color(tokens.text_dim));
    }
}

fn fill_properties(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer) {
    let LayerKind::Fill(fill) = &layer.kind else { return };
    let on: Vec<Channel> = layer.channels.keys().copied().collect();
    PropertySheet::show(ui, ("fill_sheet", layer.id.0), |sheet| {
        crate::fill_panel::fill_sheet(app, sheet, fill, &json!({"layer": layer.id.0}), &on, layer.id);
    });
}

/// Writes the material's package to a temporary file and opens it in MaterialCraft
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn open_link_in_materialcraft(app: &mut TextureCraftApp, layer: LayerId, link: &texturecraft_engine::core::MaterialLink) {
    let ext = texturecraft_engine::link_cmds::source_extension(&link.package);
    let path = std::env::temp_dir().join(format!("{}.{ext}", texturecraft_engine::formats::export::sanitize_file_stem(&link.name)));
    let opened = app
        .session
        .link_package(layer)
        .and_then(|package| std::fs::write(&path, package.as_slice()).map_err(|err| texturecraft_engine::EngineError::Other(err.to_string())))
        .and_then(|()| texturecraft_engine::link_cmds::open_in_materialcraft(&path));
    match opened {
        Ok(()) => app.toast(tr("Opened in MaterialCraft: use File > Send to TextureCraft there to bring your changes back"), false),
        Err(e) => app.toast(e.to_string(), true),
    }
}

/// Room the colour swatch of a linked-material input leaves at the row's right
const LINK_SWATCH_RIGHT_MARGIN: f32 = 4.0;

/// The editor of a linked-material colour or vector input (the sheet draws the other kinds): the new value and whether it continues a colour drag
pub(crate) fn link_param_editor(ui: &mut Ui, layer: LayerId, param: &LinkParam) -> Option<(Value, bool)> {
    let kind = param.default.get("t").and_then(Value::as_str).unwrap_or("");
    let value = param.value.get("v").unwrap_or(&Value::Null);
    let number = |item: &Value| item.as_f64().unwrap_or(0.0) as f32;
    let (low, high) = if param.max > param.min { (param.min, param.max) } else { (0.0, 1.0) };

    if kind == "color" {
        let components = value.as_array().map(|items| items.iter().map(number).collect::<Vec<f32>>()).unwrap_or_default();
        let mut rgb = [0, 1, 2].map(|index| components.get(index).copied().unwrap_or(1.0));
        let swatch_id = format!("link_{}_{}", layer.0, param.id);
        let changed = ui
            .horizontal(|ui| {
                ui.label(&param.label);
                color_swatch(ui, &swatch_id, &mut rgb, ui.available_width() - LINK_SWATCH_RIGHT_MARGIN)
            })
            .inner;

        // The picker changes the colour every frame of a drag: those frames share one undo step
        let merge = press_edit(ui, ui.make_persistent_id((swatch_id.as_str(), "press")), changed)?;
        return Some((json!([rgb[0], rgb[1], rgb[2], components.get(3).copied().unwrap_or(1.0)]), merge));
    }

    // Vectors: one slider per component; an edit applies when the drag ends, on a click or a typed value
    let mut components: Vec<f32> = value.as_array().map(|items| items.iter().map(number).collect()).unwrap_or_default();
    let decimals = if kind == "int2" { 0 } else { 2 };
    let mut changed = false;
    for (component, axis) in components.iter_mut().zip(["x", "y", "z", "w"]) {
        let current = *component;
        let response = slider(ui, &format!("{} {axis}", param.label), component, low.min(current)..=high.max(current), decimals);
        if response.drag_stopped() || (!response.dragged() && *component != current) {
            changed = true;
        } else {
            *component = current;
        }
    }
    changed.then(|| (json!(components), false))
}

/// `base` with the fields of `extra` added
pub(crate) fn with(base: &Value, extra: Value) -> Value {
    let mut merged = base.clone();
    if let (Some(fields), Value::Object(extra_fields)) = (merged.as_object_mut(), extra) {
        fields.extend(extra_fields);
    }
    merged
}

/// Anchor points (content and mask ones) below layer `reader` in its texture set: (effect id, its name, or "layer / name" where two share a name; marked off in a mask turned off)
pub(crate) fn anchors_below(project: &texturecraft_engine::core::Project, reader: LayerId) -> Vec<(LayerId, String)> {
    let Some((texture_set, _, _)) = project.locate(reader) else { return Vec::new() };

    // (id, layer name, anchor name, in a mask, the mask off)
    let anchors: Vec<(LayerId, &str, &str, bool, bool)> = project
        .walk(texture_set)
        .into_iter()
        .map(|(id, _)| id)
        .take_while(|id| *id != reader)
        .filter_map(|id| project.layers.get(&id))
        .flat_map(|layer| {
            let content = layer.content_effects.iter().map(|effect| (effect, false, false));
            let mask = layer.mask.iter().flat_map(|mask| mask.effects.iter().map(move |effect| (effect, true, !mask.enabled)));
            content
                .chain(mask)
                .filter(|(effect, _, _)| matches!(effect.kind, EffectKind::Anchor))
                .map(move |(effect, in_mask, off)| (effect.id, layer.name.as_str(), effect.name.as_str(), in_mask, off))
        })
        .collect();
    let shared = |name: &str| anchors.iter().filter(|anchor| anchor.2 == name).count() > 1;
    anchors
        .iter()
        .map(|&(id, layer, name, in_mask, off)| {
            let label = match (shared(name), in_mask) {
                (false, _) => name.to_owned(),
                (true, false) => format!("{layer} / {name}"),
                (true, true) => trf("{layer} / mask / {name}", &[("layer", &layer), ("name", &name)]),
            };
            (id, if off { format!("{label} ({})", tr("Off")) } else { label })
        })
        .collect()
}

/// Anchor choices valid below both the effect's owner and its displayed instance
pub(crate) fn effect_anchors_below(app: &TextureCraftApp, layer: LayerId, effect: LayerId) -> Vec<(LayerId, String)> {
    project_effect_anchors_below(&app.session.doc.project, layer, effect)
}

/// Anchor choices for the Properties source picker
pub(crate) fn project_effect_anchors_below(project: &texturecraft_engine::core::Project, layer: LayerId, effect: LayerId) -> Vec<(LayerId, String)> {
    let owner = project.effect(effect).map_or(layer, |(owner, _)| owner);
    let owner_anchors = anchors_below(project, owner);
    if owner == layer {
        return owner_anchors;
    }

    let displayed_anchors = anchors_below(project, layer);
    owner_anchors.into_iter().filter(|(id, _)| displayed_anchors.iter().any(|(displayed, _)| id == displayed)).collect()
}

/// Whether anchor point `anchor` sits in a mask (it then records the mask's gray, whatever the channel).
pub(crate) fn anchor_in_mask(app: &TextureCraftApp, anchor: LayerId) -> bool {
    let project = &app.session.doc.project;
    project
        .effect(anchor)
        .and_then(|(owner, _)| project.layers.get(&owner))
        .is_some_and(|layer| layer.mask.iter().flat_map(|m| m.effects.iter()).any(|effect| effect.id == anchor))
}

/// Whether this mask anchor currently records a value
fn mask_anchor_enabled(app: &TextureCraftApp, anchor: LayerId) -> Option<bool> {
    let project = &app.session.doc.project;
    let (owner, _) = project.effect(anchor)?;
    let mask = project.layers.get(&owner)?.mask.as_ref()?;
    mask.effects.iter().any(|effect| effect.id == anchor).then_some(mask.enabled)
}

pub(crate) fn anchor_label(app: &TextureCraftApp, anchor: LayerId, channel: Channel) -> String {
    let name = app.session.doc.project.effect(anchor).map(|(_, e)| e.name.clone()).unwrap_or_else(|| tr("missing").into());
    let label = trf("Anchor: {name} ({channel})", &[("name", &name), ("channel", &tr(channel.label()))]);
    if mask_anchor_enabled(app, anchor) == Some(false) { format!("{label} ({})", tr("Off")) } else { label }
}

/// A mask fill effect: its Fill, Symmetry and Grayscale sections as one sheet (its blending is on its row)
fn mask_fill_properties(app: &mut TextureCraftApp, ui: &mut Ui, layer_id: LayerId, effect_id: LayerId) {
    let Some((source, projection, sampling)) =
        mask_fill_of(&app.session.doc.project, effect_id).map(|(source, projection, sampling)| (source.clone(), projection.clone(), sampling))
    else {
        return;
    };

    PropertySheet::show(ui, ("mask_fill_sheet", effect_id.0), |sheet| {
        crate::mask_fill_panel::mask_fill_sheet(app, sheet, effect_id, source, projection, sampling, layer_id);
    });
}

/// Slider range of a fill's scale or tiling
const FILL_SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.01..=20.0;

/// Slider range of a UV fill's rotation, a full turn
const FILL_ROTATION_DEGREES_RANGE: std::ops::RangeInclusive<f32> = 0.0..=360.0;

/// Slider range of a fill's UV offset and world offset, one texture or mesh width either way
const FILL_OFFSET_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;

/// Tri-planar hardness, the exponent weighting the three projections; `layer.fill.projection` clamps to the same range
const TRIPLANAR_HARDNESS_RANGE: std::ops::RangeInclusive<f32> = 1.0..=64.0;

/// A fill's projection kind, axis and hardness, then its collapsible UV transformations
fn projection_editor(app: &mut TextureCraftApp, ui: &mut Ui, projection: &Projection, command: &str, base: &Value, salt: &str) {
    let projection_label = projection.label();
    combo(ui, tr("Projection"), tr(projection_label), |ui| {
        for (id, label) in [
            ("uv", n_("UV projection")),
            ("triplanar", n_("Tri-planar projection")),
            ("planar", n_("Planar projection")),
            ("spherical", n_("Spherical projection")),
            ("cylindrical", n_("Cylindrical projection")),
        ] {
            if ui.selectable_label(projection_label == label, tr(label)).clicked() {
                app.run(command, with(base, json!({"projection": id})));
            }
        }
    });

    // Each edit sends only the fields it changed; the command keeps the others
    let mut edit = |fields: Value| {
        app.run(command, with(base, fields));
    };
    if let Projection::Planar { axis, .. } | Projection::Cylindrical { axis, .. } = projection
        && let Some(picked_axis) = axis_combo(ui, *axis)
    {
        edit(json!({"axis": picked_axis}));
    }
    if let Projection::Triplanar { sharpness, .. } = projection {
        let mut hardness = *sharpness;
        if let Some(merge) = slider_edit(ui, tr("Hardness"), &mut hardness, TRIPLANAR_HARDNESS_RANGE, 1) {
            edit(json!({"sharpness": hardness, "merge": merge}));
        }
    }

    // One id per fill so each keeps its own open or closed state
    section(ui, &format!("sec_fill_transform{salt}"), tr("UV transformations"), true, |ui| {
        projection_transform(ui, projection, salt, &mut edit);
    });
}

/// Width of the lock button between a pair of sliders
const PAIR_LOCK_WIDTH: f32 = 22.0;

/// Height of the lock button, which sits level with the sliders' tracks at the bottom of the pair
const PAIR_LOCK_HEIGHT: f32 = 18.0;

/// Size of the lock button's icon
const PAIR_LOCK_ICON_SIZE: f32 = 14.0;

/// Height of one slider: its value row over its track
const PAIR_SLIDER_HEIGHT: f32 = 34.0;

/// Two sliders side by side under `label` (across and down), locked together while `lock` is given and set (the second greyed); the pair once one is edited, with [`slider_edit`]'s `merge`
fn pair_sliders(
    ui: &mut Ui,
    label: &str,
    mut values: [f32; 2],
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
    lock: Option<&mut bool>,
) -> Option<([f32; 2], bool)> {
    let mut edited = None;
    ui.label(label);

    let locked = lock.as_deref().copied().unwrap_or(false);
    let lock_width = if lock.is_some() { PAIR_LOCK_WIDTH } else { ui.spacing().item_spacing.x };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), PAIR_SLIDER_HEIGHT), Sense::hover());
    let half = ((rect.width() - lock_width) / 2.0).max(1.0);
    let left = egui::Rect::from_min_size(rect.min, egui::vec2(half, rect.height()));
    let right = egui::Rect::from_min_size(egui::pos2(rect.right() - half, rect.top()), egui::vec2(half, rect.height()));
    let slot = |ui: &mut Ui, area: egui::Rect| ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));

    if let Some(merge) = slider_edit(&mut slot(ui, left), "", &mut values[0], range.clone(), decimals) {
        if locked {
            values[1] = values[0];
        }
        edited = Some(merge);
    }

    if let Some(lock) = lock {
        let middle =
            egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.bottom() - PAIR_LOCK_HEIGHT / 2.0), egui::vec2(PAIR_LOCK_WIDTH, PAIR_LOCK_HEIGHT));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(middle).layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight)));
        let icon = if *lock { ic::LOCK_SIMPLE } else { ic::LOCK_SIMPLE_OPEN };
        if icon_button(&mut child, icon, tr("Lock the two values together"), *lock, PAIR_LOCK_ICON_SIZE).clicked() {
            *lock = !*lock;
            if *lock && values[1] != values[0] {
                values[1] = values[0];
                edited = Some(false);
            }
        }
    }

    let mut second = slot(ui, right);
    second.add_enabled_ui(!locked, |ui| {
        if let Some(merge) = slider_edit(ui, "", &mut values[1], range.clone(), decimals) {
            edited = Some(merge);
        }
    });

    edited.map(|merge| (values, merge))
}

/// A projection's UV transformations: tiling across and down (locked together until unlocked), rotation and offset; a 3D projection's scale and world offset too
fn projection_transform(ui: &mut Ui, projection: &Projection, salt: &str, edit: &mut impl FnMut(Value)) {
    let (tiling, rotation, offset, offset_key) = match projection {
        Projection::Uv { scale, rotation, offset } => (*scale, *rotation, *offset, "offset"),
        Projection::Triplanar { uv, .. }
        | Projection::Planar { uv, .. }
        | Projection::Spherical { uv, .. }
        | Projection::Cylindrical { uv, .. }
        | Projection::Warp { uv, .. } => (uv.tiling, uv.rotation, uv.offset, "uvOffset"),
        Projection::UvTile => return,
    };

    let lock_id = ui.make_persistent_id(("fill_tiling_lock", salt));
    let mut locked = ui.data(|data| data.get_temp::<bool>(lock_id)).unwrap_or(tiling[0] == tiling[1]);
    let was_locked = locked;
    if let Some((edited, merge)) = pair_sliders(ui, tr("Tiling"), tiling, FILL_SCALE_RANGE, 2, Some(&mut locked)) {
        edit(json!({"tiling": edited, "merge": merge}));
    }
    if locked != was_locked {
        ui.data_mut(|data| data.insert_temp(lock_id, locked));
    }

    let mut rotation_degrees = rotation;
    if let Some(merge) = slider_edit(ui, tr("Rotation"), &mut rotation_degrees, FILL_ROTATION_DEGREES_RANGE, 0) {
        edit(json!({"rotation": rotation_degrees, "merge": merge}));
    }

    if let Some((edited, merge)) = pair_sliders(ui, tr("Offset"), offset, FILL_OFFSET_RANGE, 2, None) {
        edit(json!({offset_key: edited, "merge": merge}));
    }

    match projection {
        Projection::Triplanar { scale, offset, .. } | Projection::Planar { scale, offset, .. } => {
            world_scale(ui, *scale, edit);
            if let Some((edited_offset, merge)) = offset_sliders(ui, ["X", "Y", "Z"], *offset) {
                edit(json!({"offset": edited_offset, "merge": merge}));
            }
        }
        Projection::Spherical { scale, .. } | Projection::Cylindrical { scale, .. } => world_scale(ui, *scale, edit),
        Projection::Uv { .. } | Projection::UvTile | Projection::Warp { .. } => {}
    }
}

/// A 3D projection's scale: how many times its pattern fits across the mesh
fn world_scale(ui: &mut Ui, scale: f32, edit: &mut impl FnMut(Value)) {
    let mut scale = scale;
    if let Some(merge) = slider_edit(ui, tr("Scale"), &mut scale, FILL_SCALE_RANGE, 2) {
        edit(json!({"scale": scale, "merge": merge}));
    }
}

/// A slider per offset component ("Offset U", …): the whole offset once one is edited, with [`slider_edit`]'s `merge`
fn offset_sliders<const N: usize>(ui: &mut Ui, axes: [&str; N], mut values: [f32; N]) -> Option<([f32; N], bool)> {
    let mut edit = None;
    for (axis, value) in axes.iter().zip(values.iter_mut()) {
        edit = edit.or(slider_edit(ui, &trf("Offset {axis}", &[("axis", axis)]), value, FILL_OFFSET_RANGE, 2));
    }

    edit.map(|merge| (values, merge))
}

/// The axis a planar or cylindrical projection runs along: the one picked, if any
fn axis_combo(ui: &mut Ui, axis: u8) -> Option<u8> {
    let mut picked = None;
    combo(ui, tr("Axis"), ["X", "Y", "Z"].get(usize::from(axis)).copied().unwrap_or("Z"), |ui| {
        for (i, name) in (0u8..).zip(["X", "Y", "Z"]) {
            if ui.selectable_label(i == axis, name).clicked() && i != axis {
                picked = Some(i);
            }
        }
    });
    picked
}

/// The properties of effect `effect_id` of `layer`: each kind's own settings.
fn effect_properties(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer, effect_id: LayerId) {
    let tokens = Tokens::get(ui.ctx());
    let Some(effect) = app.session.doc.project.effect(effect_id).map(|(_, effect)| effect.clone()) else { return };

    let id = effect_id.0;

    // Property sheets keep the panel's own edges; only the rows not yet laid out by one get the legacy margin
    match &effect.kind {
        EffectKind::Generator(generator) => crate::generator_panel::generator_properties(app, ui, layer.id, id, generator),
        EffectKind::Levels(levels) => crate::levels_properties::levels_properties(app, ui, layer, effect_id, levels),
        EffectKind::Fill { source, projection, .. } => legacy_body(ui, |ui| {
            ui.label(caption(tr("Fill")));
            let anchors = effect_anchors_below(app, layer.id, effect_id);
            let name = match source {
                Source::Anchor { anchor, channel, .. } => anchor_label(app, *anchor, *channel),
                Source::Uniform(_) => tr("Uniform value").to_string(),
                Source::Procedural { pattern, .. } => tr(pattern.kind.label()).to_string(),
                Source::Image { name, .. } => trf("Image: {name}", &[("name", name)]),
            };

            let (rect, source_response) = ui.allocate_exact_size(vec2(ui.available_width(), crate::placed_fill_slot::SLOT_HEIGHT), Sense::click());
            source_response.widget_info(|| egui::WidgetInfo {
                current_text_value: Some(name.clone()),
                ..egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tr("Grayscale"))
            });
            let (ctx, popup) = (ui.ctx().clone(), ui.make_persistent_id(("fill_source_picker", id)));
            if source_response.clicked() {
                crate::resource_picker::toggle(&ctx, popup);
            }
            let highlighted = source_response.hovered() || egui::Popup::is_id_open(&ctx, popup);
            crate::placed_fill_slot::paint(ui, rect, (highlighted, source_response.has_focus()), (tr("Grayscale"), &name));
            let current = crate::fill_material_panel::source_choice(&app.session, source);
            let tabs = || crate::fill_material_panel::source_tabs(&app.session, &anchors, Color32::from_gray(204), false);
            let picked = crate::resource_picker::show(&source_response, popup, &mut app.thumbs, Some(&current), current != SourceChoice::Uniform, tabs);
            let anchor_channel = if let Source::Anchor { channel, .. } = source { *channel } else { Channel::Height };
            let params = match picked.filter(|choice| *choice != current) {
                Some(SourceChoice::Procedural(kind)) => Some(json!({"effect": id, "procedural": kind.label()})),
                Some(SourceChoice::Image(image)) => Some(json!({"effect": id, "image": image})),
                Some(SourceChoice::Uniform) => Some(json!({"effect": id, "value": 1.0})),
                Some(SourceChoice::Anchor(anchor)) => Some(json!({"effect": id, "anchor": anchor.0, "anchorChannel": anchor_channel.id()})),
                Some(SourceChoice::Import) => {
                    crate::menus::invoke(app, &ctx, "resource.import");
                    None
                }
                Some(SourceChoice::Alpha(_)) | None => None,
            };
            if let Some(params) = params {
                app.run("effect.set", params);
            }

            // The source selector accepts image drops from the Assets panel
            if let Some(name) = image_drop(ui, &source_response.interact(Sense::hover())) {
                app.run("effect.set", json!({"effect": id, "image": name}));
            }

            match source {
                Source::Uniform(color) => {
                    let mut value = color[0];
                    if let Some(merge) = slider_edit(ui, tr("Value"), &mut value, 0.0..=1.0, 2) {
                        app.run("effect.set", json!({"effect": id, "value": value, "merge": merge}));
                    }
                }
                Source::Anchor { anchor, .. } if anchor_in_mask(app, *anchor) => {
                    let note = if mask_anchor_enabled(app, *anchor) == Some(false) { tr("Off") } else { tr("A mask anchor reads as its mask's gray.") };
                    ui.label(RichText::new(note).color(tokens.text_dim));
                }
                Source::Anchor { channel, .. } => {
                    let channels = app.session.doc.project.texture_set(app.session.ts()).map(|set| set.channel_list()).unwrap_or_default();
                    combo(ui, tr("Channel"), tr(channel.label()), |ui| {
                        for option in channels {
                            if ui.selectable_label(option == *channel, tr(option.label())).clicked() {
                                app.run("effect.set", json!({"effect": id, "anchorChannel": option.id()}));
                            }
                        }
                    });
                    ui.label(RichText::new(tr("Colors read as gray. Add a levels or blur effect above to shape the result.")).color(tokens.text_dim));
                }
                Source::Image { .. } => {
                    projection_editor(app, ui, projection, "effect.set", &json!({"effect": id}), &format!("maskfill{id}"));
                }
                Source::Procedural { pattern, .. } => {
                    let target = crate::pattern_target::PatternTarget { command: "effect.set", base: json!({"effect": id}), salt: format!("mask_fill_{id}") };
                    crate::pattern_panel::pattern_properties(app, ui, &target, pattern);
                }
            }
            if anchors.is_empty() {
                ui.label(RichText::new(tr("Add an anchor point to a layer below to use its stack as a mask.")).color(tokens.text_dim));
            }
        }),
        EffectKind::Material(fill) => {
            let enabled: Vec<Channel> = fill.sources.keys().copied().collect();
            PropertySheet::show(ui, ("fill_effect_sheet", id), |sheet| {
                crate::fill_panel::fill_sheet(app, sheet, fill, &json!({"effect": id}), &enabled, layer.id);
            });
        }
        EffectKind::Anchor => crate::anchor_properties::anchor_properties(app, ui, effect_id, &effect.name),
        EffectKind::Paint => legacy_body(ui, |ui| {
            let hint = if layer.content_effects.iter().any(|content| content.id == effect_id) {
                tr("Paint on the mesh to edit the layer content.")
            } else {
                tr("Paint on the mesh to edit this mask.")
            };
            ui.label(RichText::new(hint).color(tokens.text_dim));
            if app.session.tool_usable(app.session.tool.tool) {
                tool_properties(app, ui);
            }
        }),
        EffectKind::Path(paths) => legacy_body(ui, |ui| {
            ui.label(
                RichText::new(tr("Paths stay editable: pick one to move its points or change its brush and material, and the sublayer repaints at once. Add paths with the Path tool."))
                    .color(tokens.text_dim),
            );
            let editing = app.session.tool.path.editing;
            for index in 0..paths.len() {
                ui.horizontal(|ui| {
                    let selected = editing == Some((effect_id, index));
                    if ui.selectable_label(selected, format!("{} {}", tr("Path"), index + 1)).on_hover_text(tr("Edit")).clicked() {
                        if selected {
                            app.run("tool.path", json!({"clear": true}));
                        } else {
                            app.run("path.edit", json!({"effect": id, "index": index}));
                        }
                    }
                    if icon_button(ui, ic::TRASH, tr("Remove"), false, ICON_BUTTON_SIZE).clicked() {
                        app.run("path.remove", json!({"effect": id, "index": index}));
                    }
                });
            }
            if app.session.tool_usable(app.session.tool.tool) {
                tool_properties(app, ui);
            }
        }),
        EffectKind::Filter(filter) => crate::filter_properties::filter_properties(app, ui, effect_id, filter),
        EffectKind::ColorSelection(selection) => crate::color_selection_editor::color_selection_properties(app, ui, effect_id, selection),
        EffectKind::Compare { op, value, tolerance, left, right, channel, hardness } => {
            let settings = crate::compare_settings::CompareSettings {
                op: *op,
                constant: *value,
                tolerance: *tolerance,
                left: *left,
                right: *right,
                channel: *channel,
                hardness: *hardness,
            };
            crate::compare_properties::compare_properties(app, ui, layer, effect_id, settings);
        }
    }
}

#[cfg(test)]
mod layer_layout_tests {
    use super::*;

    #[test]
    fn narrow_nested_rows_keep_thumbnail_and_secondary_controls_separate() {
        for width in [120.0, 180.0, 260.0, 400.0] {
            for depth in 0..=12 {
                for folder in [false, true] {
                    let row = Rect::from_min_size(pos2(100.0, 20.0), vec2(width, 48.0));
                    let (indent, size) = layer_thumbnail_layout(width, depth, folder, 36.0);
                    let content_left = row.left() + 7.0 + indent + 20.0 + if folder { 14.0 } else { 0.0 };
                    let content = Rect::from_min_size(pos2(content_left, row.top() + 6.0), vec2(size, size));
                    let mask = content.translate(vec2(size + 4.0, 0.0));
                    let (name, show_blend) = layer_name_layout(row, mask.right() + 8.0);

                    assert!(size >= 35.0, "minimum-width panels retain usable thumbnail targets");
                    assert!(row.contains_rect(content) && row.contains_rect(mask), "width={width}, depth={depth}, folder={folder}");
                    assert!(!content.intersects(mask));
                    if name.width() >= 20.0 {
                        assert!(row.contains_rect(name));
                        assert!(name.left() > mask.right());
                    }
                    if show_blend {
                        let blend_left = row.right() - RIGHT_COLUMN - 4.0;
                        assert!(name.right() + 4.0 <= blend_left);
                        assert!(mask.right() < blend_left);
                    }
                }
            }
        }
    }

    #[test]
    fn narrow_effect_rows_keep_the_eye_preview_and_name_inside() {
        for width in [120.0, 180.0, 260.0, 400.0] {
            for level in 0..=14 {
                for preview in [18.0, 21.6, 28.0] {
                    let indent = effect_row_indent(width, level, preview);
                    let eye_right = 7.0 + indent + 16.0;
                    let preview_left = 7.0 + indent + 22.0;
                    let name_left = preview_left + preview + 8.0;
                    assert!(eye_right < preview_left);
                    assert!(name_left + 20.0 <= width - 6.0 + 0.001);
                }
            }
        }
    }

    #[test]
    fn ordinary_rows_keep_preferred_sizes_and_secondary_controls() {
        let row = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 48.0));
        let (indent, size) = layer_thumbnail_layout(row.width(), 2, false, 36.0);
        let (_, show_blend) = layer_name_layout(row, 7.0 + indent + 20.0 + size * 2.0 + 4.0 + 8.0);

        assert_eq!(indent, 28.0);
        assert_eq!(size, 36.0);
        assert!(show_blend);
    }
}
