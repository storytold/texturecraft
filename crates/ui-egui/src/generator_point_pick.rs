//! Picking a 3D generator's point on the mesh: its crosshair button arms it, the next click in the 3D view sets the point, Esc or the button again cancels

use texturecraft_engine::core::{EffectKind, GeneratorKind, LayerId};
use texturecraft_engine::{Session, StackTarget};

/// Which point of which generator effect the next click in the 3D view sets
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GeneratorPointPick {
    pub(crate) effect: u64,
    /// `effect.generator.pickPoint`'s point: distance, start or end
    pub(crate) point: &'static str,
}

impl GeneratorPointPick {
    /// Whether a `kind` generator has the point this pick sets
    fn belongs_to(&self, kind: GeneratorKind) -> bool {
        matches!((kind, self.point), (GeneratorKind::ThreeDDistance, "distance") | (GeneratorKind::ThreeDLinearGradient, "start" | "end"))
    }
}

/// Where the armed pick is kept: a transient UI mode, like the depth of field's focus pick
fn pick_id() -> egui::Id {
    egui::Id::unique("tc_generator_point_pick")
}

/// The point the next click in the 3D view sets, if one is armed
pub(crate) fn armed(ctx: &egui::Context) -> Option<GeneratorPointPick> {
    ctx.data(|data| data.get_temp::<Option<GeneratorPointPick>>(pick_id())).flatten()
}

/// Arms `pick`, or disarms it when it is the one armed
pub(crate) fn toggle(ctx: &egui::Context, pick: GeneratorPointPick) {
    let next = if armed(ctx) == Some(pick) { None } else { Some(pick) };
    set(ctx, next);
}

/// The armed pick while its effect is still the selected one and still has its point; a pick whose effect was deselected, deleted or swapped to a kind without that point is dropped
pub(crate) fn armed_on(ctx: &egui::Context, session: &Session) -> Option<GeneratorPointPick> {
    let pick = armed(ctx)?;
    let has_point = match session.doc.project.effect(LayerId(pick.effect)).map(|(_, effect)| &effect.kind) {
        Some(EffectKind::Generator(generator)) => pick.belongs_to(generator.kind),
        _ => false,
    };
    if has_point && session.target == StackTarget::Effect(LayerId(pick.effect)) {
        return Some(pick);
    }

    set(ctx, None);
    None
}

pub(crate) fn set(ctx: &egui::Context, pick: Option<GeneratorPointPick>) {
    ctx.data_mut(|data| data.insert_temp(pick_id(), pick));
}
