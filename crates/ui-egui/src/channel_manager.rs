//! The dialog that adds and removes a texture set's channels: every channel by group, ticked when the set has it

use std::sync::Arc;

use egui::{Align2, CornerRadius, FontId, Galley, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use egui_phosphor::regular as icon;
use serde_json::json;
use texturecraft_engine::core::{Channel, ChannelGroup, TextureSet};

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::property_sheet::{CHECK_SIZE, PropertySheet, paint_check, paint_keyboard_focus};
use crate::theme::{Tokens, semibold};

/// The columns, left to right: which groups each lists, and how wide it is; the User group runs on into the last column
const COLUMNS: [(&[ChannelGroup], f32); 4] = [
    (&[ChannelGroup::Base, ChannelGroup::Specular, ChannelGroup::Transmission, ChannelGroup::Subsurface], GROUP_COLUMN_WIDTH),
    (&[ChannelGroup::Coat, ChannelGroup::Sheen, ChannelGroup::Emission, ChannelGroup::ThinFilm], GROUP_COLUMN_WIDTH),
    (&[ChannelGroup::Geometry, ChannelGroup::Miscellaneous], GROUP_COLUMN_WIDTH),
    (&[ChannelGroup::User], USER_COLUMN_WIDTH),
];
const GROUP_COLUMN_WIDTH: f32 = 230.0;
const USER_COLUMN_WIDTH: f32 = 140.0;
const USER_ROWS_PER_COLUMN: usize = 16;

/// Width of the dialog, in points
const DIALOG_WIDTH: f32 = GROUP_COLUMN_WIDTH * 3.0 + USER_COLUMN_WIDTH * 2.0;

/// The channel columns scroll past this share of the window's height
const MAX_HEIGHT_SHARE: f32 = 0.75;

/// The title bar: its height, the title's left edge and size
const TITLE_BAR_HEIGHT: f32 = 30.0;
const TITLE_TEXT_LEFT: f32 = 12.0;
const TEXT_SIZE: f32 = 12.0;
const SMALL_TEXT_SIZE: f32 = 11.0;

/// Air under the title bar, above the footer, around the dialog's edge and between the footer's parts
const BODY_AIR: f32 = 8.0;
const FOOTER_AIR: f32 = 10.0;
const EDGE_AIR: f32 = 12.0;
const CHECK_TO_LABEL: f32 = 8.0;
const LEGEND_GAP: f32 = 24.0;
const LEGEND_GLYPH_TO_TEXT: f32 = 4.0;

/// Cancel and Apply: their size, the air between them, the dialog's corners
const BUTTON_SIZE: egui::Vec2 = vec2(88.0, 24.0);
const BUTTON_GAP: f32 = 8.0;
const DIALOG_RADIUS: u8 = 4;

/// An open dialog: the set it edits, each channel's tick (indexed like [`Channel::ALL`]), and whether Apply goes to every set
#[derive(Clone, Debug)]
pub(crate) struct ChannelManager {
    ts: usize,
    ticked: Vec<bool>,
    all_sets: bool,
    mesh_id: u64,
    opener: egui::Id,
}

impl ChannelManager {
    fn id() -> egui::Id {
        egui::Id::unique("channel_manager")
    }

    /// Open the dialog on texture set `ts`, ticked as the set is
    pub(crate) fn open(app: &TextureCraftApp, opener: &egui::Response, ts: usize) {
        let Some(set) = app.session.doc.project.texture_sets.get(ts) else { return };
        let manager = ChannelManager::new(set, ts, app.session.mesh_id, opener.id);
        opener.ctx.data_mut(|data| data.insert_temp(Self::id(), Some(manager)));
    }

    fn new(set: &TextureSet, ts: usize, mesh_id: u64, opener: egui::Id) -> Self {
        let ticked = Channel::ALL.iter().map(|c| set.has_channel(*c)).collect();
        ChannelManager { ts, ticked, all_sets: false, mesh_id, opener }
    }

    /// The open dialog, if any; Apply runs one `textureSet.channels`
    pub(crate) fn show(app: &mut TextureCraftApp, ctx: &egui::Context) {
        let Some(mut manager) = ctx.data(|data| data.get_temp::<Option<ChannelManager>>(Self::id())).flatten() else {
            // The opener can receive focus again once the modal layer has left egui's focus order
            if ctx.memory(|memory| memory.top_modal_layer().is_none())
                && let Some(opener) = ctx.data_mut(|data| data.remove_temp::<Option<egui::Id>>(Self::id().with("opener")).flatten())
            {
                ctx.memory_mut(|memory| memory.request_focus(opener));
            }
            return;
        };

        if manager.mesh_id != app.session.mesh_id || manager.ts >= app.session.set_count() {
            ctx.data_mut(|data| data.remove::<Option<ChannelManager>>(Self::id()));
            return;
        }

        let opening = ctx.memory(|memory| memory.top_modal_layer().is_none());
        let tokens = Tokens::get(ctx);
        let frame = egui::Frame::new()
            .fill(tokens.panel)
            .stroke(Stroke::new(1.0, tokens.button_border))
            .corner_radius(CornerRadius::same(DIALOG_RADIUS))
            .inner_margin(0);
        let modal = egui::Modal::new(Self::id().with("modal")).frame(frame).show(ctx, |ui| {
            let width = (ctx.content_rect().width() - 2.0 * EDGE_AIR).clamp(1.0, DIALOG_WIDTH);
            ui.set_width(width);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            let footer = FooterLayout::new(ui, &tokens);
            let closed = title_bar(ui, &tokens, opening);
            ui.add_space(BODY_AIR);

            // The list gets what the title bar, the footer and the air between them leave, so Apply stays in a short window
            let window = ctx.content_rect().height();
            let room = window - TITLE_BAR_HEIGHT - BODY_AIR - FOOTER_AIR - footer.height - EDGE_AIR;
            let max_height = (window * MAX_HEIGHT_SHARE).min(room).max(0.0);
            egui::ScrollArea::both().auto_shrink([false, true]).max_height(max_height).show(ui, |ui| manager.columns(ui));
            ui.add_space(FOOTER_AIR);
            let (apply, cancel) = manager.footer(ui, &tokens, footer);
            ui.add_space(EDGE_AIR);
            (apply, cancel || closed)
        });

        let (apply, cancel) = modal.inner;
        if apply && let Some(set) = app.session.doc.project.texture_sets.get(manager.ts) {
            let (add, remove) = manager.edits(set);
            if !add.is_empty() || !remove.is_empty() {
                let ids = |list: Vec<Channel>| list.into_iter().map(Channel::id).collect::<Vec<_>>();
                app.run("textureSet.channels", json!({ "ts": manager.ts, "add": ids(add), "remove": ids(remove), "allSets": manager.all_sets }));
            }
        }
        if apply || cancel || modal.should_close() {
            ctx.data_mut(|data| {
                data.remove::<Option<ChannelManager>>(Self::id());
                data.insert_temp(Self::id().with("opener"), Some(manager.opener));
            });
            ctx.request_repaint();
        } else {
            ctx.data_mut(|data| data.insert_temp(Self::id(), Some(manager)));
        }
    }

    /// The channels to add and to remove: against `set` alone, or for every set the whole ticked list
    fn edits(&self, set: &TextureSet) -> (Vec<Channel>, Vec<Channel>) {
        let ticked = |c: &Channel| self.ticked.get(c.index()).copied().unwrap_or(false);
        let (on, off): (Vec<Channel>, Vec<Channel>) = Channel::ALL.into_iter().partition(ticked);
        if self.all_sets {
            // Other sets may differ from this one: send the whole list so every set ends up the same
            return (on, off.into_iter().filter(|c| *c != Channel::BaseColor).collect());
        }

        (on.into_iter().filter(|c| !set.has_channel(*c)).collect(), off.into_iter().filter(|c| set.has_channel(*c)).collect())
    }

    /// The groups side by side, a check per channel; a dim ⊘ marks the material channels our shader doesn't read
    fn columns(&mut self, ui: &mut Ui) {
        let mut user_rows_top = None;
        ui.horizontal_top(|ui| {
            for (column, (groups, width)) in COLUMNS.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.set_width(*width);
                    PropertySheet::show(ui, ("channel_manager", column), |sheet| {
                        for group in *groups {
                            sheet.section(tr(group.label()), false);
                            let channels = group.channels();
                            let shown = if *group == ChannelGroup::User { &channels[..USER_ROWS_PER_COLUMN] } else { channels };
                            for (row, channel) in shown.iter().enumerate() {
                                let response = self.check(sheet, *channel);
                                if *group == ChannelGroup::User && row == 0 {
                                    user_rows_top = Some(response.rect.top());
                                }
                            }
                        }
                    });
                });
            }

            // The rest of the User group, its rows level with the first half's
            ui.vertical(|ui| {
                ui.set_width(USER_COLUMN_WIDTH);
                if let Some(top) = user_rows_top {
                    ui.add_space((top - ui.cursor().top()).max(0.0));
                }
                PropertySheet::show(ui, ("channel_manager", COLUMNS.len()), |sheet| {
                    for channel in &ChannelGroup::User.channels()[USER_ROWS_PER_COLUMN..] {
                        self.check(sheet, *channel);
                    }
                });
            });
        });
    }

    fn check(&mut self, sheet: &mut PropertySheet<'_>, channel: Channel) -> egui::Response {
        let Some(on) = self.ticked.get(channel.index()).copied() else {
            return sheet.ui.allocate_response(egui::Vec2::ZERO, Sense::hover());
        };

        // User channels are the user's own data for export: reading them is never the shader's job, so they carry no mark
        let user = ChannelGroup::User.channels().contains(&channel);
        let note = (!channel.is_shaded() && !user).then_some(icon::PROHIBIT);

        // Base color stays: every set has it
        let fixed = channel == Channel::BaseColor;
        let response = sheet.greyed(!fixed, fixed.then(|| tr("Base color can't be removed")), |sheet| sheet.check_noted(tr(channel.label()), Some(on), note));
        if note.is_some() {
            sheet.ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(tr("Not used by the shader")));
        }
        if response.clicked()
            && let Some(tick) = self.ticked.get_mut(channel.index())
        {
            *tick = !on;
        }
        response
    }

    /// "Apply to all Texture Sets", the ⊘ legend, then Cancel and Apply at the right; returns (Apply, Cancel) clicked
    fn footer(&mut self, ui: &mut Ui, tokens: &Tokens, layout: FooterLayout) -> (bool, bool) {
        let FooterLayout { label, text, legend_text, legend_below, height } = layout;
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        ui.painter().hline(row.x_range(), row.top() - FOOTER_AIR / 2.0, Stroke::new(1.0, tokens.separator));
        let center_y = row.top() + BUTTON_SIZE.y / 2.0;

        let check = Rect::from_min_size(pos2(row.left() + EDGE_AIR, center_y - CHECK_SIZE / 2.0), vec2(CHECK_SIZE, CHECK_SIZE));
        let hit = Rect::from_min_max(pos2(check.left(), row.top()), pos2(check.right() + CHECK_TO_LABEL + text.size().x, row.top() + BUTTON_SIZE.y));
        let response = ui.interact(hit, ui.scope_id().with("all_sets"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, self.all_sets, label));
        if response.clicked() {
            self.all_sets = !self.all_sets;
        }
        paint_check(ui.painter(), tokens, check, Some(self.all_sets), response.hovered());
        ui.painter().galley(pos2(check.right() + CHECK_TO_LABEL, center_y - text.size().y / 2.0), text, tokens.text);
        paint_keyboard_focus(ui, &response, tokens);

        let legend = if legend_below { pos2(check.left(), center_y + BUTTON_SIZE.y + BUTTON_GAP) } else { pos2(hit.right() + LEGEND_GAP, center_y) };
        let glyph = ui.painter().text(legend, Align2::LEFT_CENTER, icon::PROHIBIT, FontId::proportional(SMALL_TEXT_SIZE), tokens.text_dim);
        let text_left = glyph.right() + LEGEND_GLYPH_TO_TEXT;
        ui.painter().galley(pos2(text_left, legend.y - legend_text.size().y / 2.0), legend_text, tokens.text_dim);

        let apply = Rect::from_min_size(pos2(row.right() - EDGE_AIR - BUTTON_SIZE.x, row.bottom() - BUTTON_SIZE.y), BUTTON_SIZE);
        let cancel = apply.translate(vec2(-(BUTTON_SIZE.x + BUTTON_GAP), 0.0));
        (dialog_button(ui, tokens, apply, tr("Apply")), dialog_button(ui, tokens, cancel, tr("Cancel")))
    }
}

/// The footer's texts and rows, worked out before the channel list so the list can leave the footer its height
struct FooterLayout {
    label: &'static str,
    text: Arc<Galley>,
    legend_text: Arc<Galley>,
    legend_below: bool,
    height: f32,
}

impl FooterLayout {
    fn new(ui: &Ui, tokens: &Tokens) -> Self {
        let width = ui.available_width();
        let label = tr("Apply to all Texture Sets");
        let text = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(TEXT_SIZE), tokens.text);
        let legend_text = ui.painter().layout_no_wrap(tr("Not used by the shader").to_owned(), FontId::proportional(SMALL_TEXT_SIZE), tokens.text_dim);

        let left_width = EDGE_AIR + CHECK_SIZE + CHECK_TO_LABEL + text.size().x + LEGEND_GAP + SMALL_TEXT_SIZE + LEGEND_GLYPH_TO_TEXT + legend_text.size().x;

        // A narrow footer puts the buttons on a row of their own, and the legend under the check when the two don't fit side by side
        let legend_below = width < left_width + EDGE_AIR;
        let stacked = legend_below || width < left_width + BUTTON_GAP + 2.0 * BUTTON_SIZE.x + 2.0 * EDGE_AIR;
        let rows = 1.0 + f32::from(u8::from(legend_below)) + f32::from(u8::from(stacked));
        let height = rows * BUTTON_SIZE.y + (rows - 1.0) * BUTTON_GAP;
        FooterLayout { label, text, legend_text, legend_below, height }
    }
}

/// The title and × at the right; returns whether × was clicked
fn title_bar(ui: &mut Ui, tokens: &Tokens, opening: bool) -> bool {
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), TITLE_BAR_HEIGHT), Sense::hover());
    let top = CornerRadius { nw: DIALOG_RADIUS, ne: DIALOG_RADIUS, sw: 0, se: 0 };
    ui.painter().rect_filled(bar, top, tokens.chrome);
    let center_y = bar.center().y;
    let text = Rect::from_min_max(pos2(bar.left() + TITLE_TEXT_LEFT, bar.top()), pos2(bar.right() - TITLE_BAR_HEIGHT, bar.bottom()));
    let label = egui::Label::new(RichText::new(tr("Add or remove channels")).font(semibold(TEXT_SIZE)).color(tokens.text_strong)).truncate();
    ui.scope_builder(egui::UiBuilder::new().max_rect(text).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| ui.add(label));

    let close = Rect::from_center_size(pos2(bar.right() - TITLE_BAR_HEIGHT / 2.0, center_y), vec2(TITLE_BAR_HEIGHT, TITLE_BAR_HEIGHT));
    let response = ui.interact(close, ui.scope_id().with("close"), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr("Close")));
    if opening {
        response.request_focus();
    }
    let color = if response.hovered() { tokens.text_strong } else { tokens.text_dim };
    ui.painter().text(close.center(), Align2::CENTER_CENTER, icon::X, FontId::proportional(TEXT_SIZE), color);
    paint_keyboard_focus(ui, &response, tokens);
    response.clicked()
}

fn dialog_button(ui: &mut Ui, tokens: &Tokens, rect: Rect, text: &str) -> bool {
    let response = ui.interact(rect, ui.scope_id().with(text), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    let fill = if response.hovered() { tokens.row_hover } else { tokens.button };
    ui.painter().rect(rect, CornerRadius::same(DIALOG_RADIUS), fill, Stroke::new(1.0, tokens.button_border), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(TEXT_SIZE), tokens.text);
    paint_keyboard_focus(ui, &response, tokens);
    response.clicked()
}

#[cfg(test)]
mod tests {
    use texturecraft_engine::core::{Channel, TextureSet};

    use super::ChannelManager;

    fn set() -> TextureSet {
        texturecraft_engine::sample_session(128, 64).map(|s| s.doc.project.texture_sets[0].clone()).unwrap()
    }

    #[test]
    fn apply_sends_only_what_the_ticks_changed() {
        let set = set();
        let mut manager = ChannelManager::new(&set, 0, 0, egui::Id::NULL);
        assert_eq!(manager.edits(&set), (vec![], vec![]), "an untouched dialog changes nothing");

        manager.ticked[Channel::Ior.index()] = true;
        manager.ticked[Channel::Height.index()] = false;
        assert!(set.has_channel(Channel::Height));
        assert_eq!(manager.edits(&set), (vec![Channel::Ior], vec![Channel::Height]));
    }

    #[test]
    fn apply_to_all_sets_sends_the_whole_list_but_never_removes_base_color() {
        // Another set may lack a channel this one has: only the full list makes every set match
        let set = set();
        let mut manager = ChannelManager::new(&set, 0, 0, egui::Id::NULL);
        manager.all_sets = true;
        let (add, remove) = manager.edits(&set);
        assert!(set.channels.iter().all(|(c, _)| add.contains(c)));
        assert_eq!(add.len() + remove.len(), Channel::ALL.len(), "every channel is sent one way or the other");
        assert!(!remove.contains(&Channel::BaseColor) && !remove.iter().any(|c| set.has_channel(*c)));
    }
}
