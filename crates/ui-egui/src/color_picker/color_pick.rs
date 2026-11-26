use egui::{Context, Id};

use super::SwatchEdit;

/// What a colour bar did this frame: the colour picked (linear) and whether it continues the last edit, a click on its eyedropper, or a change to the swatches
#[derive(Clone, Copy, Debug)]
pub(crate) struct ColorPick {
    /// The bar's id: an eyedropper pick for it is handed over with [`ColorPick::deliver`]
    pub(crate) id: Id,
    pub(crate) picked: Option<[f32; 3]>,
    /// The pick continues a drag: it joins the last undo step
    pub(crate) merge: bool,
    pub(crate) eyedropper: bool,
    pub(crate) swatch: Option<SwatchEdit>,
}

impl ColorPick {
    pub(crate) fn none(id: Id) -> Self {
        ColorPick { id, picked: None, merge: false, eyedropper: false, swatch: None }
    }

    /// Hands the colour an eyedropper picked to the bar `target`, which reports it next frame
    pub(crate) fn deliver(ctx: &Context, target: Id, linear: [f32; 3]) {
        ctx.data_mut(|data| data.insert_temp(Self::delivery_id(target), linear));
    }

    /// The colour an eyedropper picked for the bar `target`, once
    pub(crate) fn delivered(ctx: &Context, target: Id) -> Option<[f32; 3]> {
        ctx.data_mut(|data| data.remove_temp::<[f32; 3]>(Self::delivery_id(target)))
    }

    fn delivery_id(target: Id) -> Id {
        target.with("eyedropped")
    }
}
