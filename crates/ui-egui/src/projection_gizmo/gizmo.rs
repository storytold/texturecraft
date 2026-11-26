use egui::{Pos2, Rect};
use serde_json::Value;

use crate::TextureCraftApp;

/// A projection gizmo whose handles a drag edits
pub(crate) trait Gizmo: Clone + Send + Sync + 'static {
    /// What a drag on one of its handles does
    type Grab: Copy + PartialEq + Send + Sync + 'static;

    /// Where a drag on it is kept in egui's memory
    fn drag_id() -> egui::Id;

    /// The handle under screen point `at`
    fn grab(&self, at: Pos2) -> Option<Self::Grab>;

    /// Whether `other` shows the same projection: a drag goes on only while the projection it began on shows
    fn same_projection(&self, other: &Self) -> bool;

    /// The command and params a drag of `grab` from `from` to `to` runs
    fn edit(&self, app: &TextureCraftApp, rect: Rect, grab: Self::Grab, from: Pos2, to: Pos2) -> Option<(&'static str, Value)>;

    /// As [`Gizmo::edit`], the drag snapping to the tool bar's steps while `snapping` (a gizmo with none ignores it)
    fn edit_with_snap(&self, app: &TextureCraftApp, rect: Rect, grab: Self::Grab, from: Pos2, to: Pos2, _snapping: bool) -> Option<(&'static str, Value)> {
        self.edit(app, rect, grab, from, to)
    }

    /// The cursor over `grab`, or dragging it
    fn cursor(&self, grab: Self::Grab, dragging: bool) -> egui::CursorIcon;
}
