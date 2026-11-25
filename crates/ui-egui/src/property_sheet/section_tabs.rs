use egui::{Align2, Color32, CornerRadius, FontId, Id, Painter, Rect, Role, Sense, Ui, WidgetInfo, pos2, vec2};

use super::row::SECTION_RULE_TO_CAPTION;
use super::{SIDE_PADDING_POINTS, SMALL_TEXT_SIZE};
use crate::theme::{Tokens, semibold};

mod section_scope;
mod section_tops;

pub(crate) use section_scope::SectionScope;
pub(super) use section_tops::SectionTops;

/// Height of the tab strip
const STRIP_HEIGHT: f32 = 30.0;

/// How far the first tab reaches left of the rows' side padding
const FIRST_TAB_OUTDENT: f32 = 4.0;

/// From the panel's left edge to the first tab
const FIRST_TAB: f32 = SIDE_PADDING_POINTS - FIRST_TAB_OUTDENT;

/// A tab's width beyond its name: icon and air (the resource picker's tabs share these measures)
pub(crate) const TAB_PADDING: f32 = 38.0;

/// Air between tabs
pub(crate) const TAB_GAP: f32 = 2.0;

/// The chosen tab starts this far below the strip's top and reaches this far into the panel
pub(crate) const TAB_TOP: f32 = 4.0;
const TAB_BELOW: f32 = 6.0;

/// Corner rounding of the chosen tab
pub(crate) const TAB_ROUNDING: u8 = 5;

/// From a tab's left edge to its icon's centre, and to its name
pub(crate) const TAB_ICON_CENTER: f32 = 14.0;
pub(crate) const TAB_NAME_LEFT: f32 = 26.0;

/// Size of a tab's icon and of its name
pub(crate) const TAB_ICON_SIZE: f32 = 13.0;
pub(crate) const TAB_TEXT_SIZE: f32 = SMALL_TEXT_SIZE;

/// A section counts as in view once its caption is this close to the top of the scrolled area
const IN_VIEW_SLACK: f32 = 30.0;

/// Where the scope of the sections being drawn is kept while a strip's contents run
fn scope_id() -> Id {
    Id::unique("property_sheet_section_scope")
}

/// Where a strip keeps its section positions and the section one of its tabs asked to jump to
fn sections_id(scope: SectionScope) -> Id {
    Id::unique("property_sheet_sections").with(scope)
}

fn jump_id(scope: SectionScope) -> Id {
    Id::unique("property_sheet_jump").with(scope)
}

/// Runs `body` with its section captions tied to the `scope` strip
pub(crate) fn in_scope<R>(ui: &mut Ui, scope: SectionScope, body: impl FnOnce(&mut Ui) -> R) -> R {
    let outer = ui.data(|data| data.get_temp::<SectionScope>(scope_id()));
    ui.data_mut(|data| data.insert_temp(scope_id(), scope));
    let result = body(ui);
    ui.data_mut(|data| data.insert_temp(scope_id(), outer.unwrap_or(SectionScope::Docked)));
    result
}

/// The strip the sections being drawn belong to: the docked panel's unless a caller said otherwise
fn current_scope(ui: &Ui) -> SectionScope {
    ui.data(|data| data.get_temp::<SectionScope>(scope_id())).unwrap_or(SectionScope::Docked)
}

/// Stores this frame's section captions for the tabs
pub(super) fn remember(ui: &Ui, tops: SectionTops) {
    let scope = current_scope(ui);
    ui.data_mut(|data| data.insert_temp(sections_id(scope), tops));
}

/// Drops the captions the sheets stored last frame, so a list drawn without sections leaves none behind; call before drawing the list
pub(crate) fn forget_sections(ui: &Ui) {
    let scope = current_scope(ui);
    ui.data_mut(|data| data.remove::<SectionTops>(sections_id(scope)));
}

/// Room to leave under the sheets just drawn in `ui`'s scroll area so the last section's top can reach the top of the view
pub(crate) fn room_past_end(ui: &Ui) -> f32 {
    let scope = current_scope(ui);
    let last_top = ui.data(|data| data.get_temp::<SectionTops>(sections_id(scope))).and_then(|tops| tops.last_top());
    let Some(last_top) = last_top else { return 0.0 };

    // Where a jump to the section scrolls to: its hairline at the top
    let reach = last_top - SECTION_RULE_TO_CAPTION;
    (reach + ui.clip_rect().height() - ui.min_rect().bottom()).max(0.0)
}

/// Whether a tab asked to show `title`: the caller scrolls to it once
pub(super) fn take_jump(ui: &Ui, title: &str) -> bool {
    let jump = jump_id(current_scope(ui));
    let wanted = ui.data(|data| data.get_temp::<String>(jump)).is_some_and(|wanted| wanted == title);
    if wanted {
        ui.data_mut(|data| data.remove::<String>(jump));
    }
    wanted
}

/// A tab from `left` to `right` on `strip`: from just under the strip's top down into the panel the strip sits on
fn tab_rect(strip: Rect, left: f32, right: f32) -> Rect {
    Rect::from_min_max(pos2(left, strip.top() + TAB_TOP), pos2(right, strip.bottom() + TAB_BELOW))
}

/// A tab from `left` to `right` filling `strip`'s whole height and reaching as far into the panel as a section tab: a docked group's shown tab
pub(crate) fn full_height_tab_rect(strip: Rect, left: f32, right: f32) -> Rect {
    Rect::from_min_max(pos2(left, strip.top()), pos2(right, strip.bottom() + TAB_BELOW))
}

/// The shown tab's box over `tab`: the panel's colour, rounded, so it rises from the panel below the strip
pub(crate) fn shown_tab_box(painter: &Painter, tokens: &Tokens, tab: Rect) {
    painter.rect_filled(tab, CornerRadius::same(TAB_ROUNDING), tokens.panel);
}

/// The tab strip on the `scope` panel's top edge, top corners rounded by `top_corner`: one tab per section; a click scrolls to it
pub(crate) fn section_tabs(ui: &mut Ui, scope: SectionScope, top_corner: u8, tabs: &[(&str, &str)]) {
    let tokens = Tokens::get(ui.ctx());
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), STRIP_HEIGHT), Sense::hover());
    let corners = CornerRadius { nw: top_corner, ne: top_corner, sw: 0, se: 0 };
    ui.painter().rect_filled(strip, corners, tokens.chrome);
    let seen = ui.data(|data| data.get_temp::<SectionTops>(sections_id(scope))).unwrap_or_default();
    let lit = seen.lit();

    let mut x = strip.left() + FIRST_TAB;
    for (index, (icon, title)) in tabs.iter().enumerate() {
        let on = lit.map_or(index == 0, |lit| lit == *title);
        let width = ui.painter().layout_no_wrap((*title).to_owned(), semibold(TAB_TEXT_SIZE), Color32::WHITE).size().x + TAB_PADDING;
        let tab = tab_rect(strip, x, x + width);
        let response = ui.interact(tab, ui.scope_id().with(("section_tab", index)), Sense::click());
        response.widget_info(|| WidgetInfo::selected(Role::Tab, true, on, *title));
        if response.clicked() {
            ui.data_mut(|data| data.insert_temp(jump_id(scope), (*title).to_owned()));
        }

        let color = if on || response.hovered() { tokens.text } else { tokens.text_dim };
        let painter = ui.painter();
        if on {
            shown_tab_box(painter, &tokens, tab);
        }
        let center_y = strip.center().y + TAB_TOP / 2.0;
        painter.text(pos2(x + TAB_ICON_CENTER, center_y), Align2::CENTER_CENTER, *icon, FontId::proportional(TAB_ICON_SIZE), color);
        let font = if on { semibold(TAB_TEXT_SIZE) } else { FontId::proportional(TAB_TEXT_SIZE) };
        painter.text(pos2(x + TAB_NAME_LEFT, center_y), Align2::LEFT_CENTER, *title, font, color);
        if response.has_focus() {
            super::switch_control::paint_focus_outline(painter, response.rect, tokens.text_strong);
        }

        x += width + TAB_GAP;
    }
}
