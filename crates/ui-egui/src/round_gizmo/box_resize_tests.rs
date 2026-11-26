use egui::Modifiers;
use texturecraft_engine::core::Projection;

use super::RoundGizmo;
use crate::frame_gizmo::tests::{drag, fill_app, projection};

#[test]
fn holding_shift_on_a_box_face_resizes_one_side_without_moving_the_opposite_face() {
    let _language = crate::i18n::TEST_LANGUAGE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut harness, layer) = fill_app("triplanar");
    let start = RoundGizmo::of(harness.state(), harness.state().viewport_rect).unwrap();
    let face = start.sizes.iter().find(|size| size.face.is_some()).expect("a box face handle");
    let axis = face.axes.iter().position(|on| *on).unwrap();
    let opposite_world = start.world(start.local - face.face.unwrap());
    let pulled = start.center + (face.at - start.center) * 1.5;

    drag(&mut harness, face.at, pulled, Modifiers::SHIFT);

    let after = RoundGizmo::of(harness.state(), harness.state().viewport_rect).unwrap();
    let Projection::Triplanar { projector, offset, .. } = projection(&harness, layer) else { panic!("a triplanar projection") };
    let after_opposite = after.world(after.local - face.face.unwrap());

    assert!(projector.scale[axis] > 1.15 && projector.scale[axis] < 1.35, "one-sided pull scales half as much as symmetric: {:?}", projector.scale);
    assert!(offset[axis].abs() > 0.05, "box centre shifts toward the pulled face: {offset:?}");
    assert!(after_opposite.distance(opposite_world) < 1e-2, "opposite face stayed anchored: {opposite_world:?} vs {after_opposite:?}");
}
