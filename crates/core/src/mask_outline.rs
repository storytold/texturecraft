//! The Mask Outline filter's settings and the band it draws

use serde::{Deserialize, Serialize};

use crate::{OutlinePosition, RampCurve, at_or_above_threshold};

/// A band along the edge where values cross a threshold, outside, inside or on both sides of the shapes
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskOutline {
    pub position: OutlinePosition,
    /// The band on black; otherwise it is laid over the values below
    pub from_black: bool,
    /// Values at or above it are inside the shapes (0..1)
    pub threshold: f32,
    /// How far the band reaches from the edge, a fraction of the UV tile (0..1)
    pub width: f32,
    /// How much of the band fades out towards its far side (0..1)
    pub blur: f32,
    /// Bends that fade: below 0 it falls off sooner, above 0 later (−1..1)
    pub curve: f32,
}

impl MaskOutline {
    /// The Mask Outline filter as the library adds it
    pub const LIBRARY: MaskOutline = MaskOutline { position: OutlinePosition::Outside, from_black: true, threshold: 1.0, width: 0.05, blur: 0.0, curve: 0.0 };

    /// Whether `value` is inside the shapes
    pub fn inside(&self, value: f32) -> bool {
        at_or_above_threshold(value, self.threshold, Self::LIBRARY.threshold)
    }

    /// The curve the band's fade takes (Curve), worked out once per filter
    pub fn fade_curve(&self) -> RampCurve {
        RampCurve::new(0.0, self.curve)
    }

    /// The band at a texel inside the shapes or not, `closeness` to the edge going from 1 at it to 0 at the band's reach, its fade bent by [`MaskOutline::fade_curve`]
    pub fn band(&self, fade_curve: RampCurve, inside: bool, closeness: f32) -> f32 {
        let closeness = if closeness.is_finite() { closeness.clamp(0.0, 1.0) } else { 0.0 };
        if !self.position.covers(inside) || closeness <= 0.0 || !self.width.is_finite() || self.width <= 0.0 {
            return 0.0;
        }

        let blur = if self.blur.is_finite() { self.blur.clamp(0.0, 1.0) } else { 0.0 };
        if blur <= 0.0 {
            return 1.0;
        }

        fade_curve.apply(closeness / blur)
    }

    /// The result at a texel of value `value`: the band on black, or over the value
    pub fn apply(&self, value: f32, band: f32) -> f32 {
        if self.from_black { band } else { value.max(band) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The band with the curve worked out as the filter does
    fn band(filter: MaskOutline, inside: bool, closeness: f32) -> f32 {
        filter.band(filter.fade_curve(), inside, closeness)
    }

    #[test]
    fn the_band_lies_on_its_side_of_the_edge() {
        let outline = MaskOutline::LIBRARY;
        assert!(outline.inside(1.0) && !outline.inside(0.99));
        assert_eq!(band(outline, false, 0.3), 1.0, "a hard band outside");
        assert_eq!(band(outline, true, 0.3), 0.0, "nothing inside an outside band");
        assert_eq!(band(outline, false, 0.0), 0.0, "nothing past its reach");

        let inside = MaskOutline { position: OutlinePosition::Inside, ..outline };
        assert_eq!((band(inside, true, 0.3), band(inside, false, 0.3)), (1.0, 0.0));

        let both = MaskOutline { position: OutlinePosition::Both, ..outline };
        assert_eq!((band(both, true, 0.3), band(both, false, 0.3)), (1.0, 1.0));
        assert_eq!(outline.apply(0.4, 0.0), 0.0, "from black");
        assert_eq!(MaskOutline { from_black: false, ..outline }.apply(0.4, 0.0), 0.4, "over the values");
    }

    #[test]
    fn blur_fades_the_band_and_curve_shape_bends_the_fade() {
        let soft = MaskOutline { blur: 1.0, ..MaskOutline::LIBRARY };
        assert!((band(soft, false, 0.5) - 0.5).abs() < 1e-6, "a linear fade");
        assert!(band(MaskOutline { curve: 0.5, ..soft }, false, 0.5) > 0.5, "falls off later");
        assert!(band(MaskOutline { curve: -0.5, ..soft }, false, 0.5) < 0.5, "falls off sooner");
        assert_eq!(band(MaskOutline { blur: 0.5, ..soft }, false, 0.6), 1.0, "only the far part fades");
        assert_eq!(band(MaskOutline { width: f32::NAN, threshold: f32::NAN, ..soft }, false, 0.5), 0.0);
    }
}
