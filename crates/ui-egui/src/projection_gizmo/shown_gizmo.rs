use egui::{Pos2, Rect, Ui};

use super::{Gizmo, dragging, let_go};
use crate::TextureCraftApp;
use crate::frame_gizmo::FrameGizmo;
use crate::round_gizmo::RoundGizmo;

/// The selected fill's projection gizmo the 3D view shows, built once a frame: a planar or warp frame, or a round projector
pub(crate) enum ShownGizmo {
    // Both boxed: each gizmo is hundreds of bytes and of a different size
    Frame(Box<FrameGizmo>),
    Round(Box<RoundGizmo>),
}

impl ShownGizmo {
    /// The gizmo of the selected fill's projection: none while the Projection row's eye hides it
    pub(crate) fn of(app: &TextureCraftApp, rect: Rect) -> Option<ShownGizmo> {
        if !app.session.doc.project.display.projection_gizmos {
            return None;
        }

        FrameGizmo::of(app, rect)
            .map(|frame| ShownGizmo::Frame(Box::new(frame)))
            .or_else(|| RoundGizmo::of(app, rect).map(|round| ShownGizmo::Round(Box::new(round))))
    }

    fn frame(gizmo: Option<&ShownGizmo>) -> Option<&FrameGizmo> {
        match gizmo? {
            ShownGizmo::Frame(frame) => Some(frame),
            ShownGizmo::Round(_) => None,
        }
    }

    fn round(gizmo: Option<&ShownGizmo>) -> Option<&RoundGizmo> {
        match gizmo? {
            ShownGizmo::Round(round) => Some(round),
            ShownGizmo::Frame(_) => None,
        }
    }

    /// Drags on `gizmo`'s handles; returns whether a drag owns the pointer
    pub(crate) fn input(app: &mut TextureCraftApp, ui: &Ui, response: &egui::Response, rect: Rect, gizmo: Option<&ShownGizmo>) -> bool {
        let ctx = ui.ctx();
        super::input(app, ctx, response, rect, ShownGizmo::frame(gizmo)) || super::input(app, ctx, response, rect, ShownGizmo::round(gizmo))
    }

    /// Ends a drag under way on either gizmo where it is
    pub(crate) fn let_go(ctx: &egui::Context) {
        let_go::<FrameGizmo>(ctx);
        let_go::<RoundGizmo>(ctx);
    }

    /// Whether screen point `at` is on one of its handles
    pub(crate) fn on_handle(&self, rect: Rect, at: Pos2) -> bool {
        rect.contains(at)
            && match self {
                ShownGizmo::Frame(frame) => frame.grab(at).is_some(),
                ShownGizmo::Round(round) => round.grab(at).is_some(),
            }
    }

    pub(crate) fn dragging(ctx: &egui::Context) -> bool {
        dragging::<FrameGizmo>(ctx).is_some() || dragging::<RoundGizmo>(ctx).is_some()
    }

    /// Whether the pointer is over one of `gizmo`'s handles or dragging one (the brush cursor hides there)
    pub(crate) fn hovered(ui: &Ui, rect: Rect, gizmo: Option<&ShownGizmo>) -> bool {
        let ctx = ui.ctx();
        Self::dragging(ctx) || ctx.pointer_hover_pos().is_some_and(|hover| gizmo.is_some_and(|gizmo| gizmo.on_handle(rect, hover)))
    }

    /// The gizmo over the 3D view
    pub(crate) fn overlay(&self, app: &TextureCraftApp, ui: &Ui, rect: Rect) {
        match self {
            ShownGizmo::Frame(frame) => crate::frame_gizmo::overlay(ui, rect, frame),
            ShownGizmo::Round(round) => crate::round_gizmo::overlay(app, ui, rect, round),
        }
    }
}
