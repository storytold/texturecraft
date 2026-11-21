//! A docked header's band: the tab strip plus the gutter above it, less a hairline of separation, with the names' capitals centred on it

use craft_ui::docking::{Layout, Node};
use craft_ui::layout::SplitAxis;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, FontId, Ui};

use super::Panel;

/// Gutter between two stacked panels left showing between the upper panel and the header band under it, as the reference keeps a dark line there
const PANEL_SEPARATION: f32 = 2.0;

/// How far a header under another panel reaches above its tab strip: the gutter between them, less the separation
pub(super) const HEADER_RISE: f32 = crate::theme::GUTTER as f32 - PANEL_SEPARATION;

/// Whether `panel`'s docked group sits under another panel, a gutter above its header; floating groups and the top row have none
pub(super) fn under_a_panel(layout: &Layout<Panel>, panel: Panel) -> bool {
    let Some(root) = &layout.root else { return false };

    let mut nodes: Vec<(&Node<Panel>, bool)> = vec![(root, false)];
    while let Some((node, below)) = nodes.pop() {
        match node {
            Node::Split { axis, first, second, .. } => {
                nodes.push((first, below));
                nodes.push((second, below || *axis == SplitAxis::Vertical));
            }
            Node::Tabs { panels, .. } if panels.contains(&panel) => return below,
            Node::Dock { columns, .. } => {
                for column in columns {
                    if let Some(index) = column.groups.iter().position(|group| group.panels.contains(&panel)) {
                        return below || index > 0;
                    }
                }
            }
            _ => {}
        }
    }

    false
}

/// `name` laid out so that, centred as the dock centres every name, its capitals' middle sits `lift` above the strip's (a galley's height counts descenders)
pub(super) fn header_name(ui: &Ui, name: &str, font: FontId, color: Color32, lift: f32) -> LayoutJob {
    let format = TextFormat { font_id: font, color, valign: Align::TOP, ..Default::default() };
    let mut job = LayoutJob::single_section(name.to_owned(), format);
    let galley = ui.painter().layout_job(job.clone());

    // The first capital's ink; a name without one (a script without capitals) keeps the galley's middle
    let capital = galley.rows.first().and_then(|row| row.glyphs.iter().find(|glyph| glyph.chr.is_uppercase()).map(|glyph| (row.pos.y, glyph)));
    let ink_middle = capital.map_or(galley.size().y / 2.0, |(row_top, glyph)| row_top + glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y / 2.0);

    // Room below the name raises it when centred, room above lowers it
    let shift = ink_middle - galley.size().y / 2.0 + lift;
    if shift < 0.0
        && let Some(section) = job.sections.first_mut()
    {
        section.format.valign = Align::BOTTOM;
    }

    job.first_row_min_height = galley.size().y + 2.0 * shift.abs();
    job
}
