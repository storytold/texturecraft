use crate::gradient_input::{interpolate, linear_to_srgb, ramp_stops};
use crate::{ChannelKind, Filter, GradientInput};

/// An sRGB gradient prepared for repeated sampling
pub struct PreparedGradient {
    stops: Vec<(f32, [f32; 3])>,
    input: GradientInput,
}

impl PreparedGradient {
    /// Prepare an sRGB ramp by converting its endpoints once
    pub fn of(filter: &Filter) -> Option<Self> {
        let Filter::GradientMap { positions, linear: false, input, .. } = filter else { return None };

        let mut stops: Vec<_> = ramp_stops(filter.ramp_colors(), positions).map(|(position, color)| (position, color.map(linear_to_srgb))).collect();

        // Stable order preserves which stop wins at shared positions
        stops.sort_by(|left, right| left.0.total_cmp(&right.0));
        Some(Self { stops, input: *input })
    }

    /// Sample the prepared ramp at a normalized position
    pub fn at(&self, position: f32) -> Option<[f32; 3]> {
        let position = if position.is_finite() { position.clamp(0.0, 1.0) } else { 0.0 };
        let split = self.stops.partition_point(|stop| stop.0 <= position);
        let before = split.checked_sub(1).and_then(|index| self.stops.get(index)).copied();
        let after = self.stops.get(split).copied();
        Some(interpolate(before.or(after)?, after.or(before)?, false, position))
    }

    /// Apply the prepared gradient to one value
    pub fn point(&self, kind: ChannelKind, value: [f32; 3]) -> Option<[f32; 3]> {
        self.input.map(kind, value, |position| self.at(position))
    }
}
