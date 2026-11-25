/// The eyedropper disc's colour over the app: what a click there leaves the bar holding (the model under it, else the bar's own)
pub struct SwatchColor;

/// Where this frame's colour on the model is kept, with the pass it was read in
const MODEL_KEY: &str = "eyedropper_swatch_model";
/// Where the waiting bar's colour is kept
const BAR_KEY: &str = "eyedropper_swatch_bar";

impl SwatchColor {
    /// The model's base colour (linear) under the pointer this frame
    pub(crate) fn over_model(ctx: &egui::Context, linear: [f32; 3]) {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(MODEL_KEY), (pass, linear)));
    }

    /// The colour (linear) of the bar whose eyedropper waits
    pub(crate) fn of_bar(ctx: &egui::Context, linear: [f32; 3]) {
        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(BAR_KEY), linear));
    }

    /// The disc's colour this frame as 8-bit sRGB, none before the waiting bar has been drawn
    pub(crate) fn shown(ctx: &egui::Context) -> Option<[u8; 3]> {
        let pass = ctx.cumulative_pass_nr();
        let model = ctx.data(|data| data.get_temp::<(u64, [f32; 3])>(egui::Id::unique(MODEL_KEY)));
        let bar = ctx.data(|data| data.get_temp::<[f32; 3]>(egui::Id::unique(BAR_KEY)));
        chosen(model, pass, bar)
    }
}

/// The model's colour if it was read in pass `pass`, else the bar's, as 8-bit sRGB
fn chosen(model: Option<(u64, [f32; 3])>, pass: u64, bar: Option<[f32; 3]>) -> Option<[u8; 3]> {
    let linear = model.filter(|(read, _)| *read == pass).map(|(_, linear)| linear).or(bar)?;
    Some(linear.map(egui::ecolor::gamma_u8_from_linear_f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_disc_shows_the_model_under_the_pointer_and_else_the_bars_own_colour() {
        let (red, blue) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);

        assert_eq!(chosen(None, 3, None), None, "nothing before the bar is drawn");
        assert_eq!(chosen(Some((3, blue)), 3, Some(red)), Some([0, 0, 255]), "the model under the pointer");
        assert_eq!(chosen(Some((2, blue)), 3, Some(red)), Some([255, 0, 0]), "off the model: the bar's own colour, a click there keeps it");
        assert_eq!(chosen(None, 3, Some([0.5; 3])), Some([188; 3]), "shown as the screen shows it (sRGB)");
    }
}
