//! The active tab in a panel group, drawn as a raised box joined to the panel body

use craft_ui::docking::{DockContent, Node};
use egui::text::TextWrapping;
use egui::{Rect, Stroke, StrokeKind, Ui, pos2};

use super::header_band::{HEADER_RISE, header_name, under_a_panel};
use super::{HEADER_TITLE_SIZE, Panel, Panels, area_id};
use crate::property_sheet::{full_height_tab_rect, shown_tab_box};
use crate::theme::semibold;

/// Horizontal padding of craft-ui's dock tab, in points
const DOCK_TAB_PADDING: f32 = 24.0;

/// From a panel's left edge to its header's name
pub(crate) const HEADER_TITLE_LEFT: f32 = DOCK_TAB_PADDING / 2.0;

/// Gap between the active tab's box and its neighbours, in points
const TAB_BOX_INSET: f32 = 4.0;

/// Insets the focus outline inside the tab's actual hit rect
const TAB_FOCUS_INSET: f32 = 2.0;

/// Width of the focus outline in points
const TAB_FOCUS_STROKE_WIDTH: f32 = 1.0;

/// The tab group containing `panel`; a collapsed stack has no tabs
fn group_of(node: &Node<Panel>, panel: Panel) -> Option<&[Panel]> {
    match node {
        Node::Split { first, second, .. } => group_of(first, panel).or_else(|| group_of(second, panel)),
        Node::Tabs { panels, .. } if panels.contains(&panel) => Some(panels),
        Node::Dock { columns, .. } => {
            columns.iter().flat_map(|column| &column.groups).find(|group| group.panels.contains(&panel)).map(|group| group.panels.as_slice())
        }
        _ => None,
    }
}

impl Panels<'_> {
    /// How far a header band reaches above its tab strip
    fn header_rise(&self, panel: Panel) -> f32 {
        if under_a_panel(self.layout, panel) { HEADER_RISE } else { 0.0 }
    }

    /// How far a header name sits above the tab strip's centre
    pub(super) fn header_lift(&self, panel: Panel) -> f32 {
        self.header_rise(panel) / 2.0
    }

    /// Paints the active tab and its focus outline above the renderer's fallback paint
    pub(super) fn paint_shown_group_tab(&self, ui: &Ui, panel: Panel) {
        let layout = self.layout;
        let in_group = layout
            .root
            .iter()
            .chain(layout.floating.iter().map(|window| &window.root))
            .find_map(|root| group_of(root, panel))
            .is_some_and(|group| group.len() > 1);
        if !in_group {
            return;
        }

        // The shared renderer submits the active tab before it calls the panel body
        let tab_id = area_id().with(("tab", &panel));
        let Some(tab) = ui.ctx().read_response(tab_id) else { return };
        let body = ui.max_rect();
        let tab_rect = tab.rect;
        if tab.layer_id != ui.layer_id() || tab_rect.bottom() != body.top() || tab_rect.left() < body.left() || tab_rect.right() > body.right() {
            return;
        }

        let band_top = tab_rect.top() - self.header_rise(panel);
        let box_left = if tab_rect.left() > body.left() { tab_rect.left() + TAB_BOX_INSET } else { tab_rect.left() };
        let box_right = tab_rect.right() - TAB_BOX_INSET;
        if box_left >= box_right {
            return;
        }

        // Extend the body's clip over the header without widening it
        let clip = ui.clip_rect();
        let mut painter = ui.painter().clone();
        painter.set_clip_rect(Rect::from_x_y_ranges(clip.x_range(), band_top..=clip.bottom()));
        let band = Rect::from_x_y_ranges(body.x_range(), band_top..=tab_rect.bottom());
        shown_tab_box(&painter, &self.tokens, full_height_tab_rect(band, box_left, box_right));

        let name = self.logical_label(&panel).to_uppercase();
        let font = semibold(HEADER_TITLE_SIZE);
        let mut job = header_name(ui, &name, font, self.style.text, (tab_rect.top() - band_top) / 2.0);
        job.wrap = TextWrapping::truncate_at_width((tab_rect.width() - DOCK_TAB_PADDING).max(0.0));
        let galley = painter.layout_job(job);
        let at = pos2(tab_rect.left() + HEADER_TITLE_LEFT, tab_rect.center().y - galley.size().y / 2.0);
        painter.galley(at, galley, self.style.text);

        // Preserve the shared dock's keyboard focus ring above the opaque selected tab
        let pointer_focus = ui.data(|data| data.get_temp::<bool>(tab_id.with("pointer-focus"))).unwrap_or(false);
        let focus_rect = tab_rect.shrink(TAB_FOCUS_INSET);
        if tab.has_focus() && !pointer_focus && focus_rect.is_positive() {
            painter.rect_stroke(focus_rect, 0.0, Stroke::new(TAB_FOCUS_STROKE_WIDTH, self.style.accent), StrokeKind::Inside);
        }
    }
}
