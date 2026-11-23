use super::IN_VIEW_SLACK;

/// The sections' captions seen last frame, top to bottom, and the top of the area they scroll in
#[derive(Clone, Default)]
pub(crate) struct SectionTops {
    pub(crate) tops: Vec<(String, f32)>,
    pub(crate) view_top: f32,
}

impl SectionTops {
    /// The section in view: the last caption at the top (the panel scrolls far enough for the last one to get there)
    pub(crate) fn lit(&self) -> Option<&str> {
        let reached = self.tops.iter().rev().find(|(_, top)| *top <= self.view_top + IN_VIEW_SLACK);
        reached.or(self.tops.first()).map(|(title, _)| title.as_str())
    }

    /// The top of the last section's caption
    pub(crate) fn last_top(&self) -> Option<f32> {
        self.tops.last().map(|(_, top)| *top)
    }
}
