use egui::ecolor::hsv_from_rgb;

use super::{ColorModel, ColorSpace};

/// One colour bar's picker between frames: open or not, the colour it opened on, and the hue and saturation a gray would lose
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PickerState {
    pub(crate) open: bool,
    /// The colour when the picker opened (linear), shown beside the new one and restored by a click
    pub(crate) before: [f32; 3],
    /// The colour being edited (linear)
    pub(crate) linear: [f32; 3],
    /// Hue, saturation and value of the shown values, kept across grays and blacks
    pub(crate) hsv: [f32; 3],
    pub(crate) space: ColorSpace,
    pub(crate) model: ColorModel,
    /// Height of the picker outside its resizable colour field, in points
    pub(crate) fixed_height: Option<f32>,
    /// Where its title bar's drag left the window's top left corner; None while it stands beside the panel
    pub(crate) moved_to: Option<egui::Pos2>,
}

impl PickerState {
    /// Follows `linear` when it changed outside the picker (undo, another panel), keeping the hue when it didn't
    pub(crate) fn follow(&mut self, linear: [f32; 3]) {
        if self.linear != linear {
            self.set_linear(linear);
        }
    }

    /// Sets the colour from outside the hue field (typed, restored, picked), deriving hue, saturation and value again
    pub(crate) fn set_linear(&mut self, linear: [f32; 3]) {
        self.linear = linear;
        let (hue, saturation, value) = hsv_from_rgb(self.shown());
        self.hsv = [hue, saturation, value];
    }

    /// Sets the colour from hue, saturation and value (field, strip, HSV sliders)
    pub(crate) fn set_hsv(&mut self, hsv: [f32; 3]) {
        self.hsv = hsv.map(|part| part.clamp(0.0, 1.0));
        let [hue, saturation, value] = self.hsv;
        self.linear = self.space.linear(egui::ecolor::rgb_from_hsv((hue, saturation, value)));
    }

    /// Shows the values in `space` from now on
    pub(crate) fn set_space(&mut self, space: ColorSpace) {
        let [hue, saturation, _] = self.hsv;
        self.space = space;
        self.set_linear(self.linear);

        // A gray keeps the hue it had and a black its saturation too, so raising Value brings the colour back
        let [_, new_saturation, new_value] = self.hsv;
        if new_saturation == 0.0 {
            self.hsv[0] = hue;
        }
        if new_value == 0.0 {
            self.hsv[1] = saturation;
        }
    }

    /// The colour's values in the shown space
    pub(crate) fn shown(&self) -> [f32; 3] {
        self.space.shown(self.linear)
    }

    /// Opens on `linear`, which becomes the colour to go back to
    pub(crate) fn open_on(&mut self, linear: [f32; 3]) {
        self.open = true;
        self.before = linear;
        self.set_linear(linear);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_black_keeps_its_hue_and_saturation_through_a_colour_space_change() {
        let mut state = PickerState::default();
        let blue = 2.0 / 3.0;
        state.set_hsv([blue, 1.0, 0.0]);
        state.set_space(ColorSpace::Linear);

        assert_eq!(state.hsv, [blue, 1.0, 0.0], "black keeps the field's hue and saturation");

        state.set_hsv([state.hsv[0], state.hsv[1], 1.0]);
        let [red, green, blue_part] = state.linear;
        assert!(blue_part > 0.9 && red < 0.01 && green < 0.01, "raising Value brings blue back: {:?}", state.linear);

        // A gray keeps its hue but takes its own saturation; a colour is read again
        state.set_hsv([0.25, 0.0, 0.5]);
        state.set_space(ColorSpace::Srgb);

        assert_eq!((state.hsv[0], state.hsv[1]), (0.25, 0.0));

        state.set_hsv([0.0, 1.0, 1.0]);
        state.set_space(ColorSpace::Linear);

        assert!(state.hsv[0].abs() < 1e-4 && (state.hsv[1] - 1.0).abs() < 1e-4, "red reads as red: {:?}", state.hsv);
    }
}
