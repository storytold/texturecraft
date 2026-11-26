use serde::{Deserialize, Serialize};

/// The UV Random Color generator's controls: one random value per UV island, picked at random or along a gradient
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UvRandomColorParameters {
    /// Pick each island's color along [`UvRandomColorParameters::gradient`] instead of at random
    pub custom_gradient: bool,
    /// The gradient's colors (linear RGB, [`UvRandomColorParameters::MAX_GRADIENT_COLORS`] at most)
    pub gradient: Vec<[f32; 3]>,
    /// Each color's place on the gradient (0..1); colors past the end are evenly spaced (projects saved before have none)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<f32>,
    /// How many of the colors the gradient uses (all of them where None): the rest are kept for when the count grows again
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<u8>,
}

impl UvRandomColorParameters {
    pub const MAX_GRADIENT_COLORS: usize = 16;

    /// The fewest colors the gradient's count goes down to
    pub const MIN_QUANTITY: u8 = 2;

    /// Every color brought into 0..1 (non-finite components black), at most [`UvRandomColorParameters::MAX_GRADIENT_COLORS`] of them, and at least one; places into 0..1, one per color at most
    pub fn clamped(self) -> UvRandomColorParameters {
        let unit = |value: f32| if value.is_finite() { value.clamp(0.0, 1.0) } else { 0.0 };
        let gradient: Vec<[f32; 3]> = self.gradient.into_iter().take(UvRandomColorParameters::MAX_GRADIENT_COLORS).map(|color| color.map(unit)).collect();
        let gradient = if gradient.is_empty() { UvRandomColorParameters::default().gradient } else { gradient };
        let positions = self.positions.into_iter().take(gradient.len()).map(unit).collect();
        let quantity = self.quantity.map(|count| count.clamp(UvRandomColorParameters::MIN_QUANTITY, UvRandomColorParameters::MAX_GRADIENT_COLORS as u8));
        UvRandomColorParameters { gradient, positions, quantity, ..self }
    }

    /// The colors the gradient uses: the first [`UvRandomColorParameters::quantity`] of them (all of them where it has none)
    pub fn used_colors(&self) -> &[[f32; 3]] {
        let used = self.quantity.map_or(self.gradient.len(), usize::from).min(self.gradient.len());
        self.gradient.get(..used).unwrap_or(&self.gradient)
    }

    /// Each used color's place on the gradient, evenly spaced where it has none
    pub fn used_positions(&self) -> Vec<f32> {
        let used = self.used_colors().len();
        let last = used.saturating_sub(1).max(1) as f32;
        (0..used).map(|index| self.positions.get(index).copied().unwrap_or(index as f32 / last)).collect()
    }

    /// The gradient's color at `t` (linear blend)
    pub fn at(&self, t: f32) -> Option<[f32; 3]> {
        let colors = self.used_colors();

        // An evenly spaced gradient (projects saved before) takes the path it took then, so it renders bit for bit as before
        if self.positions.is_empty() { super::gradient(colors, t) } else { crate::gradient_input::ramp(colors, &self.positions, true, t) }
    }

    /// Use the first `wanted` colors: an evenly spaced gradient keeps its places from now on, and each added color starts at 1 as the gradient's color there
    pub fn set_quantity(&mut self, wanted: u8) {
        let wanted = wanted.clamp(UvRandomColorParameters::MIN_QUANTITY, UvRandomColorParameters::MAX_GRADIENT_COLORS as u8);

        // Preserve the implicit positions of visible stops and the stored positions of hidden stops
        let even = self.used_positions();
        let placed = self.positions.len();
        self.positions.extend(even.into_iter().skip(placed));
        let fill = self.at(1.0).unwrap_or([1.0; 3]);
        let wanted_len = usize::from(wanted);
        if self.gradient.len() < wanted_len {
            self.gradient.resize(wanted_len, fill);
        }
        if self.positions.len() < wanted_len {
            self.positions.resize(wanted_len, 1.0);
        }
        self.quantity = Some(wanted);
    }
}

impl Default for UvRandomColorParameters {
    fn default() -> UvRandomColorParameters {
        UvRandomColorParameters { custom_gradient: false, gradient: vec![[0.0; 3], [1.0; 3]], positions: Vec::new(), quantity: None }
    }
}
