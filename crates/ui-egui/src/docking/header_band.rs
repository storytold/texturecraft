//! Positions dock tab titles within the tab strip and the gutter above it.

use craft_ui::docking::{Layout, Node};
use craft_ui::layout::SplitAxis;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, FontId, Ui};

use super::Panel;

/// Gap left between a header band and the panel above it, in points.
const HEADER_SEPARATION: f32 = 2.0;

/// How far a header under another panel reaches above its tab strip: the gutter between them, less the separation
pub(super) const HEADER_RISE: f32 = crate::theme::GUTTER as f32 - HEADER_SEPARATION;

/// Whether `panel`'s docked group sits under another panel, a gutter above its header; floating groups and the top row have none
pub(super) fn under_a_panel(layout: &Layout<Panel>, panel: Panel) -> bool {
    layout.root.as_ref().and_then(|node| group_is_below_panel(node, panel, false)).unwrap_or(false)
}

fn group_is_below_panel(node: &Node<Panel>, panel: Panel, below_panel: bool) -> Option<bool> {
    match node {
        Node::Split { axis, first, second, .. } => {
            group_is_below_panel(second, panel, below_panel || *axis == SplitAxis::Vertical).or_else(|| group_is_below_panel(first, panel, below_panel))
        }
        Node::Tabs { panels, .. } if panels.contains(&panel) => Some(below_panel),
        Node::Dock { columns, .. } => {
            columns.iter().find_map(|column| column.groups.iter().position(|group| group.panels.contains(&panel))).map(|index| below_panel || index > 0)
        }
        _ => None,
    }
}

/// `name` laid out so that, centred as the dock centres every name, its capitals' middle sits `lift` above the strip's (a galley's height counts descenders)
pub(super) fn header_name(ui: &Ui, name: &str, font: FontId, color: Color32, lift_points: f32) -> LayoutJob {
    let format = TextFormat { font_id: font, color, valign: Align::TOP, ..Default::default() };
    let mut job = LayoutJob::single_section(name.to_owned(), format);
    let galley = ui.painter().layout_job(job.clone());

    // Scripts without capitals use the whole galley as their ink bounds
    let capital = galley.rows.first().and_then(|row| row.glyphs.iter().find(|glyph| glyph.chr.is_uppercase()).map(|glyph| (row.pos.y, glyph)));
    let ink_middle = capital.map_or(galley.size().y / 2.0, |(row_top, glyph)| row_top + glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y / 2.0);

    // Room below the name raises it when centred, room above lowers it
    let shift = ink_middle - galley.size().y / 2.0 + lift_points;
    if shift < 0.0
        && let Some(section) = job.sections.first_mut()
    {
        section.format.valign = Align::BOTTOM;
    }

    job.first_row_min_height = galley.size().y + 2.0 * shift.abs();
    job
}
