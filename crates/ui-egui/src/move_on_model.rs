//! Move on model: hold the preview square and drag to slide the selected fill's projection, with Shift or the right button held to turn it, and turn the wheel to scale it. The pointer is held while it lasts (hidden, no screen edge stops it, back where the press began when let go). Letting go ends it, as one undo step; the 3D view keeps its own drags and wheel

use egui::{CursorIcon, Pos2, Rect};
use egui_phosphor::regular as ic;
use serde_json::{Value, json};
use texturecraft_engine::StackTarget;
use texturecraft_engine::core::{EffectKind, LayerId, LayerKind, Projection};

use crate::TextureCraftApp;
use crate::frame_gizmo::FrameGizmo;
use crate::i18n::tr;
use crate::move_hold::MoveHold;
use crate::property_sheet::PropertySheet;
use crate::round_gizmo::RoundGizmo;

/// The fill whose projection Move on model moves: a fill layer, or a content fill effect in it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveOnModel {
    pub layer: LayerId,
    pub effect: Option<LayerId>,
}

impl MoveOnModel {
    /// A fill layer's own projection
    pub(crate) fn layer(layer: LayerId) -> MoveOnModel {
        MoveOnModel { layer, effect: None }
    }

    /// The `layer` or `effect` param of the fill commands
    pub(crate) fn target(self) -> Value {
        match self.effect {
            Some(effect) => json!({"effect": effect.0}),
            None => json!({"layer": self.layer.0}),
        }
    }

    /// The selected fill layer, or the selected fill effect in it (its content or its mask); its projector shows in the 3D view
    pub(crate) fn selected(app: &TextureCraftApp) -> Option<MoveOnModel> {
        let layer = app.session.selected_layer()?;
        match app.session.target {
            StackTarget::Content => Some(Self::layer(app.session.project().content_layer(layer)?.id)),
            StackTarget::Effect(effect) => Some(MoveOnModel { layer, effect: Some(effect) }),
            StackTarget::Mask => None,
        }
    }

    /// Resolves the projection and its edit command from the target's effective stacks
    pub(crate) fn projection_editor(self, app: &TextureCraftApp) -> Option<(&Projection, &'static str)> {
        let project = app.session.project();
        let content = project.content_layer(self.layer)?;
        if let Some(effect_id) = self.effect {
            let mask_owner = project.layer(project.mask_owner(self.layer)).ok()?;
            let effect = mask_owner.mask.iter().flat_map(|mask| &mask.effects).chain(&content.content_effects).find(|effect| effect.id == effect_id)?;
            return match &effect.kind {
                EffectKind::Material(fill) => Some((&fill.projection, "layer.fill.projection")),
                EffectKind::Fill { projection, .. } => Some((projection, "effect.set")),
                _ => None,
            };
        }

        match &content.kind {
            LayerKind::Fill(fill) => Some((&fill.projection, "layer.fill.projection")),
            _ => None,
        }
    }
}

fn hold_id() -> egui::Id {
    egui::Id::unique("tc_move_on_model_hold")
}

/// Image repeats a drag across the view's height slides by
const SLIDE_PER_HEIGHT: f32 = 1.0;
/// Degrees a turning drag turns per point
const TURN_PER_POINT: f32 = 0.5;
/// Growth per wheel point (exponential)
const SCALE_PER_WHEEL_POINT: f32 = 0.0015;

/// Move on model on `target`'s preview square: while it is held down it moves the projection
pub(crate) fn pad(app: &mut TextureCraftApp, sheet: &mut PropertySheet<'_>, square: Rect, target: MoveOnModel) {
    let held = app.moving_on_model == Some(target);
    let mut response = sheet.hold_pad(square, ic::HAND, tr("Move on model"), held);
    if !held {
        response = sheet.tip_beside(response, tr("Hold and drag to slide the projection; Shift or the right button turns it, the wheel scales it"));
    }
    let ctx = sheet.ui.ctx().clone();
    hold(app, &ctx, &response, target);
}

/// Whether a Move on model hold is under way, or let go this frame (that release is no click in the view); the 3D view then keeps its camera, brush and gizmos still. Ends a hold whose window lost the focus
pub(crate) fn holding(app: &mut TextureCraftApp, ctx: &egui::Context) -> bool {
    let held = app.moving_on_model.is_some();
    if held && !still_pressed(ctx) {
        end(app, ctx);
    }
    held
}

/// Ends a hold whose fill or pad went away, once a frame whichever views show
pub(crate) fn end_when_gone(app: &mut TextureCraftApp, ctx: &egui::Context) {
    let Some(target) = app.moving_on_model else { return };

    if target.projection_editor(app).is_none() || pad_gone(ctx) {
        end(app, ctx);
    }
}

/// Whether the held pad was not drawn last pass nor this one (its layer was deselected, its panel hidden)
fn pad_gone(ctx: &egui::Context) -> bool {
    let last_drawn = ctx.data(|data| data.get_temp::<MoveHold>(hold_id())).map(|held| held.pass);
    last_drawn.is_none_or(|pass| pass.saturating_add(1) < ctx.cumulative_pass_nr())
}

/// Whether the left button still holds: a window that lost the focus gets no release, so the hold ends there as on one
fn still_pressed(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.pointer.primary_down()) && !crate::viewport::focus_lost(ctx)
}

/// Ends the hold where it is: its edits stay one undo step
fn end(app: &mut TextureCraftApp, ctx: &egui::Context) {
    app.moving_on_model = None;
    ctx.data_mut(|data| data.remove::<MoveHold>(hold_id()));
}

/// A press on the pad starts the hold; each frame until the left button lets go, pointer moves and wheel steps edit the projection
fn hold(app: &mut TextureCraftApp, ctx: &egui::Context, pad: &egui::Response, target: MoveOnModel) {
    let primary = still_pressed(ctx);
    let (pressed, turn, pointer) =
        ctx.input(|input| (input.pointer.primary_pressed(), input.modifiers.shift || input.pointer.secondary_down(), input.pointer.latest_pos()));
    let pass = ctx.cumulative_pass_nr();
    let mut held = ctx.data(|data| data.get_temp::<MoveHold>(hold_id())).filter(|held| held.target == target);

    // Only a fresh press starts it: after a focus loss egui still has the left button down
    if held.is_none()
        && primary
        && pressed
        && pad.is_pointer_button_down_on()
        && let Some(at) = pointer
    {
        held = Some(MoveHold { target, last: at, edited_revision: None, pass: pass.wrapping_sub(1) });
    }

    let Some(mut held) = held.filter(|_| primary && target.projection_editor(app).is_some()) else {
        if app.moving_on_model == Some(target) {
            end(app, ctx);
        }
        if pad.hovered() {
            ctx.set_cursor_icon(CursorIcon::Grab);
        }
        return;
    };

    app.moving_on_model = Some(target);
    crate::pointer_lock::ask_for_hold(ctx);
    if held.pass == pass {
        return;
    }

    held.pass = pass;

    let rect = app.viewport_rect;
    if let Some(at) = pointer.filter(|at| *at != held.last) {
        if let Some((command, params)) = dragged(app, target, held.last, at, turn, rect) {
            step(app, &mut held, command, params);
        }
        held.last = at;
    }

    // The wheel scales the projection instead of scrolling the panel or zooming the camera while the pad is held
    let wheel = ctx.input_mut(|input| std::mem::take(&mut input.smooth_scroll_delta).y);
    if wheel != 0.0
        && let Some((command, params)) = resized(app, target, (SCALE_PER_WHEEL_POINT * wheel).exp(), rect)
    {
        step(app, &mut held, command, params);
    }

    ctx.data_mut(|data| data.insert_temp(hold_id(), held));

    // The panel drew before this edit: one more frame shows its fields and preview where the projection is now
    ctx.request_repaint();
}

/// Runs one edit of the hold; the edits after the first that changed something join its undo step
fn step(app: &mut TextureCraftApp, held: &mut MoveHold, command: &str, params: Value) {
    crate::projection_gizmo::edit_in_one_step(&mut app.session, command, params, &mut held.edited_revision);
}

/// The selected planar or warp frame of a fill layer or of a mask fill effect, which moves by its frame
fn frame_of(app: &TextureCraftApp, target: MoveOnModel, rect: Rect) -> Option<FrameGizmo> {
    FrameGizmo::of(app, rect).filter(|gizmo| match target.effect {
        Some(effect) => gizmo.effect == Some(effect),
        None => gizmo.layer == target.layer && gizmo.effect.is_none(),
    })
}

/// The `layer` (and for a mask fill `effect`) params of `layer.decal.set` editing `gizmo`'s frame, added to `params`
fn frame_target(mut params: Value, gizmo: &FrameGizmo) -> Value {
    params["layer"] = json!(gizmo.layer.0);
    if let Some(effect) = gizmo.effect {
        params["effect"] = json!(effect.0);
    }
    params
}

/// A projection's UV tiling, rotation and offset (None for Fill (Match per UV Tile))
fn placement(projection: &Projection) -> Option<([f32; 2], f32, [f32; 2])> {
    match projection {
        Projection::UvTile => None,
        Projection::Uv { scale, rotation, offset } => Some((*scale, *rotation, *offset)),
        Projection::Triplanar { uv, .. }
        | Projection::Planar { uv, .. }
        | Projection::Spherical { uv, .. }
        | Projection::Cylindrical { uv, .. }
        | Projection::Warp { uv, .. } => Some((uv.tiling, uv.rotation, uv.offset)),
    }
}

/// The edit a pointer move from `from` to `to` makes: a frame or projector slides by it, a UV image slides; with `turn` it turns instead
fn dragged(app: &TextureCraftApp, target: MoveOnModel, from: Pos2, to: Pos2, turn: bool, rect: Rect) -> Option<(&'static str, Value)> {
    let delta = to - from;
    if let Some(gizmo) = frame_of(app, target, rect) {
        let params = if turn { json!({"rotation": gizmo.decal.rotation - delta.x * TURN_PER_POINT}) } else { json!({"offset": gizmo.slid_on_screen(delta)?}) };
        return Some(("layer.decal.set", frame_target(params, &gizmo)));
    }

    if !turn && let Some(round) = RoundGizmo::of(app, rect).filter(|round| round.target == target) {
        return round.slid(app, rect, from, to);
    }

    let (projection, command) = target.projection_editor(app)?;
    let (_, rotation, offset) = placement(projection)?;
    let slide = delta * SLIDE_PER_HEIGHT / rect.height().max(1.0);
    let offset = [offset[0] - slide.x, offset[1] - slide.y];
    let mut params = match (turn, projection) {
        (true, _) => json!({"rotation": rotation + delta.x * TURN_PER_POINT}),
        (false, Projection::Uv { .. }) => json!({"offset": offset}),
        (false, _) => json!({"uvOffset": offset}),
    };
    merge_target(&mut params, target);
    Some((command, params))
}

/// The edit a wheel step growing the projection by `growth` makes: a frame grows, other projections' images repeat less
fn resized(app: &TextureCraftApp, target: MoveOnModel, growth: f32, rect: Rect) -> Option<(&'static str, Value)> {
    if let Some(gizmo) = frame_of(app, target, rect) {
        let [width, height] = gizmo.decal.size;
        return Some(("layer.decal.set", frame_target(json!({"size": [width * growth, height * growth]}), &gizmo)));
    }

    let (projection, command) = target.projection_editor(app)?;
    let (tiling, _, _) = placement(projection)?;
    let mut params = json!({"tiling": [tiling[0] / growth, tiling[1] / growth]});
    merge_target(&mut params, target);
    Some((command, params))
}

/// Adds the `layer` or `effect` param naming `target` to `params`
fn merge_target(params: &mut Value, target: MoveOnModel) {
    if let (Some(params), Value::Object(fields)) = (params.as_object_mut(), target.target()) {
        params.extend(fields);
    }
}

#[cfg(test)]
mod tests {
    use egui::{Modifiers, PointerButton, vec2};
    use egui_kittest::kittest::Queryable;
    use texturecraft_engine::core::{Projection, UvTransform};

    use super::*;
    use crate::frame_gizmo::tests::{drag, fill_app, projection};
    use crate::pointer_lock::system_pointer::SystemPointer;
    use crate::round_gizmo::tests::on_the_model_off_the_handles;
    use crate::state::ViewLayout;

    type Harness = egui_kittest::Harness<'static, TextureCraftApp>;

    fn uv_of(projection: &Projection) -> UvTransform {
        match projection {
            Projection::Triplanar { uv, .. } => *uv,
            other => panic!("tri-planar: {other:?}"),
        }
    }

    fn projector_offset(projection: &Projection) -> [f32; 3] {
        match projection {
            Projection::Triplanar { offset, .. } => *offset,
            other => panic!("tri-planar: {other:?}"),
        }
    }

    /// Presses the left button on Move on model (the preview square) and holds it; returns the system's pointer, resting there
    fn press_the_button(harness: &mut Harness) -> SystemPointer {
        let at = harness.get_by_label("Move on model").rect().center();
        let mut pointer = SystemPointer::at(harness, at);
        pointer.button(harness, PointerButton::Primary, true, Modifiers::NONE);
        pointer
    }

    /// A wheel step up where the pointer is, over a few frames as the wheel scrolls smoothly
    fn wheel(harness: &mut Harness, pointer: &mut SystemPointer) {
        harness.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 120.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Modifiers::NONE,
        });
        for _ in 0..6 {
            pointer.frame(harness);
        }
    }

    /// Lets go of the left button, and lets a frame show where the pointer came back to
    fn let_go(harness: &mut Harness, pointer: &mut SystemPointer) {
        pointer.button(harness, PointerButton::Primary, false, Modifiers::NONE);
        pointer.frame(harness);
    }

    /// Holding the pad and dragging carries the projector with the mouse, the pointer hidden and kept on the pad; letting go ends it there
    #[test]
    fn holding_the_button_and_dragging_carries_the_projector_as_one_undo_step() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("triplanar");
        let rect = harness.state().viewport_rect;
        let before = RoundGizmo::of(harness.state(), rect).unwrap();
        let (steps, camera) = (harness.state().session.history_labels().len(), harness.state().session.camera);

        let mut pointer = press_the_button(&mut harness);
        let from = pointer.at;

        assert_eq!(harness.state().moving_on_model, Some(MoveOnModel::layer(layer)), "a press starts the hold");

        let moved = vec2(-300.0, -20.0);
        pointer.drags(&mut harness, moved, 30, Modifiers::NONE);
        let after = RoundGizmo::of(harness.state(), rect).unwrap();

        assert!((after.center - (before.center + moved)).length() < 2.0, "the projector follows the mouse: {:?} -> {:?}", before.center, after.center);
        assert!(pointer.hidden, "the pointer hides while held");
        assert!(pointer.at.distance(from) < 1.0, "and stays on the pad, so no screen edge stops the drag: {:?}", pointer.at);

        let slid = projection(&harness, layer);
        assert_ne!(projector_offset(&slid), [0.0; 3], "the projector itself moved: {slid:?}");
        assert_eq!(uv_of(&slid), UvTransform::default(), "not only the image inside it");

        let_go(&mut harness, &mut pointer);

        assert_eq!(harness.state().moving_on_model, None, "letting go ends it");
        assert!(!pointer.hidden && pointer.at == from, "the pointer shows again where the press began: {:?}", pointer.at);
        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "one undo step per hold");
        assert_eq!(harness.state().session.camera, camera, "the camera kept still");

        // The 3D view has its drags back: a drag on the model off the projector's handles moves nothing
        let off = on_the_model_off_the_handles(&harness);
        drag(&mut harness, off, off + vec2(60.0, 0.0), Modifiers::NONE);

        assert_eq!(projection(&harness, layer), slid, "no mode is left on");
    }

    /// A mask fill's warp frame moves with the hold as a fill layer's does: the frame slides, not only the image inside it
    #[test]
    fn holding_a_mask_fills_pad_slides_its_frame() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("triplanar");
        harness.state_mut().run("mask.add.black", json!({"layer": layer.0})).unwrap();
        let effect = harness.state_mut().run("effect.add.fill", json!({"layer": layer.0, "target": "mask"})).unwrap()["effect"].as_u64().unwrap();
        harness.state_mut().run("layer.fill.projection", json!({"effect": effect, "projection": "warp"})).unwrap();
        harness.state_mut().run("layer.select", json!({"layer": layer.0, "target": effect})).unwrap();
        harness.run_steps(4);
        let mask_projection = |harness: &Harness| match &harness.state().session.doc.project.effect(LayerId(effect)).unwrap().1.kind {
            EffectKind::Fill { projection, .. } => projection.clone(),
            other => panic!("a mask fill: {other:?}"),
        };
        let Projection::Warp { frame: before, uv: uv_before, .. } = mask_projection(&harness) else { panic!("a warp") };

        let mut pointer = press_the_button(&mut harness);
        assert_eq!(harness.state().moving_on_model, Some(MoveOnModel { layer, effect: Some(LayerId(effect)) }), "a press starts the hold");

        pointer.drags(&mut harness, vec2(-60.0, 0.0), 10, Modifiers::NONE);
        let_go(&mut harness, &mut pointer);

        let Projection::Warp { frame: after, uv: uv_after, .. } = mask_projection(&harness) else { panic!("still a warp") };
        assert_ne!(after.offset, before.offset, "the frame slid");
        assert_eq!(uv_after, uv_before, "not only the image inside it");
    }

    /// Losing the focus mid-hold (Alt+Tab) ends it as letting go does: one undo step, the pointer shown, and moves after it (egui still has the button down) leave the projection alone
    #[test]
    fn losing_the_window_focus_mid_hold_ends_it_as_letting_go_does() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("triplanar");
        let steps = harness.state().session.history_labels().len();
        let mut pointer = press_the_button(&mut harness);
        pointer.drags(&mut harness, vec2(-60.0, 0.0), 10, Modifiers::NONE);

        assert_eq!(harness.state().moving_on_model, Some(MoveOnModel::layer(layer)), "the hold is under way");

        harness.input_mut().events.push(egui::Event::WindowFocused(false));
        pointer.frame(&mut harness);
        let left = projection(&harness, layer);
        assert_eq!(harness.state().moving_on_model, None, "the focus loss ends the hold");
        assert!(!pointer.hidden, "the pointer shows again");
        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "one undo step, as letting go makes");

        pointer.drags(&mut harness, vec2(-60.0, 0.0), 10, Modifiers::NONE);

        assert_eq!(harness.state().moving_on_model, None, "the button still down in egui starts no new hold");
        assert_eq!(projection(&harness, layer), left, "moves after it leave the projection alone");
    }

    /// Removing the held fill mid-hold (its pad goes with it) ends the hold at once, the button still down, with or without the 3D view: the pointer shows
    #[test]
    fn removing_the_held_fill_ends_the_hold() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for layout in [ViewLayout::Split, ViewLayout::Only2D] {
            let (mut harness, layer) = fill_app("triplanar");
            harness.state_mut().ui.layout = layout;
            harness.run_steps(2);
            let mut pointer = press_the_button(&mut harness);
            pointer.drags(&mut harness, vec2(-60.0, 0.0), 10, Modifiers::NONE);

            assert_eq!(harness.state().moving_on_model, Some(MoveOnModel::layer(layer)), "{layout:?}: the hold is under way");

            harness.state_mut().run("layer.remove", json!({"layer": layer.0})).unwrap();
            pointer.frame(&mut harness);

            assert_eq!(harness.state().moving_on_model, None, "{layout:?}: the hold ended with its fill");
            assert!(!pointer.hidden, "{layout:?}: the pointer shows again");
        }
    }

    /// The preview square is the pad: a press anywhere on it, its corner too, starts the hold and holds the pointer
    #[test]
    fn the_preview_square_is_the_pad_from_corner_to_corner() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("triplanar");
        let pad = harness.get_by_label("Move on model").rect();

        assert!((pad.width() - pad.height()).abs() < 1.0 && pad.width() > 60.0, "Move on model is the preview square, no row of its own: {pad:?}");

        let before = projection(&harness, layer);
        let mut pointer = SystemPointer::at(&mut harness, pad.left_top() + vec2(4.0, 4.0));
        pointer.button(&mut harness, PointerButton::Primary, true, Modifiers::NONE);

        assert_eq!(harness.state().moving_on_model, Some(MoveOnModel::layer(layer)), "a press at its corner starts the hold");

        pointer.drags(&mut harness, vec2(40.0, 0.0), 10, Modifiers::NONE);

        assert!(pointer.hidden, "the pointer is held");

        let_go(&mut harness, &mut pointer);

        assert_ne!(projection(&harness, layer), before, "the drag moved the projection");
        assert_eq!(harness.state().moving_on_model, None);
    }

    /// While held, Shift or the right button turns the image and the wheel scales it, all in the hold's one undo step; the camera neither orbits nor zooms. Released, the wheel zooms again
    #[test]
    fn shift_or_the_right_button_turns_and_the_wheel_scales_while_held() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("triplanar");
        let rect = harness.state().viewport_rect;
        let (steps, camera) = (harness.state().session.history_labels().len(), harness.state().session.camera);
        let mut pointer = press_the_button(&mut harness);
        let from = pointer.at;

        pointer.drags(&mut harness, vec2(-40.0, 0.0), 30, Modifiers::SHIFT);
        let shifted = uv_of(&projection(&harness, layer));

        assert!(shifted.rotation > 0.0, "Shift turns the image: {shifted:?}");
        assert_eq!(projector_offset(&projection(&harness, layer)), [0.0; 3], "and does not slide it");

        // Press the second button over the viewport while the first still holds the drag
        let over_view = rect.center() + vec2(-100.0, 80.0);
        pointer.drags(&mut harness, over_view - (from + vec2(-40.0, 0.0)), 30, Modifiers::NONE);
        let slid = projection(&harness, layer);
        pointer.button(&mut harness, PointerButton::Secondary, true, Modifiers::NONE);
        pointer.drags(&mut harness, vec2(-40.0, 0.0), 30, Modifiers::NONE);
        let turned = projection(&harness, layer);
        assert!(uv_of(&turned).rotation < shifted.rotation, "the right button turns it: {shifted:?} -> {turned:?}");
        assert_eq!(projector_offset(&turned), projector_offset(&slid), "without sliding");

        pointer.button(&mut harness, PointerButton::Secondary, false, Modifiers::NONE);

        wheel(&mut harness, &mut pointer);
        let scaled = uv_of(&projection(&harness, layer));
        assert!(scaled.tiling[0] < uv_of(&turned).tiling[0] && scaled.tiling[0] == scaled.tiling[1], "the wheel up makes the image bigger: {scaled:?}");
        assert_eq!(harness.state().session.camera, camera, "the camera neither orbited nor zoomed");
        assert!(harness.state().properties_popup.is_none(), "the right click opened no Properties popup");

        let_go(&mut harness, &mut pointer);

        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "the whole hold is one undo step");

        let back = pointer.at;
        pointer.moves(&mut harness, over_view - back, Modifiers::NONE);
        wheel(&mut harness, &mut pointer);

        assert_ne!(harness.state().session.camera, camera, "let go, the wheel zooms the camera");
        assert_eq!(uv_of(&projection(&harness, layer)), scaled, "and leaves the projection alone");
    }

    /// A hold whose first edit changes nothing (Shift with an up and down drag turns a frame by nothing) makes no undo step, so its later edits make the hold's own step instead of joining the edit before it
    #[test]
    fn a_hold_that_first_changes_nothing_keeps_its_own_undo_step() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("warp");
        harness.state_mut().run("layer.decal.set", json!({"layer": layer.0, "rotation": 30.0})).unwrap();
        harness.run_steps(2);
        let turned = projection(&harness, layer);
        let steps = harness.state().session.history_labels().len();

        let mut pointer = press_the_button(&mut harness);
        pointer.drags(&mut harness, vec2(0.0, -40.0), 10, Modifiers::SHIFT);

        assert_eq!(projection(&harness, layer), turned, "an up and down turn changes nothing");

        pointer.drags(&mut harness, vec2(0.0, -40.0), 10, Modifiers::NONE);
        let_go(&mut harness, &mut pointer);

        assert_ne!(projection(&harness, layer), turned, "the slide moved the frame");
        assert_eq!(harness.state().session.history_labels().len(), steps + 1, "the hold made its own undo step");

        harness.state_mut().session.undo().unwrap();

        assert_eq!(projection(&harness, layer), turned, "undo takes back the hold only");
    }

    /// A UV image slides as far as the mouse goes, past the window's edge; a warp's frame slides across its plane
    #[test]
    fn a_hold_slides_a_uv_image_and_a_frame_across_its_plane() {
        let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (mut harness, layer) = fill_app("uv");
        let height = harness.state().viewport_rect.height();
        let mut pointer = press_the_button(&mut harness);
        let from = pointer.at;
        wheel(&mut harness, &mut pointer);

        assert_eq!(harness.get_by_label("Move on model").rect().center(), from, "a wheel step over the panel while held scales, it does not scroll the panel");
        let Projection::Uv { scale, .. } = projection(&harness, layer) else { panic!("still UV") };
        assert!(scale[0] < 1.0, "the wheel up makes the image bigger: {scale:?}");

        let far = vec2(-3.0 * from.x, 0.0);
        pointer.drags(&mut harness, far, 60, Modifiers::NONE);
        let_go(&mut harness, &mut pointer);
        let Projection::Uv { offset, .. } = projection(&harness, layer) else { panic!("still UV") };
        assert!(
            (offset[0] - (-far.x) * SLIDE_PER_HEIGHT / height).abs() < 0.01 && offset[1] == 0.0,
            "a drag left slides the image left, all the way: {offset:?}"
        );

        let (mut harness, layer) = fill_app("warp");
        let mut pointer = press_the_button(&mut harness);
        pointer.drags(&mut harness, vec2(0.0, -40.0), 30, Modifiers::NONE);
        let_go(&mut harness, &mut pointer);
        let Projection::Warp { frame, .. } = projection(&harness, layer) else { panic!("still a warp") };
        assert!(frame.offset[1] > 0.0 && frame.offset[0].abs() < frame.offset[1] * 0.2, "a drag up slides the frame up: {frame:?}");
    }
}
