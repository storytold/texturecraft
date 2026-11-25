//! A settings panel as one scrolling page: its section tabs jump to a section, and scrolling highlights the section in view

use egui::Ui;

use crate::i18n::tr;
use crate::theme::caption;
use crate::widgets::icon_button;

mod page_section;
mod page_state;

pub(crate) use page_section::PageSection;
use page_state::PageState;

/// Size of a section tab
const TAB_SIZE: f32 = 28.0;

/// A section counts as reached once its heading is this close to the top of the view, in points
const REACHED_TOLERANCE: f32 = 4.0;

/// Space above every section heading after the first
const SECTION_GAP: f32 = 10.0;

/// The section tabs, then the sections in one scroll area; `body` draws section `index` under its heading
pub(crate) fn show(ui: &mut Ui, id_salt: &str, sections: &[PageSection], mut body: impl FnMut(&mut Ui, usize)) {
    let id = ui.make_persistent_id(("section_page", id_salt));
    let mut state = ui.data(|data| data.get_temp::<PageState>(id)).unwrap_or_default();

    ui.horizontal(|ui| {
        for (index, section) in sections.iter().enumerate() {
            if icon_button(ui, section.icon, tr(section.title), state.active == index, TAB_SIZE).clicked() {
                state.jump = Some(index);
            }
        }
    });
    ui.separator();

    let mut area = egui::ScrollArea::vertical().id_salt(id.with("scroll")).auto_shrink([false, true]);
    let jumped = state.jump.take().and_then(|index| state.tops.get(index).map(|top| (index, *top)));
    if let Some((_, top)) = jumped {
        area = area.vertical_scroll_offset(top);
    }

    let output = area.show(ui, |ui| {
        let origin = ui.min_rect().top();
        let mut tops = Vec::with_capacity(sections.len());
        for (index, section) in sections.iter().enumerate() {
            if index > 0 {
                ui.add_space(SECTION_GAP);
                ui.separator();
            }
            tops.push(ui.cursor().top() - origin);
            ui.label(caption(&tr(section.title).to_uppercase()));
            body(ui, index);
        }
        tops
    });

    let offset = output.state.clamped_offset().y;
    let bottom = (output.content_size.y - output.inner_rect.height()).max(0.0);
    state.tops = output.inner;
    if let Some((index, _)) = jumped {
        state.pinned = Some((index, offset));
    }

    state.active = active_section(&state.tops, offset, bottom, &mut state.pinned);
    ui.data_mut(|data| data.insert_temp(id, state));
}

/// The section whose heading the view has passed last, the last one once the view is at the bottom, or a clicked one while the view stays put
fn active_section(tops: &[f32], offset: f32, bottom: f32, pinned: &mut Option<(usize, f32)>) -> usize {
    if let Some((index, pinned_offset)) = *pinned {
        if (offset - pinned_offset).abs() <= REACHED_TOLERANCE && index < tops.len() {
            return index;
        }

        *pinned = None;
    }

    let last = tops.len().saturating_sub(1);
    if bottom > 0.0 && offset >= bottom - REACHED_TOLERANCE {
        return last;
    }

    tops.iter().rposition(|top| *top <= offset + REACHED_TOLERANCE).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_section_the_view_passed_last_is_active() {
        let tops = [0.0, 300.0, 700.0];
        let mut pinned = None;
        assert_eq!(active_section(&tops, 0.0, 900.0, &mut pinned), 0);
        assert_eq!(active_section(&tops, 299.0, 900.0, &mut pinned), 1, "within the tolerance");
        assert_eq!(active_section(&tops, 500.0, 900.0, &mut pinned), 1);
        assert_eq!(active_section(&tops, 900.0, 900.0, &mut pinned), 2, "the bottom shows the last section");
    }

    #[test]
    fn a_clicked_section_stays_active_until_the_view_moves() {
        let tops = [0.0, 300.0, 700.0];

        // The page cannot scroll to the last heading: the jump stops at the bottom, 400
        let mut pinned = Some((2, 400.0));
        assert_eq!(active_section(&tops, 400.0, 0.0, &mut pinned), 2);
        assert_eq!(active_section(&tops, 100.0, 400.0, &mut pinned), 0);
        assert!(pinned.is_none(), "scrolling away ends the pin");
    }
}
