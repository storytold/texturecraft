//! The dialog that picks texture sets: which ones share settings, or which ones a copy goes to

use std::sync::Arc;
use texturecraft_engine::Session;

use egui::{Align2, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as icon;
use serde_json::{Value, json};
use texturecraft_engine::bake::settings_patch;
use texturecraft_engine::core::{BakeSettings, BakeSyncGroup, BakedMap, Project};

use super::common_settings::antialiasing_label;
use super::set_picker_purpose::SetPickerPurpose;
use super::set_picker_row::SetPickerRow;
use crate::TextureCraftApp;
use crate::i18n::{baked_map_label, tr, trf};
use crate::property_sheet::PropertySheet;
use crate::theme::{Tokens, semibold};

/// Width of the dialog, in points
const DIALOG_WIDTH: f32 = 420.0;

/// The title bar: its height, the gear's and the title's left edges, and the glyph and title sizes
const TITLE_BAR_HEIGHT: f32 = 30.0;
const TITLE_GEAR_CENTER: f32 = 19.0;
const TITLE_TEXT_LEFT: f32 = 34.0;
const TITLE_GLYPH_SIZE: f32 = 14.0;
const TITLE_TEXT_SIZE: f32 = 12.0;

/// Air under the title bar and above the buttons, and around the dialog's edge
const BODY_AIR: f32 = 12.0;
const BUTTONS_AIR: f32 = 18.0;
const EDGE_AIR: f32 = 12.0;

/// OK and Cancel: their size, the air between them, their corners
const BUTTON_SIZE: egui::Vec2 = vec2(88.0, 24.0);
const BUTTON_GAP: f32 = 8.0;
const DIALOG_RADIUS: u8 = 4;

/// An open picker: what it is for, and per texture set whether it is ticked
#[derive(Clone, Debug)]
pub(super) struct SetPicker {
    purpose: SetPickerPurpose,
    chosen: Vec<bool>,
    mesh_id: u64,
    active: usize,
    opener: egui::Id,
    cached: Option<(u64, &'static str, Arc<[SetPickerRow]>)>,
}

impl SetPicker {
    fn id() -> egui::Id {
        egui::Id::unique("bake_set_picker")
    }

    /// Open the picker for `purpose`; a sync picker starts with the sets sharing the group ticked, a copy picker with none
    pub(super) fn open(app: &TextureCraftApp, opener: &egui::Response, purpose: SetPickerPurpose) {
        let chosen = app
            .session
            .doc
            .project
            .texture_sets
            .iter()
            .map(|set| match purpose {
                SetPickerPurpose::Sync(group) => set.bake.sync.shares(group),
                SetPickerPurpose::Copy { .. } => false,
            })
            .collect();
        opener.ctx.data_mut(|data| {
            data.insert_temp(Self::id(), SetPicker { purpose, chosen, mesh_id: app.session.mesh_id, active: app.session.ts(), opener: opener.id, cached: None })
        });
    }

    /// The open picker, if any; OK runs its command
    pub(super) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
        let Some(mut picker) = ctx.data(|data| data.get_temp::<SetPicker>(Self::id())) else {
            // The opener can receive focus again once the modal layer has left egui's focus order
            if ctx.memory(|memory| memory.top_modal_layer().is_none())
                && let Some(opener) = ctx.data_mut(|data| data.remove_temp::<Option<egui::Id>>(Self::id()).flatten())
            {
                ctx.memory_mut(|memory| memory.request_focus(opener));
            }

            return;
        };

        let active = app.session.ts();
        if picker.mesh_id != app.session.mesh_id || picker.active != active || picker.chosen.len() != app.session.set_count() {
            ctx.data_mut(|data| data.remove::<SetPicker>(Self::id()));
            return;
        }

        let opening = picker.cached.is_none() && ctx.memory(|memory| memory.top_modal_layer().is_none());
        let rows = picker.visible_rows(&app.session, active);
        let tokens = Tokens::get(ctx);
        let frame = egui::Frame::new()
            .fill(tokens.panel)
            .stroke(Stroke::new(1.0, tokens.button_border))
            .corner_radius(CornerRadius::same(DIALOG_RADIUS))
            .inner_margin(0);
        let modal = egui::Modal::new(Self::id().with("modal")).frame(frame).show(ctx, |ui| {
            ui.set_width(DIALOG_WIDTH);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let closed = title_bar(ui, &tokens, &picker.title(), opening);
            ui.add_space(BODY_AIR);
            PropertySheet::show(ui, "bake_set_picker_sheet", |sheet| picker.list(sheet, &rows));
            ui.add_space(BUTTONS_AIR);
            let (ok, cancel) = buttons(ui, &tokens);
            ui.add_space(EDGE_AIR);
            (ok, cancel || closed)
        });

        let (ok, cancel) = modal.inner;
        if ok {
            picker.apply(app, active);
        }
        if ok || cancel || modal.should_close() {
            ctx.data_mut(|data| {
                data.remove::<SetPicker>(Self::id());
                data.insert_temp(Self::id(), Some(picker.opener));
            });
            ctx.request_repaint();
        } else {
            ctx.data_mut(|data| data.insert_temp(Self::id(), picker));
        }
    }

    /// The listed sets
    fn rows(&self, project: &Project, active: usize) -> Vec<SetPickerRow> {
        let chosen: Vec<usize> = self.chosen.iter().enumerate().filter(|(_, chosen)| **chosen).map(|(index, _)| index).collect();
        project
            .texture_sets
            .iter()
            .enumerate()
            // A copy goes from the active set, so it isn't listed
            .filter(|(index, _)| matches!(self.purpose, SetPickerPurpose::Sync(_)) || *index != active)
            .map(|(index, set)| {
                let note = match self.purpose {
                    // Include this row when predicting its sync source
                    SetPickerPurpose::Sync(group) if !set.bake.sync.shares(group) => {
                        let mut members = chosen.clone();
                        if let Err(slot) = members.binary_search(&index) {
                            members.insert(slot, index);
                        }
                        project
                            .bake_sync_source(group, &members, active)
                            .filter(|source| *source != index)
                            .and_then(|source| project.texture_sets.get(source))
                            .and_then(|source| differing_values(&set.bake, &source.bake, group))
                    }
                    _ => None,
                };
                SetPickerRow { index, name: set.name.clone(), note }
            })
            .collect()
    }

    fn visible_rows(&mut self, session: &Session, active: usize) -> Arc<[SetPickerRow]> {
        let language = crate::i18n::current().code;
        if let Some((revision, cached_language, rows)) = &self.cached
            && *revision == session.revision
            && *cached_language == language
        {
            return rows.clone();
        }

        let rows: Arc<[SetPickerRow]> = self.rows(&session.doc.project, active).into();
        self.cached = Some((session.revision, language, rows.clone()));
        rows
    }

    fn title(&self) -> String {
        match self.purpose {
            SetPickerPurpose::Sync(group) => trf("{settings} Sync", &[("settings", &group_label(group))]),
            SetPickerPurpose::Copy { what: "checked", .. } => tr("Apply Checked to More Texture Sets").to_owned(),
            SetPickerPurpose::Copy { what: "check", map: Some(map) } => trf("Check {map} for More Texture Sets", &[("map", &tr(baked_map_label(map)))]),
            SetPickerPurpose::Copy { map: Some(map), .. } => trf("Apply {map} Settings to More Texture Sets", &[("map", &tr(baked_map_label(map)))]),
            SetPickerPurpose::Copy { .. } => tr("Sync All Settings to More Texture Sets").to_owned(),
        }
    }

    /// The master check and one check per listed set, indented under it
    fn list(&mut self, sheet: &mut PropertySheet<'_>, rows: &[SetPickerRow]) {
        let ticked = rows.iter().filter(|row| self.chosen.get(row.index).copied().unwrap_or(false)).count();
        let all_ticked = ticked == rows.len();
        let label = if all_ticked { tr("Deselect All") } else { tr("Select All") };
        let state = if ticked == 0 { Some(false) } else { all_ticked.then_some(true) };
        if sheet.check_mixed(label, state).clicked() {
            self.cached = None;
            for row in rows {
                if let Some(chosen) = self.chosen.get_mut(row.index) {
                    *chosen = !all_ticked;
                }
            }
        }
        sheet.indented(|sheet| {
            for row in rows {
                let Some(chosen) = self.chosen.get_mut(row.index) else { continue };
                if sheet.check_noted(&row.name, Some(*chosen), row.note.as_deref()).clicked() {
                    self.cached = None;
                    *chosen = !*chosen;
                }
            }
        });
    }

    fn apply(&self, app: &mut TextureCraftApp, active: usize) {
        let ticked: Vec<usize> = self.chosen.iter().enumerate().filter(|(_, chosen)| **chosen).map(|(index, _)| index).collect();
        match self.purpose {
            SetPickerPurpose::Sync(group) => {
                app.run("bake.sync", json!({ "group": group_id(group), "ts": ticked }));
            }
            SetPickerPurpose::Copy { what, map } if !ticked.is_empty() => {
                app.run("bake.copy", json!({ "what": what, "map": map.map(BakedMap::id), "from": active, "ts": ticked }));
            }
            SetPickerPurpose::Copy { .. } => {}
        }
    }
}

/// The group as the `bake.sync` command names it
pub(super) fn group_id(group: BakeSyncGroup) -> &'static str {
    match group {
        BakeSyncGroup::Common => "common",
        BakeSyncGroup::Map(map) => map.id(),
    }
}

fn group_label(group: BakeSyncGroup) -> String {
    match group {
        BakeSyncGroup::Common => tr("Common Settings").to_owned(),
        BakeSyncGroup::Map(map) => trf("{map} Settings", &[("map", &tr(baked_map_label(map)))]),
    }
}

/// The title bar: gear, title, and × at the right; returns whether × was clicked
fn title_bar(ui: &mut Ui, tokens: &Tokens, title: &str, opening: bool) -> bool {
    let (bar, _) = ui.allocate_exact_size(vec2(DIALOG_WIDTH, TITLE_BAR_HEIGHT), Sense::hover());
    let top = CornerRadius { nw: DIALOG_RADIUS, ne: DIALOG_RADIUS, sw: 0, se: 0 };
    ui.painter().rect_filled(bar, top, tokens.chrome);
    let center_y = bar.center().y;
    ui.painter().text(pos2(bar.left() + TITLE_GEAR_CENTER, center_y), Align2::CENTER_CENTER, icon::GEAR, FontId::proportional(TITLE_GLYPH_SIZE), tokens.text);
    let text = Rect::from_min_max(pos2(bar.left() + TITLE_TEXT_LEFT, bar.top()), pos2(bar.right() - TITLE_BAR_HEIGHT, bar.bottom()));
    let label = egui::Label::new(RichText::new(title).font(semibold(TITLE_TEXT_SIZE)).color(tokens.text_strong)).truncate();
    ui.scope_builder(egui::UiBuilder::new().max_rect(text).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| ui.add(label));

    let close = Rect::from_center_size(pos2(bar.right() - TITLE_BAR_HEIGHT / 2.0, center_y), vec2(TITLE_BAR_HEIGHT, TITLE_BAR_HEIGHT));
    let response = ui.interact(close, ui.scope_id().with("close"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Close")));
    if opening {
        response.request_focus();
    }

    let color = if response.hovered() { tokens.text_strong } else { tokens.text_dim };
    ui.painter().text(close.center(), Align2::CENTER_CENTER, icon::X, FontId::proportional(TITLE_TEXT_SIZE), color);
    crate::property_sheet::paint_keyboard_focus(ui, &response, tokens);
    response.clicked()
}

/// OK then Cancel at the right, both neutral; returns (OK, Cancel) clicked
fn buttons(ui: &mut Ui, tokens: &Tokens) -> (bool, bool) {
    let (row, _) = ui.allocate_exact_size(vec2(DIALOG_WIDTH, BUTTON_SIZE.y), Sense::hover());
    let cancel = Rect::from_min_size(pos2(row.right() - EDGE_AIR - BUTTON_SIZE.x, row.top()), BUTTON_SIZE);
    let ok = cancel.translate(vec2(-(BUTTON_SIZE.x + BUTTON_GAP), 0.0));
    (dialog_button(ui, tokens, ok, tr("OK")), dialog_button(ui, tokens, cancel, tr("Cancel")))
}

fn dialog_button(ui: &mut Ui, tokens: &Tokens, rect: Rect, text: &str) -> bool {
    let response = ui.interact(rect, ui.scope_id().with(text), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    let fill = if response.hovered() { tokens.row_hover } else { tokens.button };
    ui.painter().rect(rect, CornerRadius::same(DIALOG_RADIUS), fill, Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(TITLE_TEXT_SIZE), tokens.text);
    crate::property_sheet::paint_keyboard_focus(ui, &response, tokens);
    response.clicked()
}

/// What `set` holds in `group` that differs from `source`, dim beside it: output size and antialiasing by value, the rest counted
fn differing_values(set: &BakeSettings, source: &BakeSettings, group: BakeSyncGroup) -> Option<String> {
    let mut taken = set.clone();
    taken.copy_group(source, group);
    let mut patch = settings_patch(set, &taken);
    let mut values = Vec::new();
    if let Some(common) = patch.get_mut("common").and_then(Value::as_object_mut) {
        let resolution = common.remove("resolution").is_some();
        let height = common.remove("resolution_height").is_some();
        if resolution || height {
            values.push(match set.common.output_size() {
                (width, height) if width == height => width.to_string(),
                (width, height) => format!("{width} × {height}"),
            });
        }
        if common.remove("antialiasing").is_some() {
            values.push(tr(antialiasing_label(set.common.antialiasing)).to_owned());
        }
    }
    let rest = leaf_count(&patch);
    match (values.is_empty(), rest) {
        (true, 0) => None,
        (true, 1) => Some(tr("1 differing setting").to_owned()),
        (true, count) => Some(trf("{count} differing settings", &[("count", &count.to_string())])),
        (false, 0) => Some(values.join(" · ")),
        (false, count) => Some(trf("{values} · {count} more", &[("values", &values.join(" · ")), ("count", &count.to_string())])),
    }
}

/// How many plain values a settings patch changes
fn leaf_count(patch: &Value) -> usize {
    match patch {
        Value::Object(fields) => fields.values().map(leaf_count).sum(),
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::core::{BakeAntialiasing, BakeResolution, BakeSettings, BakeSyncGroup, BakedMap};

    use super::{SetPicker, SetPickerPurpose, differing_values};

    #[test]
    fn a_set_shows_its_own_size_and_antialiasing_and_counts_the_rest() {
        let source = BakeSettings::default();
        let mut set = BakeSettings::default();
        set.common.resolution = BakeResolution::R1024;
        set.common.antialiasing = BakeAntialiasing::Subsampling4x4;
        set.common.dilation = 8;
        set.high_poly.ignore_backface = !source.high_poly.ignore_backface;

        let note = differing_values(&set, &source, BakeSyncGroup::Common);

        assert_eq!(note.as_deref(), Some("1024 · Subsampling 4x4 · 2 more"));
    }

    #[test]
    fn a_set_matching_the_group_shows_nothing_and_other_groups_are_not_compared() {
        let source = BakeSettings::default();
        let mut set = BakeSettings::default();
        set.common.is_enabled = false;
        set.maps.ao.parameters.rays.rays = 16;

        assert_eq!(differing_values(&set, &source, BakeSyncGroup::Common), None);
        assert_eq!(differing_values(&set, &source, BakeSyncGroup::Map(BakedMap::AmbientOcclusion)).as_deref(), Some("1 differing setting"));
    }

    #[test]
    fn the_note_compares_with_the_set_the_sync_will_take_settings_from() {
        let ao = BakeSyncGroup::Map(BakedMap::AmbientOcclusion);
        let mut project = texturecraft_engine::sample_session(256, 64).unwrap().doc.project;
        let third = project.texture_sets[1].clone();
        project.texture_sets.push(third);
        for (index, set) in project.texture_sets.iter_mut().enumerate() {
            set.bake = BakeSettings::default();
            set.bake.sync.set_shares(ao, index == 0);
        }
        project.texture_sets[1].bake.maps.ao.parameters.rays.rays = 16;

        // Set 0 leaves the group and sets 1 and 2 join with set 1 active: set 2 takes set 1's values, not set 0's
        let picker =
            SetPicker { purpose: SetPickerPurpose::Sync(ao), chosen: vec![false, true, true], mesh_id: 0, active: 1, opener: egui::Id::NULL, cached: None };
        let rows = picker.rows(&project, 1);
        let note = |index: usize| rows.iter().find(|row| row.index == index).and_then(|row| row.note.clone());

        assert_eq!(note(2).as_deref(), Some("1 differing setting"));
        assert_eq!(note(1), None, "the source itself has nothing to take");
    }
}
