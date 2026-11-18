//! Panel bodies of the right column: Texture Sets, Layers, Texture Set Settings, Properties
//! (hosted as dock tabs by `docking`).

use egui::{Color32, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::compose::ThumbTarget;
use texturecraft_engine::core::{
    BakedMap, BlendMode, Channel, ChannelBlend, ChannelKind, Decal, DecalFlip, Effect, EffectKind, Fill, Filter, Layer, LayerId, LayerKind, LinkParam,
    MaterialLink, Projection, Source,
};
use texturecraft_engine::filter_lib::scalar_params;
use texturecraft_engine::paint::alpha_params::MAX_ALPHA_CONTRAST;
use texturecraft_engine::paint::{BrushAlpha, BrushSettings, alphas};
use texturecraft_engine::{CloneFrom, StackTarget, Tool};

use crate::TextureCraftApp;
use crate::filter_slot::SLOT_HEIGHT;
use crate::i18n::{n_, tr, trf};
use crate::layer_panel::{effect_preview_size, effect_row_height, layer_preview_size, layer_row_height};
use crate::shelf::{Asset, ShelfDrag};
use crate::state::PropsTab;
use crate::theme::{Tokens, caption};
use crate::toolbar::{MAX_BRUSH_SIZE, MIN_BRUSH_SIZE};
use crate::widgets::{chip, color_swatch, combo, dnd_release, elided, icon_button, section, section_with_reset, slider, slider_edit};

/// The filter Add filter adds: at its defaults it changes nothing, and its slot opens the picker to swap it
const NEW_FILTER: &str = "hsl";
const FILTER_SWATCH_WIDTH: f32 = 36.0;
const FILTER_HINT_SIZE: f32 = 10.5;
/// The most colors `effect.set {colors}` takes for a gradient map
const MAX_GRADIENT_STOPS: usize = 16;

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

/// The add-mask commands, for the selected layer or (`row`) the layer whose mask slot was clicked.
fn add_mask_menu(app: &mut TextureCraftApp, ui: &mut Ui, row: Option<u64>) {
    let enabled = row.is_some() || app.session.selected_layer().is_some();
    for (label, id) in
        [(tr("Add white mask"), "mask.add.white"), (tr("Add black mask"), "mask.add.black"), (tr("Add mask with color selection"), "mask.add.colorSelection")]
    {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            if let Some(layer) = row {
                app.run("layer.select", json!({ "layer": layer }));
            }
            app.run(id, Value::Null);
            ui.close();
        }
    }
}

pub(crate) fn layers(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());

    // Wrap in a narrow column so rows stay within the panel
    let project_open = app.session.project_open;
    ui.add_enabled_ui(project_open, |ui| {
        ui.horizontal_wrapped(|ui| {
            // Keep menu buttons as compact as the adjacent icon buttons
            ui.spacing_mut().button_padding.x = 4.0;
            ui.spacing_mut().item_spacing.x = TOOLBAR_SPACING;
            let channels = app.session.doc.project.texture_set(app.session.ts()).map(|t| t.channel_list()).unwrap_or_default();

            // The channel box takes what the six buttons leave, so the toolbar stays on one row in a narrow column
            let buttons = 2.0 * (MENU_BUTTON_WIDTH + TOOLBAR_SPACING) + 4.0 * (TOOLBAR_ICON_SIZE + TOOLBAR_SPACING);
            let channel_width = (ui.available_width() - buttons).clamp(CHANNEL_BOX_MIN_WIDTH, CHANNEL_BOX_WIDTH);
            ui.allocate_ui(vec2(channel_width, ICON_BUTTON_SIZE), |ui| {
                egui::ComboBox::from_id_salt("layer_channel").width(channel_width).truncate().selected_text(tr(app.ui.layer_channel.label())).show_ui(
                    ui,
                    |ui| {
                        for channel in channels {
                            ui.selectable_value(&mut app.ui.layer_channel, channel, tr(channel.label()));
                        }
                    },
                );
            });
            ui.menu_button(RichText::new(ic::MAGIC_WAND).size(14.0), |ui| {
                let selected = app.session.selected_layer().and_then(|id| app.session.doc.project.layers.get(&id)).cloned();
                match &selected {
                    Some(layer) => effect_menu(app, ui, layer),
                    None => {
                        ui.label(RichText::new(tr("Select a layer")).color(tokens.text_dim));
                    }
                }
            })
            .response
            .on_hover_text(tr("Add effect"));
            ui.menu_button(RichText::new(ic::SELECTION_BACKGROUND).size(14.0), |ui| add_mask_menu(app, ui, None)).response.on_hover_text(tr("Add mask"));
            if icon_button(ui, ic::PAINT_BRUSH, tr("Add paint layer"), false, TOOLBAR_ICON_SIZE).clicked() {
                app.run("layer.add.paint", Value::Null);
            }
            if icon_button(ui, ic::PAINT_BUCKET, tr("Add fill layer"), false, TOOLBAR_ICON_SIZE).clicked() {
                app.run("layer.add.fill", Value::Null);
            }
            if icon_button(ui, ic::FOLDER_SIMPLE, tr("Add folder"), false, TOOLBAR_ICON_SIZE).clicked() {
                app.run("layer.add.group", Value::Null);
            }
            if icon_button(ui, ic::TRASH, &crate::keymap::tip(&app.ui.keymap, tr("Remove layer"), "layer.remove"), false, TOOLBAR_ICON_SIZE).clicked() {
                app.run("layer.remove", Value::Null);
            }
        })
    });

    // The empty state has no layers: the toolbar stays, greyed
    if !project_open {
        return;
    }

    ui.add_space(2.0);
    let ts = app.session.ts();
    let stack: Vec<LayerId> = app.session.doc.project.texture_sets.get(ts).map(|t| t.stack.iter().rev().copied().collect()).unwrap_or_default();
    let now = ui.input(|i| i.time);
    app.layer_panel.begin(&app.session, now);
    egui::ScrollArea::vertical().id_salt("layers_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let mut count = 0;
        for id in stack {
            layer_tree(app, ui, id, 0, &mut count);
        }

        // Empty space keeps the selection and drops below the rows
        let free = ui.available_height();
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), if free.is_finite() { free.max(24.0) } else { 24.0 }), Sense::hover());
        if let Some(dragged) = dnd_release::<u64>(&resp) {
            let bottom = app.session.doc.project.texture_sets.get(ts).and_then(|t| t.stack.first().copied());
            if let Some(layers) = selection_with(app, LayerId(*dragged)) {
                app.run("layer.move", json!({"layers": layers, "index": 0}));
            } else if bottom.map(|b| b.0) != Some(*dragged) {
                app.run("layer.move", json!({"layer": *dragged, "index": 0}));
            }
        }
        match dnd_release::<ShelfDrag>(&resp).map(|d| d.asset.clone()) {
            Some(Asset::Material(m)) => {
                app.run("material.apply", json!({"material": m, "index": 0}));
            }
            Some(Asset::Library(path)) => {
                app.run("material.load", json!({"path": path, "index": 0}));
            }
            _ => {}
        }
        let material = resp.dnd_hover_payload::<ShelfDrag>().is_some_and(|d| d.asset.is_material());
        if resp.dnd_hover_payload::<u64>().is_some() || material {
            ui.painter().line_segment([rect.left_top(), rect.right_top()], Stroke::new(2.0, tokens.accent));
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

/// "Add effect" items for `layer`: content when it is the selected target, else the mask
fn effect_menu(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer) {
    let content = !layer.is_group() && app.session.selected_layer() == Some(layer.id) && targets_content(app);
    let target = if content { "content" } else { "mask" };
    ui.label(caption(if content { tr("Add to the layer content") } else { tr("Add to the mask") }));

    // Generators and compare masks always go to the mask; anchor points go where the others do
    let items = [
        (tr("Add generator"), "effect.add.generator", json!({})),
        (tr("Add paint"), "effect.add.paint", json!({"target": target})),
        (tr("Add fill"), "effect.add.fill", json!({"target": target})),
        (tr("Add levels"), "effect.add.levels", json!({"target": target})),
        (tr("Add compare mask"), "effect.add.compare", json!({})),
        (tr("Add filter"), "effect.add.filter", json!({"target": target, "filter": NEW_FILTER})),
        (tr("Add anchor point"), "effect.add.anchor", json!({"target": target})),
    ];
    for (label, command, params) in items {
        if !ui.button(label).clicked() {
            continue;
        }

        if app.session.selected_layer() != Some(layer.id) {
            app.run("layer.select", json!({"layer": layer.id.0}));
        }

        let added = app.run(command, with(&params, json!({"layer": layer.id.0})));

        // The neutral filter is a placeholder: its slot opens the picker to choose the real one
        if command == "effect.add.filter"
            && let Some(effect) = added.as_ref().and_then(|result| result["effect"].as_u64())
        {
            app.ui.dock.focus(crate::docking::Panel::Properties);
            crate::filter_slot::open_picker_when_shown(ui.ctx(), effect);
        }

        ui.close();
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
/// Width of the Layers toolbar's channel box when there is room, and the least it shrinks to.
const CHANNEL_BOX_WIDTH: f32 = 86.0;
const CHANNEL_BOX_MIN_WIDTH: f32 = 40.0;
/// Gap between the Layers toolbar's buttons.
const TOOLBAR_SPACING: f32 = 2.0;
/// Edge of the toolbar's icon buttons, and the width of its two menu buttons.
const TOOLBAR_ICON_SIZE: f32 = 20.0;
const MENU_BUTTON_WIDTH: f32 = 22.0;
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

    // A line under the layer and its effects separates it from the next layer.
    let tokens = Tokens::get(ui.ctx());
    let available = ui.available_rect_before_wrap();
    ui.painter().hline(available.x_range(), available.top() - 0.5, Stroke::new(1.0, tokens.separator));
    if let LayerKind::Group { children, collapsed } = &layer.kind
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
    let several = selection_with(app, layer.id);
    let content_target = selected && targets_content(app);
    let preferred_size = layer_preview_size(app);
    let row_height = layer_row_height(preferred_size);
    let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click_and_drag());
    row.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListItem, true, highlighted, &layer.name));

    // Background, folder lines, the selected rows' accent edge and the colour tag
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
    painter.rect_stroke(thumbnail, THUMBNAIL_ROUNDING, target_stroke(&tokens, content_target), egui::StrokeKind::Outside);
    let content_response = ui.interact(thumbnail, ui.scope_id().with(("content", layer.id.0)), Sense::click()).on_hover_text(tr("Select the layer content"));
    if content_response.clicked() {
        let modifiers = ui.input(|input| input.modifiers);
        click_layer(app, modifiers, layer.id.0, Some("content"));
    }
    x = thumbnail.right() + THUMBNAIL_SPACING;

    // An instance draws its source's content, blending and (unless it has its own) mask
    let project = &app.session.doc.project;
    let source = layer.instance_source().map(|source| {
        let set = project.locate(source).and_then(|(set, _, _)| project.texture_sets.get(set)).map(|set| set.name.as_str()).unwrap_or("?");
        project.layers.get(&source).map_or_else(|| tr("missing source").to_string(), |source_layer| format!("{} · {set}", source_layer.name))
    });
    let mask = project.layers.get(&project.mask_owner(layer.id)).and_then(|owner| owner.mask.clone());
    let blend = project.content_layer(layer.id).and_then(|content| content.blend(channel));

    // Mask thumbnail, selectable on its own; or the slot that adds one
    let mask_rect;
    if let Some(mask) = &mask {
        let mask_target = selected && !targets_content(app);
        mask_rect = Rect::from_min_size(pos2(x, rect.center().y - thumbnail_size * 0.5), vec2(thumbnail_size, thumbnail_size));
        if !thumb_box(ui, app, mask_rect, ThumbTarget::Mask { layer: layer.id }, !layer.visible || !mask.enabled) {
            let white = mask.background == texturecraft_engine::core::MaskBackground::White;
            painter.rect_filled(mask_rect, THUMBNAIL_ROUNDING, Color32::from_gray(if white { WHITE_MASK_GRAY } else { BLACK_MASK_GRAY }));
            if !mask.effects.is_empty() {
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
        if !mask.enabled {
            painter.line_segment([mask_rect.left_top(), mask_rect.right_bottom()], Stroke::new(HIDDEN_STRIKE_WIDTH, tokens.danger));
        }
        let mask_response = ui
            .interact(mask_rect, ui.scope_id().with(("mask", layer.id.0)), Sense::click())
            .on_hover_text(tr("Select the mask (Alt+click: show it in the viewport)"));
        if mask_response.clicked() {
            app.run("layer.select", json!({"layer": layer.id.0, "target": "mask"}));
            if ui.input(|input| input.modifiers.alt) {
                let showing = app.session.view == texturecraft_engine::ViewMode::Mask(layer.id);
                let view = if showing { json!({"mode": "material"}) } else { json!({"mode": "mask", "layer": layer.id.0}) };
                app.run("viewport.view", view);
            }
        }
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
        egui::Popup::menu(&slot).show(|ui| add_mask_menu(app, ui, Some(layer.id.0)));
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
        let effects = layer.content_effects.len() + mask.as_ref().map_or(0, |mask| mask.effects.len());
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
    let in_drag = |dragged: u64| dragged == layer.id.0 || several.as_ref().is_some_and(|layers| layers.contains(&dragged));
    let moving = row.dnd_hover_payload::<u64>().is_some_and(|dragged| !in_drag(*dragged));
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
        && !in_drag(*dragged)
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
    row.context_menu(|ui| layer_context_menu(app, ui, layer));
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
        let (mode, opacity) =
            blend_controls(ui, id, (mode_rect, opacity_rect), (effect.blend, effect.opacity), (tr("Blending mode of the effect"), &effect.name));
        if let Some(mode) = mode {
            app.run("effect.set", json!({"effect": effect_id.0, "blend": format!("{mode:?}")}));
        }
        if let Some((opacity, merge)) = opacity {
            app.run("effect.set", json!({"effect": effect_id.0, "opacity": opacity, "merge": merge}));
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

fn layer_context_menu(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer) {
    let tk = Tokens::get(ui.ctx());
    ui.set_min_width(230.0);
    let lid = layer.id.0;
    let keymap = app.ui.keymap.clone();

    // On one of several selected layers, remove, duplicate and group act on all of them
    let several = selection_with(app, layer.id);
    let run = |app: &mut TextureCraftApp, ui: &mut Ui, label: &str, id: &str, p: Value, enabled: bool| {
        let mut b = egui::Button::new(label);
        if let Some(sc) = crate::keymap::shortcut_of(&keymap, id) {
            b = b.shortcut_text(sc);
        }
        if ui.add_enabled(enabled, b).clicked() {
            let batch = matches!(id, "layer.remove" | "layer.duplicate" | "layer.group");
            match several.as_ref().filter(|_| batch) {
                Some(layers) => app.run(id, json!({"layers": layers})),
                None => {
                    app.run("layer.select", json!({"layer": lid}));
                    app.run(id, p)
                }
            };
            ui.close();
        }
    };

    // An instance edits its source mask until it gets its own
    let project = &app.session.doc.project;
    let has_mask = project.layers.get(&project.mask_owner(layer.id)).is_some_and(|l| l.mask.is_some());

    // A switched-off mask has no values to export
    let mask_on = project.layers.get(&project.mask_owner(layer.id)).is_some_and(|l| l.mask.as_ref().is_some_and(|m| m.enabled));
    run(app, ui, tr("Add white mask"), "mask.add.white", json!({"layer": lid}), layer.mask.is_none());
    run(app, ui, tr("Add black mask"), "mask.add.black", json!({"layer": lid}), layer.mask.is_none());
    run(app, ui, tr("Add mask with color selection"), "mask.add.colorSelection", json!({"layer": lid}), true);
    ui.separator();
    run(app, ui, tr("Toggle mask"), "mask.toggle", json!({"layer": lid}), has_mask);
    run(app, ui, tr("Invert mask background"), "mask.invertBackground", json!({"layer": lid}), has_mask);
    run(app, ui, tr("Invert mask"), "mask.invert", json!({"layer": lid}), has_mask);
    run(app, ui, tr("Clear mask"), "mask.clear", json!({"layer": lid}), has_mask);
    run(app, ui, tr("Remove mask"), "mask.remove", json!({"layer": lid}), has_mask);
    #[cfg(not(target_arch = "wasm32"))]
    if ui.add_enabled(mask_on, egui::Button::new(tr("Export mask to file..."))).clicked() {
        export_mask_file(app, layer);
        ui.close();
    }
    if ui.add_enabled(mask_on, egui::Button::new(tr("Copy mask to clipboard"))).clicked() {
        copy_mask(app, ui.ctx(), layer.id);
        ui.close();
    }
    geometry_mask_menu(app, ui, layer);
    ui.separator();
    solo_menu(app, ui, layer.id);
    ui.separator();
    run(app, ui, tr("Flatten layer"), "layer.flatten", json!({"layer": lid}), !layer.is_group());
    #[cfg(not(target_arch = "wasm32"))]
    if ui.button(tr("Create smart material")).clicked() {
        let file = crate::library_dir().join(format!("{}.tcmat", texturecraft_engine::formats::sanitize_resource_name(&layer.name)));
        if let Some(r) = app.run("material.save", json!({"layer": lid, "path": file.display().to_string(), "name": layer.name})) {
            let p = r.get("path").and_then(|p| p.as_str()).unwrap_or_default().to_string();
            app.toast(trf("Smart material saved to {path}", &[("path", &p)]), false);
            app.library = None;
        }
        ui.close();
    }
    #[cfg(not(target_arch = "wasm32"))]
    if ui.add_enabled(has_mask, egui::Button::new(tr("Create smart mask"))).clicked() {
        app.run("layer.select", json!({"layer": lid}));
        crate::smart_mask_menu::save(app, layer, &crate::library_dir());
        ui.close();
    }
    ui.separator();
    run(app, ui, tr("Cut layer(s)"), "layer.cut", json!({"layer": lid}), true);
    run(app, ui, tr("Copy layer(s)"), "layer.copy", json!({"layer": lid}), true);
    let paste_layer = crate::menus::enabled(app, "layer.paste");
    let paste_effect = texturecraft_engine::effect_clipboard::clipboard_effect(&app.session).is_some();
    run(app, ui, tr("Paste layer(s)"), "layer.paste", json!({}), paste_layer);
    run(app, ui, tr("Paste effect"), "effect.paste", json!({}), paste_effect);
    let paste_instance = crate::menus::enabled(app, "layer.pasteInstance");
    run(app, ui, tr("Paste as instance"), "layer.pasteInstance", json!({}), paste_instance);
    run(app, ui, tr("Duplicate layer(s)"), "layer.duplicate", json!({"layer": lid}), true);
    run(app, ui, tr("Instantiate across texture sets"), "layer.instantiateAll", json!({"layer": lid}), app.session.set_count() > 1);
    run(app, ui, tr("Copy to all texture sets"), "layer.instantiateAll", json!({"layer": lid, "copy": true}), app.session.set_count() > 1);
    run(app, ui, tr("Remove layer(s)"), "layer.remove", json!({"layer": lid}), true);
    run(app, ui, tr("Group layer(s)"), "layer.group", json!({"layer": lid}), true);
    blending_options_menu(app, ui, layer);
    ui.separator();
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, tk.text_dim), egui::StrokeKind::Inside);
        ui.painter().line_segment([r.left_bottom(), r.right_top()], Stroke::new(1.0, tk.danger));
        if resp.clicked() {
            app.run("layer.colorTag", json!({"layer": lid}));
        }
        for (i, c) in TAG_COLORS.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
            ui.painter().rect_filled(r, 2.0, *c);
            if resp.clicked() {
                app.run("layer.colorTag", json!({"layer": lid, "tag": i}));
            }
        }
    });
    ui.separator();
    effect_menu(app, ui, layer);
}

// ---------------------------------------------------------------------------------------------
// Texture set settings

pub(crate) fn texture_set_settings(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tk = Tokens::get(ui.ctx());
    let ts = app.session.ts();
    let Some(set) = app.session.doc.project.texture_sets.get(ts).cloned() else { return };
    egui::ScrollArea::vertical().id_salt("tss").show(ui, |ui| {
        ui.label(caption(tr("General properties")));
        egui::Grid::new("tss_grid").num_columns(2).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
            ui.label(tr("Name"));
            ui.label(RichText::new(&set.name).color(tk.text_strong));
            ui.end_row();
            ui.label(tr("Size"));
            egui::ComboBox::from_id_salt("ts_size").selected_text(format!("{}", set.resolution)).show_ui(ui, |ui| {
                for r in [256u32, 512, 1024, 2048, 4096, 8192] {
                    if ui.selectable_label(r == set.resolution, format!("{r}")).clicked() {
                        app.start_resize(ts, r);
                    }
                }
            });
            ui.end_row();
            ui.label(tr("UV tiles (UDIM)"));
            let mut tiled = !set.uv_tiles.is_empty();
            if ui.checkbox(&mut tiled, "").changed() {
                app.run("textureSet.uvTiles", json!({"ts": ts, "enabled": tiled}));
            }
            ui.end_row();
            // Each tile's own size (per-tile resolution).
            for &udim in &set.uv_tiles {
                let r = set.tile_resolution(udim);
                ui.label(RichText::new(format!("   {udim}")).color(tk.text_dim));
                egui::ComboBox::from_id_salt(("ts_tile", udim)).selected_text(format!("{r}")).show_ui(ui, |ui| {
                    for v in [256u32, 512, 1024, 2048, 4096, 8192] {
                        if ui.selectable_label(v == r, format!("{v}")).clicked() {
                            app.run("textureSet.tile.resolution", json!({"ts": ts, "tile": udim, "resolution": v}));
                        }
                    }
                });
                ui.end_row();
            }
            ui.label(tr("Shader instance"));
            ui.label(tr("Main shader"));
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(caption(tr("Channels")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(tr("Add channel"), |ui| {
                    for c in Channel::ALL {
                        if !set.has_channel(c) && ui.button(tr(c.label())).clicked() {
                            app.run("textureSet.channel.add", json!({"ts": ts, "channel": c.id()}));
                            ui.close();
                        }
                    }
                });
            });
        });
        for (c, f) in &set.channels {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tr(c.label())).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if *c != Channel::BaseColor && icon_button(ui, ic::X, tr("Remove channel"), false, 18.0).clicked() {
                        app.run("textureSet.channel.remove", json!({"ts": ts, "channel": c.id()}));
                    }
                    ui.label(RichText::new(f.label()).color(tk.text_dim));
                });
            });
        }
        ui.add_space(6.0);
        let mut hs = set.height_strength;
        if let Some(merge) = slider_edit(ui, tr("Height to normal strength"), &mut hs, 0.0..=10.0, 2) {
            app.run("textureSet.settings", json!({"ts": ts, "heightStrength": hs, "merge": merge}));
        }
        let directx = app.session.doc.project.normal_format == texturecraft_engine::core::NormalFormat::DirectX;
        combo(ui, tr("Normal map format"), if directx { "DirectX" } else { "OpenGL" }, |ui| {
            for (id, label, on) in [("openGl", "OpenGL", !directx), ("directX", "DirectX", directx)] {
                if ui.selectable_label(on, label).clicked() && !on {
                    app.run("textureSet.settings", json!({"ts": ts, "normalFormat": id}));
                }
            }
        })
        .on_hover_text(tr(
            "For the whole project: how normal map images set as mesh maps are read (set them again after a change), and the default for export",
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(caption(tr("Mesh maps")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tr("Bake Mesh Maps")).clicked() {
                    app.ui.mode = crate::state::Mode::Baking;
                }
            });
        });
        for m in BakedMap::ALL {
            let present = app.session.doc.baked.get(&ts).is_some_and(|b| b.maps.contains_key(&m));
            let row = ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                ui.painter().rect_filled(r, 2.0, if present { mesh_map_color(m) } else { tk.button });
                ui.vertical(|ui| {
                    ui.label(RichText::new(tr(crate::i18n::baked_map_label(m))).strong());
                    ui.label(
                        RichText::new(if present { format!("{}_{}", set.name, m.id()) } else { tr("Missing (bake to create)").into() })
                            .size(10.5)
                            .color(tk.text_dim),
                    );
                });
                // An image resource instead of a bake, or no map.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.menu_button(RichText::new(ic::DOTS_THREE).size(13.0), |ui| {
                        if let Some(name) = image_menu(app, ui) {
                            app.run("textureSet.meshMap.set", json!({"ts": ts, "map": m.id(), "image": name}));
                        }
                        ui.separator();
                        if ui.add_enabled(present, egui::Button::new(tr("Clear"))).clicked() {
                            app.run("textureSet.meshMap.clear", json!({"ts": ts, "map": m.id()}));
                            ui.close();
                        }
                    });
                });
            });
            // A shelf image dropped on the row becomes this mesh map.
            let resp = row.response.interact(Sense::hover()).on_hover_text(tr("Drop an image from the Assets panel here to use it as this mesh map"));
            if let Some(name) = image_drop(ui, &resp) {
                app.run("textureSet.meshMap.set", json!({"ts": ts, "map": m.id(), "image": name}));
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
            let image = ImageRes { width: 32, height: 32, rgba: vec![[index as f32 / image_count as f32, 0.5, 0.25, 1.0]; 32 * 32], srgb: false };
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

/// An image from the Assets panel dropped on `resp`, outlined while one hovers it. Other assets
/// are left for the drop targets around it.
pub(crate) fn image_drop(ui: &Ui, resp: &egui::Response) -> Option<String> {
    if !resp.dnd_hover_payload::<ShelfDrag>().is_some_and(|d| matches!(d.asset, Asset::Texture(_))) {
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
        (Some(_), StackTarget::Effect(effect)) => project.effect(effect).map(|(_, found)| found.name.clone()).unwrap_or_default(),
        (Some(layer), StackTarget::Content) if matches!(layer.kind, LayerKind::Fill(_)) => tr("Fill").to_string(),
        (Some(layer), StackTarget::Content) if layer.is_group() => tr("Folder").to_string(),
        (Some(layer), StackTarget::Content) if layer.instance_source().is_some() => tr("Instance").to_string(),
        (Some(_), StackTarget::Mask) if !tool_usable => tr("Mask").to_string(),
        (None, _) if !tool_usable => return tr("Properties").to_string(),
        _ => tr(tool.label()).to_string(),
    };
    format!("{} - {what}", tr("Properties"))
}

pub(crate) fn properties(app: &mut TextureCraftApp, ui: &mut Ui) {
    // A floating scroll bar would cover the right-aligned slider values and section reset icons
    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
    egui::ScrollArea::vertical().id_salt("props").auto_shrink([false, false]).show(ui, |ui| properties_contents(app, ui));
}

/// What the Properties panel edits: polygon fill's own settings, the selected effect, fill, folder, instance or mask, else the tool (or a hint to select a layer when it can't act)
pub(crate) fn properties_contents(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    if !app.session.project_open {
        ui.label(RichText::new(tr("No properties found")).color(tokens.text_dim));
        return;
    }

    let selected = app.session.selected_layer().and_then(|id| app.session.doc.project.layers.get(&id).cloned());
    let tool = app.session.tool.tool;
    let tool_usable = app.session.tool_usable(tool);

    match (&selected, app.session.target) {
        // Polygon fill shows its own settings and says what it fills into, wherever it can fill
        _ if tool == Tool::PolygonFill && tool_usable => polygon_fill_properties(app, ui),
        (Some(layer), StackTarget::Effect(effect)) => effect_properties(app, ui, layer, effect),
        (Some(layer), StackTarget::Content) if matches!(layer.kind, LayerKind::Fill(_)) => fill_properties(app, ui, layer),
        (Some(layer), StackTarget::Content) if layer.is_group() => {
            ui.label(RichText::new(tr("A folder blends its children (pass-through). Set per-channel opacity in the layer rows.")).color(tokens.text_dim));
        }
        (Some(layer), StackTarget::Content) if layer.instance_source().is_some() => instance_properties(app, ui, layer),
        (Some(layer), StackTarget::Mask) => {
            height_mask_section(app, ui, layer);
            if tool_usable {
                ui.separator();
                tool_properties(app, ui);
            }
        }
        (None, _) if !tool_usable => {
            ui.label(RichText::new(tr("Select a layer to paint on")).color(tokens.text_dim));
        }
        _ => tool_properties(app, ui),
    }
}

/// An instance draws its source: point there for edits.
fn instance_properties(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer) {
    let tk = Tokens::get(ui.ctx());
    let Some(src) = layer.instance_source() else { return };
    let p = &app.session.doc.project;
    let name = p.layers.get(&src).map_or_else(|| tr("a missing layer").to_string(), |l| format!("\"{}\"", l.name));
    let set = p.locate(src).and_then(|(ts, _, _)| p.texture_sets.get(ts)).map(|t| t.name.clone()).unwrap_or_default();
    ui.label(caption(tr("Instance")));
    ui.label(
        RichText::new(format!(
            "{} {}",
            ic::LINK,
            trf("Draws {name} of texture set \"{set}\". Edits to the source show in every instance.", &[("name", &name), ("set", &set)])
        ))
        .color(tk.text_dim),
    );
    if ui.button(tr("Select source")).clicked() {
        app.run("layer.select", json!({"layer": src.0}));
    }
}

/// Mask with height: the layer's height shapes its mask transitions.
fn height_mask_section(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer) {
    let id = layer.id.0;
    let hm = app.session.doc.project.content_layer(layer.id).map(|l| l.height_mask).unwrap_or_default();
    section(ui, "sec_height_mask", tr("Mask with height"), true, |ui| {
        let mut on = hm.enabled;
        if ui
            .checkbox(&mut on, tr("Combine with the layer's height"))
            .on_hover_text(tr("Partial mask values follow the layer's height: high parts show first"))
            .changed()
        {
            app.run("layer.set", json!({"layer": id, "heightMask": {"enabled": on}}));
        }
        ui.add_enabled_ui(hm.enabled, |ui| {
            let (mut contrast, mut offset) = (hm.contrast, hm.offset);
            if let Some(merge) = slider_edit(ui, tr("Contrast"), &mut contrast, 0.0..=16.0, 2) {
                app.run("layer.set", json!({"layer": id, "heightMask": {"contrast": contrast}, "merge": merge}));
            }
            if let Some(merge) = slider_edit(ui, tr("Height offset"), &mut offset, -1.0..=1.0, 2) {
                app.run("layer.set", json!({"layer": id, "heightMask": {"offset": offset}, "merge": merge}));
            }
        });
    });
}

fn tool_properties(app: &mut TextureCraftApp, ui: &mut Ui) {
    let tokens = Tokens::get(ui.ctx());
    // Brush preview: a stroke and a sphere with the tool's material
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 88.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, tokens.viewport);
    let half_width = rect.width() * 0.5;
    let brush = &app.session.tool.brush;
    let mut points = Vec::new();
    for step in 0..=40 {
        let fraction = step as f32 / 40.0;
        points.push(pos2(rect.left() + 14.0 + fraction * (half_width - 28.0), rect.center().y + (fraction * 9.0).sin() * 18.0));
    }

    let stroke_width = (brush.size / 0.25 * 60.0).clamp(2.0, 30.0);
    let alpha = (brush.flow * brush.opacity * 255.0) as u8;
    painter.add(egui::Shape::line(points, Stroke::new(stroke_width, Color32::from_white_alpha(alpha.max(30)))));
    let base_color = app.session.tool.values.get(&Channel::BaseColor).copied().unwrap_or([0.8; 3]);
    crate::thumbs::paint_sphere(painter, pos2(rect.left() + half_width * 1.5, rect.center().y), 36.0, base_color);
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        let button_size = ui.spacing().interact_size.y;
        for (tab, icon, tip) in
            [(PropsTab::Brush, ic::PAINT_BRUSH, tr("Brush")), (PropsTab::Alpha, ic::CIRCLE_HALF, tr("Alpha")), (PropsTab::Material, ic::SPHERE, tr("Material"))]
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
        PropsTab::Material => material_section(app, ui),
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
                app.session.tool.mask_value = gray;
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

/// The tool's material: channel chips, then each enabled channel's value (or the mask gray while painting a mask)
fn material_controls(app: &mut TextureCraftApp, ui: &mut Ui) {
    use texturecraft_engine::core::ChannelKind;

    let tokens = Tokens::get(ui.ctx());
    let channels = material_channel_chips(app, ui);
    ui.add_space(4.0);

    if !targets_content(app) {
        let mut gray = app.session.tool.mask_value;
        if slider(ui, tr("Mask value (gray)"), &mut gray, 0.0..=1.0, 2).changed() {
            app.session.tool.mask_value = gray;
        }
        return;
    }

    for channel in channels {
        if !app.session.tool.enabled.contains(&channel) {
            continue;
        }

        let mut value = app.session.tool.values.get(&channel).copied().unwrap_or(channel.default_value());
        match channel.kind() {
            ChannelKind::Color => {
                ui.horizontal(|ui| {
                    ui.label(tr(channel.label()));
                    if color_swatch(ui, &format!("tool_{}", channel.id()), &mut value, ui.available_width() - 4.0) {
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
    if let Some(link) = &fill.link {
        link_properties(app, ui, layer.id, link);
        ui.add_space(6.0);
    }
    let on: Vec<Channel> = layer.channels.keys().copied().collect();
    ui.label(caption(tr("Fill")));
    fill_editor(app, ui, fill, &json!({"layer": layer.id.0}), &on, layer.id);
}

/// A MaterialCraft material's presets and exposed inputs; edits store the value and re-cook the
/// maps on a worker thread (desktop only: cooking runs MaterialCraft's command line).
fn link_properties(app: &mut TextureCraftApp, ui: &mut Ui, layer: LayerId, link: &MaterialLink) {
    let tk = Tokens::get(ui.ctx());
    let base = json!({"layer": layer.0, "cook": false});
    ui.label(caption(tr("MaterialCraft material")));
    ui.horizontal(|ui| {
        ui.label(RichText::new(&link.name).strong());
        ui.label(RichText::new(trf("Cooked at {size} px", &[("size", &link.size)])).size(10.0).color(tk.text_dim));
        if app.link_cooks.iter().any(|c| c.layer == layer) {
            ui.spinner();
        }
    });
    let editable = cfg!(not(target_arch = "wasm32"));
    ui.add_enabled_ui(editable, |ui| {
        if !link.presets.is_empty() {
            combo(ui, tr("Preset"), "", |ui| {
                for p in &link.presets {
                    if ui.selectable_label(false, &p.name).clicked() && app.run("material.link.preset", with(&base, json!({"name": p.name}))).is_some() {
                        app.cook_link(layer);
                    }
                }
            });
        }
        let mut group = "";
        for p in &link.params {
            if p.group != group && !p.group.is_empty() {
                ui.label(RichText::new(p.group.to_uppercase()).size(10.0).strong().color(tk.text_dim));
            }
            group = &p.group;
            if let Some(v) = link_param_editor(ui, layer, p)
                && app.run("material.link.set", with(&base, json!({"id": p.id, "value": v}))).is_some()
            {
                app.cook_link(layer);
            }
        }
    });
    if !editable {
        ui.label(RichText::new(tr("Parameters change in the desktop app, with MaterialCraft installed")).size(10.0).color(tk.text_dim));
    }
    #[cfg(not(target_arch = "wasm32"))]
    ui.horizontal(|ui| {
        if ui.button(tr("Re-cook")).clicked() {
            app.cook_link(layer);
        }
        if ui.button(tr("Open in MaterialCraft")).clicked() {
            let ext = texturecraft_engine::link_cmds::source_extension(&link.package);
            let path = std::env::temp_dir().join(format!("{}.{ext}", texturecraft_engine::formats::export::sanitize_file_stem(&link.name)));
            let opened = app
                .session
                .link_package(layer)
                .and_then(|b| std::fs::write(&path, b.as_slice()).map_err(|e| texturecraft_engine::EngineError::Other(e.to_string())))
                .and_then(|()| texturecraft_engine::link_cmds::open_in_materialcraft(&path));
            match opened {
                Ok(()) => app.toast(tr("Opened in MaterialCraft: use File > Send to TextureCraft there to bring your changes back"), false),
                Err(e) => app.toast(e.to_string(), true),
            }
        }
    });
}

/// The editor of one linked-material input; returns its new value when an edit completes.
fn link_param_editor(ui: &mut Ui, layer: LayerId, p: &LinkParam) -> Option<Value> {
    let t = p.default.get("t").and_then(Value::as_str).unwrap_or("");
    let v = p.value.get("v").unwrap_or(&Value::Null);
    let num = |x: &Value| x.as_f64().unwrap_or(0.0) as f32;
    let (lo, hi) = if p.max > p.min { (p.min, p.max) } else { (0.0, 1.0) };
    // A slider edit completes when the drag ends, on a click or a typed value.
    let edit = |ui: &mut Ui, label: &str, x: f32, decimals: usize| {
        let mut y = x;
        let r = slider(ui, label, &mut y, lo.min(x)..=hi.max(x), decimals);
        (r.drag_stopped() || (!r.dragged() && y != x)).then_some(y)
    };
    match t {
        "bool" => {
            let mut b = v.as_bool().unwrap_or(num(v) != 0.0);
            ui.checkbox(&mut b, &p.label).changed().then(|| json!(b))
        }
        "enum" if !p.options.is_empty() => {
            let cur = num(v).max(0.0) as usize;
            let mut pick = None;
            combo(ui, &p.label, p.options.get(cur).map_or("", String::as_str), |ui| {
                for (i, o) in p.options.iter().enumerate() {
                    if ui.selectable_label(i == cur, o).clicked() {
                        pick = Some(json!(i));
                    }
                }
            });
            pick
        }
        "float" | "int" | "enum" => edit(ui, &p.label, num(v), if t == "float" { 2 } else { 0 }).map(|y| json!(y)),
        "color" => {
            let a = v.as_array().map(|a| a.iter().map(num).collect::<Vec<f32>>()).unwrap_or_default();
            let mut rgb = [0, 1, 2].map(|i| a.get(i).copied().unwrap_or(1.0));
            let mut out = None;
            ui.horizontal(|ui| {
                ui.label(&p.label);
                if color_swatch(ui, &format!("link_{}_{}", layer.0, p.id), &mut rgb, ui.available_width() - 4.0) {
                    out = Some(json!([rgb[0], rgb[1], rgb[2], a.get(3).copied().unwrap_or(1.0)]));
                }
            });
            out
        }
        _ => {
            // Vectors: one slider per component.
            let mut a: Vec<f32> = v.as_array().map(|a| a.iter().map(num).collect()).unwrap_or_default();
            let decimals = if t == "int2" { 0 } else { 2 };
            let mut changed = false;
            for (x, axis) in a.iter_mut().zip(["x", "y", "z", "w"]) {
                if let Some(y) = edit(ui, &format!("{} {axis}", p.label), *x, decimals) {
                    *x = y;
                    changed = true;
                }
            }
            changed.then(|| json!(a))
        }
    }
}

/// `base` with the fields of `extra` added
pub(crate) fn with(base: &Value, extra: Value) -> Value {
    let mut params = base.clone();
    if let (Some(fields), Value::Object(extra)) = (params.as_object_mut(), extra) {
        fields.extend(extra);
    }
    params
}

/// Anchor points below layer `reader` in its texture set: (effect id, "layer / anchor" label).
pub(crate) fn anchors_below(app: &TextureCraftApp, reader: LayerId) -> Vec<(LayerId, String)> {
    let project = &app.session.doc.project;
    let Some((texture_set, _, _)) = project.locate(reader) else { return Vec::new() };
    project
        .walk(texture_set)
        .into_iter()
        .map(|(id, _)| id)
        .take_while(|id| *id != reader)
        .filter_map(|id| project.layers.get(&id))
        .flat_map(|layer| {
            let mask = layer.mask.iter().flat_map(|mask| {
                mask.effects.iter().filter(|effect| matches!(effect.kind, EffectKind::Anchor)).map(move |effect| {
                    let label = trf("{layer} / mask / {name}", &[("layer", &layer.name), ("name", &effect.name)]);
                    (effect.id, if mask.enabled { label } else { format!("{label} ({})", tr("Off")) })
                })
            });
            let content = layer
                .content_effects
                .iter()
                .filter(|effect| matches!(effect.kind, EffectKind::Anchor))
                .map(|effect| (effect.id, format!("{} / {}", layer.name, effect.name)));
            content.chain(mask).collect::<Vec<_>>()
        })
        .collect()
}

/// Anchor choices for an effect, valid for its owner and recorded below the displayed instance
pub(crate) fn effect_anchors_below(app: &TextureCraftApp, layer: LayerId, effect: LayerId) -> Vec<(LayerId, String)> {
    let owner = app.session.doc.project.effect(effect).map_or(layer, |(owner, _)| owner);
    let owner_anchors = anchors_below(app, owner);
    if owner == layer {
        return owner_anchors;
    }

    let displayed_anchors = anchors_below(app, layer);
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

/// The anchor points of `anchors` as menu entries under their own heading (a note when there
/// are none); returns the one clicked.
pub(crate) fn anchor_entries(ui: &mut Ui, anchors: &[(LayerId, String)], current: Option<LayerId>) -> Option<LayerId> {
    ui.separator();
    ui.label(caption(tr("Anchor points")));
    if anchors.is_empty() {
        ui.label(RichText::new(tr("No anchor point below this layer")).color(Tokens::get(ui.ctx()).text_dim));
    }

    let mut picked = None;
    for (anchor, label) in anchors {
        if ui.selectable_label(current == Some(*anchor), trf("Anchor: {name}", &[("name", label)])).clicked() {
            picked = Some(*anchor);
        }
    }
    picked
}

pub(crate) fn anchor_label(app: &TextureCraftApp, anchor: LayerId, channel: Channel) -> String {
    let name = app.session.doc.project.effect(anchor).map(|(_, e)| e.name.clone()).unwrap_or_else(|| tr("missing").into());
    let label = trf("Anchor: {name} ({channel})", &[("name", &name), ("channel", &tr(channel.label()))]);
    if mask_anchor_enabled(app, anchor) == Some(false) { format!("{label} ({})", tr("Off")) } else { label }
}

/// Space above a fill's Material caption, setting it apart from the projection
const FILL_MATERIAL_GAP: f32 = 6.0;

/// Space between a fill's channel chips and its channel sources
const FILL_CHANNELS_GAP: f32 = 4.0;

/// Projection and sources of a fill layer or fill effect (`base`) for its enabled channels; `reader` owns it
fn fill_editor(app: &mut TextureCraftApp, ui: &mut Ui, fill: &Fill, base: &Value, enabled_channels: &[Channel], reader: LayerId) {
    let salt = base.to_string();
    if let Projection::Decal(decal) = &fill.projection {
        decal_editor(app, ui, decal, base, reader);
    } else {
        projection_editor(app, ui, &fill.projection, "layer.fill.projection", base, &salt);
    }

    ui.add_space(FILL_MATERIAL_GAP);
    ui.label(caption(tr("Material")));
    let channels = app.session.doc.project.texture_set(app.session.ts()).map(|texture_set| texture_set.channel_list()).unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        for channel in &channels {
            let enabled = enabled_channels.contains(channel);
            if chip(ui, tr(channel.short()), enabled).on_hover_text(tr(SOLO_HINT)).clicked() {
                let solo = ui.input(|input| input.modifiers.alt);
                app.run("layer.fill.set", with(base, json!({"channel": channel.id(), "enabled": solo || !enabled, "solo": solo})));
            }
        }
    });

    ui.add_space(FILL_CHANNELS_GAP);
    let anchors = anchors_below(app, reader);
    for channel in channels {
        if enabled_channels.contains(&channel) {
            fill_channel(app, ui, fill, base, channel, &anchors, &salt);
        }
    }
}

/// Slider range of a fill's scale or tiling
const FILL_SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.01..=20.0;

/// Slider range of a UV fill's rotation, a full turn
const FILL_ROTATION_DEGREES_RANGE: std::ops::RangeInclusive<f32> = 0.0..=360.0;

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
        projection_transform(ui, projection, &mut edit);
    });
}

/// Scale plus what the projection kind adds: UV rotation and offset, or a world offset
fn projection_transform(ui: &mut Ui, projection: &Projection, edit: &mut impl FnMut(Value)) {
    let mut scale = match projection {
        Projection::Uv { scale, .. } => scale[0],
        Projection::Triplanar { scale, .. } | Projection::Planar { scale, .. } | Projection::Spherical { scale } | Projection::Cylindrical { scale, .. } => {
            *scale
        }
        Projection::Decal(_) => return,
    };
    if let Some(merge) = slider_edit(ui, tr("Scale / tiling"), &mut scale, FILL_SCALE_RANGE, 2) {
        edit(json!({"scale": scale, "merge": merge}));
    }

    match projection {
        Projection::Uv { rotation, offset, .. } => {
            let mut rotation_degrees = *rotation;
            if let Some(merge) = slider_edit(ui, tr("Rotation"), &mut rotation_degrees, FILL_ROTATION_DEGREES_RANGE, 0) {
                edit(json!({"rotation": rotation_degrees, "merge": merge}));
            }
            if let Some((edited_offset, merge)) = offset_sliders(ui, ["U", "V"], *offset) {
                edit(json!({"offset": edited_offset, "merge": merge}));
            }
        }
        Projection::Triplanar { offset, .. } | Projection::Planar { offset, .. } => {
            if let Some((edited_offset, merge)) = offset_sliders(ui, ["X", "Y", "Z"], *offset) {
                edit(json!({"offset": edited_offset, "merge": merge}));
            }
        }
        Projection::Spherical { .. } | Projection::Cylindrical { .. } | Projection::Decal(_) => {}
    }
}

/// A slider per offset component ("Offset U", …): the whole offset once one is edited, with
/// [`slider_edit`]'s `merge`.
fn offset_sliders<const N: usize>(ui: &mut Ui, axes: [&str; N], mut v: [f32; N]) -> Option<([f32; N], bool)> {
    let mut edit = None;
    for (axis, x) in axes.iter().zip(v.iter_mut()) {
        edit = edit.or(slider_edit(ui, &trf("Offset {axis}", &[("axis", axis)]), x, -1.0..=1.0, 2));
    }
    edit.map(|merge| (v, merge))
}

/// The axis a planar or cylindrical projection runs along: the one picked, if any.
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

/// A decal's frame: place it with a click in the 3D view, then size, turn and shift it. `base` is the
/// `{layer}` or `{effect}` holding it (a mask fill effect's decal edits like a layer's); `layer` is the reader.
fn decal_editor(app: &mut TextureCraftApp, ui: &mut Ui, d: &Decal, base: &Value, layer: LayerId) {
    // What a placing click or the stack selection names: the effect when the decal is a mask effect's
    let holder = base.get("effect").and_then(Value::as_u64).map_or(layer, LayerId);
    let tk = Tokens::get(ui.ctx());
    ui.label(caption(tr("Decal")));
    let placing = app.placing_decal == Some(holder);
    let label = format!("{} {}", ic::CROSSHAIR, if placing { tr("Click the mesh to place the decal") } else { tr("Place on the surface") });
    if ui.selectable_label(placing, label).on_hover_text(tr("The next click in the 3D view moves the decal there")).clicked() {
        app.placing_decal = if placing { None } else { Some(holder) };
    }
    let (lo, hi) = app.session.mesh.bounds();
    let diag = (hi - lo).length().max(1e-3);
    let mut v = [d.size[0], d.size[1], d.rotation, d.offset[0], d.offset[1], d.depth];
    let rows = [
        (tr("Width"), diag * 0.01..=diag * 2.0),
        (tr("Height"), diag * 0.01..=diag * 2.0),
        (tr("Rotation (degrees)"), 0.0..=360.0),
        (tr("Offset right"), -diag..=diag),
        (tr("Offset up"), -diag..=diag),
        (tr("Depth"), diag * 0.005..=diag),
    ];
    let mut edit = None;
    for (row, ((label, range), x)) in rows.into_iter().zip(v.iter_mut()).enumerate() {
        edit = edit.or(slider_edit(ui, label, x, range, 3).map(|merge| (row, merge)));
    }

    // The lock is a setting of the editor, not of the decal: every decal starts locked
    let keep_id = ui.scope_id().with(("decal_keep_proportions", holder.0));
    let mut keep = ui.ctx().data(|data| data.get_temp::<bool>(keep_id)).unwrap_or(true);
    if ui.checkbox(&mut keep, tr("Keep proportions")).on_hover_text(tr("Width and height follow each other by the image's aspect")).changed() {
        ui.ctx().data_mut(|data| data.insert_temp(keep_id, keep));
        if keep {
            app.run("layer.decal.set", with(base, json!({"keepProportions": true})));
        }
    }
    let mut flip = d.flip;
    let flipped = ui.horizontal(|ui| ui.checkbox(&mut flip[0], tr("Flip U")).changed() | ui.checkbox(&mut flip[1], tr("Flip V")).changed()).inner;
    if flipped {
        app.run("layer.decal.set", with(base, json!({"flipU": flip[0], "flipV": flip[1]})));
    }
    if let Some((row, merge)) = edit {
        // Locked: the edited side sets the other (a height sets the width, anything else the height)
        let size = if keep && row == 1 { json!(v[1]) } else { json!([v[0], v[1]]) };
        app.run(
            "layer.decal.set",
            with(base, json!({"size": size, "keepProportions": keep && row < 2, "rotation": v[2], "offset": [v[3], v[4]], "depth": v[5], "merge": merge})),
        );
    }
    decal_symmetry_editor(app, ui, d, base);
    ui.label(
        RichText::new(tr("The image's alpha cuts the decal's shape; faces turned away and surfaces beyond its depth stay clear."))
            .size(10.5)
            .color(tk.text_dim),
    );
}

/// The decal's mirrored copy on the other side of the mesh: the axis it is mirrored across and how its image is flipped
fn decal_symmetry_editor(app: &mut TextureCraftApp, ui: &mut Ui, decal: &Decal, base: &Value) {
    let axes = ["X", "Y", "Z"];
    let axis = decal.symmetry.map(|symmetry| usize::from(symmetry.axis));
    let mut picked = None;
    combo(ui, tr("Mirror across"), axis.and_then(|axis| axes.get(axis).copied()).unwrap_or_else(|| tr("Off")), |ui| {
        if ui.selectable_label(axis.is_none(), tr("Off")).clicked() && axis.is_some() {
            picked = Some(json!({"symmetry": "off"}));
        }
        for (i, name) in axes.into_iter().enumerate() {
            if ui.selectable_label(axis == Some(i), name).clicked() && axis != Some(i) {
                picked = Some(json!({"symmetry": name.to_ascii_lowercase()}));
            }
        }
    });

    if let Some(symmetry) = decal.symmetry {
        let flips = [(DecalFlip::None, tr("None"), "none"), (DecalFlip::U, "U", "u"), (DecalFlip::V, "V", "v")];
        let current = flips.iter().find(|flip| flip.0 == symmetry.flip).map_or("", |flip| flip.1);
        combo(ui, tr("Flip the copy's image"), current, |ui| {
            for (flip, name, id) in flips {
                if ui.selectable_label(flip == symmetry.flip, name).clicked() && flip != symmetry.flip {
                    picked = Some(json!({"symmetryFlip": id}));
                }
            }
        });
    }

    if let Some(symmetry) = decal.symmetry {
        // The plane moves within the mesh, as the symmetry popup's axis position does
        let (lo, hi) = app.session.mesh.bounds();
        let limit = ((hi - lo) * 0.5)[usize::from(symmetry.axis.min(2))];
        let mut offset = symmetry.offset;
        if let Some(merge) = slider_edit(ui, tr("Axis position"), &mut offset, -limit..=limit, 2) {
            picked = Some(json!({"symmetryOffset": offset, "merge": merge}));
        }
    }

    if let Some(change) = picked {
        app.run("layer.decal.set", with(base, change));
    }
}

/// Space a fill channel's color swatch leaves at the group's right edge
const CHANNEL_SWATCH_MARGIN: f32 = 4.0;
/// Width of a pattern's low and high swatches
const PATTERN_RANGE_SWATCH_WIDTH: f32 = 60.0;
/// Size of the gradient map menu icon beside the fill source slot
const FILL_CHANNEL_MENU_ICON_SIZE: f32 = 13.0;
const GRADIENT_MAP_HINT: &str = n_("Colors the procedural or anchor from dark to light through color stops");

/// The source slot, source settings and gradient map of one fill channel
fn fill_channel(app: &mut TextureCraftApp, ui: &mut Ui, fill: &Fill, base: &Value, channel: Channel, anchors: &[(LayerId, String)], salt: &str) {
    let tokens = Tokens::get(ui.ctx());
    let set = |extra: Value| with(base, with(&json!({"channel": channel.id()}), extra));
    let default_source = Source::uniform_of(channel);
    let source = fill.sources.get(&channel).unwrap_or(&default_source);
    let is_color = channel.kind() == ChannelKind::Color;
    let group = ui.group(|ui| {
        let slot = crate::fill_source_target::FillSourceTarget::Channel { base: base.clone(), channel };
        if is_color {
            ui.allocate_ui_with_layout(vec2(ui.available_width(), SLOT_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(RichText::new(ic::DOTS_THREE).size(FILL_CHANNEL_MENU_ICON_SIZE), |ui| gradient_map_item(app, ui, source, &set))
                    .response
                    .on_hover_text(tr(GRADIENT_MAP_HINT));
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    crate::fill_source_slot::show(app, ui, &slot, source, (anchors, tr(channel.label())));
                });
            });
        } else {
            crate::fill_source_slot::show(app, ui, &slot, source, (anchors, tr(channel.label())));
        }

        match source {
            Source::Uniform(value) if is_color => {
                let mut value = *value;
                if color_swatch(ui, &format!("fill_{salt}_{}", channel.id()), &mut value, ui.available_width() - CHANNEL_SWATCH_MARGIN) {
                    app.run("layer.fill.set", set(json!({"color": value})));
                }
            }
            Source::Uniform(value) => {
                let mut value = value[0];
                let range = if channel.kind() == ChannelKind::Signed { -1.0..=1.0 } else { 0.0..=1.0 };
                if let Some(merge) = slider_edit(ui, tr("Value"), &mut value, range, 2) {
                    app.run("layer.fill.set", set(json!({"value": value, "merge": merge})));
                }
            }
            Source::Procedural { pattern, low, high, ramp } => {
                let target = crate::pattern_target::PatternTarget {
                    command: "layer.fill.set",
                    base: with(base, json!({"channel": channel.id(), "low": low, "high": high})),
                    salt: format!("{salt}_{}", channel.id()),
                };
                crate::pattern_panel::pattern_properties(app, ui, &target, pattern);
                if is_color && ramp.is_empty() {
                    let (mut low, mut high) = (*low, *high);
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        ui.label(tr("Low"));
                        changed |= color_swatch(ui, &format!("lo_{salt}_{}", channel.id()), &mut low, PATTERN_RANGE_SWATCH_WIDTH);
                        ui.label(tr("High"));
                        changed |= color_swatch(ui, &format!("hi_{salt}_{}", channel.id()), &mut high, PATTERN_RANGE_SWATCH_WIDTH);
                    });
                    if changed {
                        crate::pattern_panel::send(app, &target, pattern, json!({"low": low, "high": high}));
                    }
                }
            }
            Source::Image { name, normal_format } => {
                ui.label(trf("Image: {name}", &[("name", name)]));
                if channel.kind() == ChannelKind::NormalMap {
                    let mut directx = *normal_format == texturecraft_engine::core::NormalFormat::DirectX;
                    let toggle = ui
                        .checkbox(&mut directx, tr("DirectX normal map"))
                        .on_hover_text(tr("Flips the image's green channel, for normal maps made with Y pointing down"));
                    if toggle.changed() {
                        app.run("layer.fill.set", set(json!({"normalFormat": if directx { "directX" } else { "openGl" }})));
                    }
                }
            }
            Source::Anchor { anchor, channel: anchored, .. } => {
                ui.label(RichText::new(anchor_label(app, *anchor, *anchored)).color(tokens.text_dim));
            }
        }

        if is_color && !source.ramp().is_empty() {
            ramp_editor(app, ui, source.ramp(), &format!("ramp_{salt}_{}", channel.id()), &set);
        }
    });

    // A shelf image dropped on the channel becomes its source
    if let Some(name) = image_drop(ui, &group.response.interact(Sense::hover())) {
        app.run("layer.fill.set", set(json!({"image": name})));
    }
}

/// The Gradient Map item beside a color channel's source slot: turns the color stops of its
/// procedural (starting from its low and high, so the look holds) or anchor on, or off.
fn gradient_map_item(app: &mut TextureCraftApp, ui: &mut Ui, source: &Source, set: &impl Fn(Value) -> Value) {
    let can = matches!(source, Source::Procedural { .. } | Source::Anchor { .. });
    let on = !source.ramp().is_empty();
    let hint = tr(GRADIENT_MAP_HINT);
    let item = ui.add_enabled_ui(can, |ui| ui.selectable_label(on, tr("Gradient Map"))).inner;
    if item.on_hover_text(hint).on_disabled_hover_text(hint).clicked() {
        let params = if on { json!({ "colors": [] }) } else { json!({ "gradientMap": true }) };
        app.run("layer.fill.set", set(params));
        ui.close();
    }
}

/// The color stops of a fill channel's gradient map.
fn ramp_editor(app: &mut TextureCraftApp, ui: &mut Ui, ramp: &[[f32; 3]], salt: &str, set: &impl Fn(Value) -> Value) {
    let mut stops = ramp.to_vec();
    if gradient_stops(ui, salt, &mut stops) {
        app.run("layer.fill.set", set(json!({ "colors": stops })));
    }
}

/// Swatches for gradient map stops (dark to light) with Add color and Remove last; true when
/// they changed.
fn gradient_stops(ui: &mut Ui, salt: &str, stops: &mut Vec<[f32; 3]>) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (i, stop) in stops.iter_mut().enumerate() {
            changed |= color_swatch(ui, &format!("{salt}_{i}"), stop, FILTER_SWATCH_WIDTH);
        }
    });
    ui.horizontal(|ui| {
        if ui.add_enabled(stops.len() < MAX_GRADIENT_STOPS, egui::Button::new(format!("{} {}", ic::PLUS, tr("Add color")))).clicked() {
            stops.push(stops.last().copied().unwrap_or([1.0; 3]));
            changed = true;
        }
        if ui.add_enabled(stops.len() > 1, egui::Button::new(format!("{} {}", ic::MINUS, tr("Remove last")))).clicked() {
            stops.pop();
            changed = true;
        }
    });
    changed
}

/// The properties of effect `effect_id` of `layer`: each kind's own settings.
fn effect_properties(app: &mut TextureCraftApp, ui: &mut Ui, layer: &Layer, effect_id: LayerId) {
    let tokens = Tokens::get(ui.ctx());
    let Some(effect) = app.session.doc.project.effect(effect_id).map(|(_, effect)| effect.clone()) else { return };

    let id = effect_id.0;

    match &effect.kind {
        EffectKind::Generator(generator) => crate::generator_panel::generator_properties(app, ui, layer.id, id, generator),
        EffectKind::Levels(levels) => crate::levels_properties::levels_properties(app, ui, layer, effect_id, levels),
        EffectKind::Fill { source, projection } => {
            ui.label(caption(tr("Fill")));
            let anchors = effect_anchors_below(app, layer.id, effect_id);
            let target = crate::fill_source_target::FillSourceTarget::Mask { effect: id };
            let slot = ui.scope(|ui| crate::fill_source_slot::show(app, ui, &target, source, (&anchors, tr("Grayscale")))).response;

            // The slot accepts image drops from the Assets panel
            if let Some(name) = image_drop(ui, &slot.interact(Sense::hover())) {
                app.run("effect.set", json!({"effect": id, "image": name}));
            }

            match source {
                Source::Uniform(color) => {
                    let mut value = color[0];
                    if let Some(merge) = slider_edit(ui, tr("Value"), &mut value, 0.0..=1.0, 2) {
                        app.run("effect.set", json!({"effect": id, "value": value, "merge": merge}));
                    }
                }
                Source::Procedural { pattern, .. } => {
                    let target = crate::pattern_target::PatternTarget { command: "effect.set", base: json!({"effect": id}), salt: format!("mask_fill_{id}") };
                    crate::pattern_panel::pattern_properties(app, ui, &target, pattern);
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
                    if let Projection::Decal(decal) = projection {
                        decal_editor(app, ui, decal, &json!({"effect": id}), layer.id);
                    } else {
                        projection_editor(app, ui, projection, "effect.set", &json!({"effect": id}), &format!("maskfill{id}"));
                    }
                }
            }
            if anchors.is_empty() {
                ui.label(RichText::new(tr("Add an anchor point to a layer below to use its stack as a mask.")).color(tokens.text_dim));
            }
        }
        EffectKind::Material(fill) => {
            ui.label(caption(tr("Fill")));
            let enabled: Vec<Channel> = fill.sources.keys().copied().collect();
            fill_editor(app, ui, fill, &json!({"effect": id}), &enabled, layer.id);
        }
        EffectKind::Anchor => {
            ui.label(caption(tr("Anchor point")));
            let text = if anchor_in_mask(app, effect_id) {
                tr("Records the mask value at this point. Fill effects and generators in masks above can read it.")
            } else {
                tr("Records the stack up to this layer. Fill effects in masks (and fill layers) above can read it as a source.")
            };
            ui.label(RichText::new(text).color(tokens.text_dim));
        }
        EffectKind::Paint => {
            let hint = if layer.content_effects.iter().any(|content| content.id == effect_id) {
                tr("Paint on the mesh to edit the layer content.")
            } else {
                tr("Paint on the mesh to edit this mask. Use the Material tab's mask value for gray.")
            };
            ui.label(RichText::new(hint).color(tokens.text_dim));
            if app.session.tool_usable(app.session.tool.tool) {
                tool_properties(app, ui);
            }
        }
        EffectKind::Path(paths) => {
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
        }
        EffectKind::Filter(filter) => filter_properties(app, ui, id, filter),
        EffectKind::ColorSelection { colors, tolerance } => crate::color_selection_editor::color_selection_properties(app, ui, effect_id, colors, *tolerance),
        other => {
            ui.label(RichText::new(tr(other.name())).color(tokens.text_dim));
        }
    }
}

/// The filter slot and the parameters of filter effect `id`
fn filter_properties(app: &mut TextureCraftApp, ui: &mut Ui, id: u64, filter: &Filter) {
    let tk = Tokens::get(ui.ctx());
    let set = |app: &mut TextureCraftApp, key: &str, value: Value, merge: bool| {
        let mut params = json!({ "effect": id, "merge": merge });
        params[key] = value;
        app.run("effect.set", params);
    };
    crate::filter_slot::show(app, ui, id, filter);

    if let Filter::Hsl { hue, .. } = filter {
        let mut degrees = hue * 360.0;
        if let Some(merge) = slider_edit(ui, tr("Hue shift (degrees)"), &mut degrees, -180.0..=180.0, 0) {
            set(app, "hue", json!(degrees / 360.0), merge);
        }
    }

    for (key, label, mut value, min, max) in scalar_params(filter) {
        let decimals = if matches!(key, "seed" | "levels") {
            0
        } else if max - min > 10.0 {
            1
        } else {
            2
        };
        if let Some(merge) = slider_edit(ui, tr(label), &mut value, min..=max, decimals) {
            set(app, key, json!(value), merge);
        }
    }

    match filter {
        Filter::ColorBalance { shadows, midtones, highlights } => {
            for (key, label, rgb) in [("shadows", tr("Shadows"), shadows), ("midtones", tr("Midtones"), midtones), ("highlights", tr("Highlights"), highlights)]
            {
                ui.label(RichText::new(label).strong());
                let mut balance = *rgb;
                let mut edit = None;
                for (component, name) in balance.iter_mut().zip([tr("Cyan / red"), tr("Magenta / green"), tr("Yellow / blue")]) {
                    edit = edit.or(slider_edit(ui, name, component, -1.0..=1.0, 2));
                }
                if let Some(merge) = edit {
                    set(app, key, json!(balance), merge);
                }
            }
        }
        Filter::ReplaceColor { from, to, .. } => {
            for (key, label, rgb) in [("from", tr("Color to replace"), from), ("to", tr("Replacement color"), to)] {
                let mut color = *rgb;
                ui.horizontal(|ui| {
                    if color_swatch(ui, &format!("replace_{key}_{id}"), &mut color, FILTER_SWATCH_WIDTH) {
                        set(app, key, json!(color), false);
                    }
                    ui.label(label);
                });
            }
            ui.label(RichText::new(tr("Colors near the first shift towards the second, keeping their shading.")).size(FILTER_HINT_SIZE).color(tk.text_dim));
        }
        Filter::GradientMap { colors } => {
            let mut stops = colors.clone();
            if gradient_stops(ui, &format!("gradient_{id}"), &mut stops) {
                set(app, "colors", json!(stops), false);
            }
            ui.label(RichText::new(tr("Dark to light: the content's luminance picks a color along these stops.")).size(FILTER_HINT_SIZE).color(tk.text_dim));
        }
        Filter::Warp { .. }
        | Filter::DirectionalBlur { .. }
        | Filter::SlopeBlur { .. }
        | Filter::DirectionalWarp { .. }
        | Filter::HighPass { .. }
        | Filter::EdgeDetect { .. }
        | Filter::Bevel { .. }
        | Filter::Mosaic { .. }
        | Filter::Glow { .. } => {
            ui.label(RichText::new(tr("Works in texture space, like the blur: it does not cross UV seams.")).size(FILTER_HINT_SIZE).color(tk.text_dim));
        }
        _ => {}
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
