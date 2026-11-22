//! Which compositing rules a project follows: TextureCraft's own ([`CompositeRules::default`]), or the rules measured
//! from the reference app's renders, which a project imported from that app switches on so it looks as it did there.
//!
//! The setting is saved in the project only when it differs from the default, so projects made in TextureCraft are
//! written, composited and exported exactly as before. Two independent parts:
//!
//! - [`BlendRules`]: the blend formulas and how a layer is laid over the stack (see [`BlendRules::Measured`]).
//! - [`HeightNormal`]: how the exported normal map is derived from the height channel and combined with the baked
//!   normal and the normal channel.

use serde::{Deserialize, Serialize};

use crate::BlendMode;

/// The rules of one project. The default is TextureCraft's own on both counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct CompositeRules {
    #[serde(default)]
    pub blend: BlendRules,
    #[serde(default)]
    pub height_normal: HeightNormal,
}

impl CompositeRules {
    /// The rules measured from the reference app, with the height filter of the document it came from.
    pub fn measured(height_normal: HeightNormal) -> CompositeRules {
        CompositeRules { blend: BlendRules::Measured, height_normal }
    }

    /// True for TextureCraft's own rules (the project file leaves the setting out).
    pub fn is_native(&self) -> bool {
        *self == CompositeRules::default()
    }
}

/// How layers blend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum BlendRules {
    /// TextureCraft's own formulas ([`crate::blend1`], [`crate::blend3`]); a folder blends its children straight into
    /// the stack below and is mixed back by its opacity and mask.
    #[default]
    Native,
    /// The rules measured from the reference app's renders:
    ///
    /// - The separable formulas of [`measured1`] (divide, inverse divide, soft light, linear light and the dodge and
    ///   burn edges differ from TextureCraft's own).
    /// - Clamp before opacity: on a channel that stores 0..1 (and in masks) the blended value is clamped to 0..1
    ///   before the layer's strength mixes it in; float channels (height) do not clamp ([`BlendRules::blend1`]).
    /// - Normal-map modes weaken the layer's normal towards flat by its strength before blending, instead of mixing
    ///   the blended normal back ([`weaken_normal`]).
    /// - Mode only where something is below: a layer laid over partial coverage `a` gives
    ///   `t w (1 - a) + b a (1 - w) + f(b, t) w a` ([`mix_over`]), so over nothing it shows its own value whatever its
    ///   mode. A folder that does not pass through composites its children on their own, from nothing, and lays the
    ///   result over the stack below with its own mode.
    Measured,
}

impl BlendRules {
    /// One component of a separable blend under these rules (non-separable modes act as `Normal`).
    pub fn blend1(self, mode: BlendMode, a: f32, b: f32) -> f32 {
        match self {
            BlendRules::Native => crate::blend1(mode, a, b),
            BlendRules::Measured => measured1(mode, a, b),
        }
    }

    /// A three-component blend under these rules, non-separable modes included.
    pub fn blend3(self, mode: BlendMode, a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        match self {
            BlendRules::Measured if !mode.is_non_separable() => [measured1(mode, a[0], b[0]), measured1(mode, a[1], b[1]), measured1(mode, a[2], b[2])],
            _ => crate::blend3(mode, a, b),
        }
    }

    /// [`BlendRules::blend1`] as the stack mixes it in: under the measured rules clamped to 0..1 on a `unit` channel
    /// (one that stores 0..1), except for the modes that take the layer's value as it is.
    pub fn blended1(self, mode: BlendMode, a: f32, b: f32, unit: bool) -> f32 {
        let v = self.blend1(mode, a, b);
        if self.clamps(mode, unit) { clamp_unit(v) } else { v }
    }

    /// [`BlendRules::blend3`] as the stack mixes it in (see [`BlendRules::blended1`]).
    pub fn blended3(self, mode: BlendMode, a: [f32; 3], b: [f32; 3], unit: bool) -> [f32; 3] {
        let v = self.blend3(mode, a, b);
        if self.clamps(mode, unit) { v.map(clamp_unit) } else { v }
    }

    fn clamps(self, mode: BlendMode, unit: bool) -> bool {
        self == BlendRules::Measured && unit && !matches!(mode, BlendMode::Normal | BlendMode::Passthrough | BlendMode::Replace | BlendMode::Disable)
    }

    pub fn is_measured(self) -> bool {
        self == BlendRules::Measured
    }
}

fn clamp_unit(v: f32) -> f32 {
    if v.is_nan() { v } else { v.clamp(0.0, 1.0) }
}

/// How the exported normal map takes in the height channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum HeightNormal {
    /// TextureCraft's own: a 3×3 Sobel slope of [`crate::TextureSet::height_strength`] × block size / 16 per unit of
    /// height, laid onto the normal channel, the result onto the baked normal.
    #[default]
    Native,
    /// Measured, documents of the reference app's format up to 61: 3×3 Sobel (`[1 2 1] ⊗ [-1 0 1] / 8`) at
    /// [`MEASURED_SLOPE`]; the height normal laid onto the baked normal first, the normal channel onto that.
    Sobel,
    /// Measured, documents of format 62 on: one-texel backward differences (`h(x) - h(x - 1)` across,
    /// `h(y) - h(y - 1)` down the image) at [`MEASURED_SLOPE`], combined as [`HeightNormal::Sobel`].
    Backward,
}

/// Normal tilt per unit of height per UV unit at height strength 1 under the measured conventions: `width / 100` per
/// unit of height between neighbouring texels (measured at 1K, 2K and 4K; independent of the mesh size).
pub const MEASURED_SLOPE: f32 = 0.01;

impl HeightNormal {
    /// The measured filter of a document of the reference app's format `version` (Sobel up to 61, backward
    /// differences from 64 on; 62 and 63 were never seen and count as the newer).
    pub fn for_document_format(version: u32) -> HeightNormal {
        if version <= 61 { HeightNormal::Sobel } else { HeightNormal::Backward }
    }

    pub fn is_measured(self) -> bool {
        self != HeightNormal::Native
    }

    /// Height change per texel along image x and image y (y down) under a measured filter; `h(dx, dy)` reads the height
    /// at an offset (the caller clamps it to the image). `None` for [`HeightNormal::Native`].
    pub fn gradient(self, h: impl Fn(i64, i64) -> f32) -> Option<(f32, f32)> {
        match self {
            HeightNormal::Native => None,
            HeightNormal::Sobel => {
                let gx = h(1, -1) - h(-1, -1) + 2.0 * (h(1, 0) - h(-1, 0)) + h(1, 1) - h(-1, 1);
                let gy = h(-1, 1) - h(-1, -1) + 2.0 * (h(0, 1) - h(0, -1)) + h(1, 1) - h(1, -1);
                Some((gx / 8.0, gy / 8.0))
            }
            HeightNormal::Backward => Some((h(0, 0) - h(-1, 0), h(0, 0) - h(0, -1))),
        }
    }
}

fn dodge(b: f32, t: f32) -> f32 {
    if t >= 1.0 { if b > 0.0 { 1.0 } else { 0.0 } } else { (b / (1.0 - t)).min(1.0) }
}

fn burn(b: f32, t: f32) -> f32 {
    if t == 0.0 { if b >= 1.0 { 1.0 } else { 0.0 } } else { (1.0 - (1.0 - b) / t).max(0.0) }
}

fn div(a: f32, b: f32) -> f32 {
    if b.abs() <= 1e-6 { if a > 0.0 { 1.0 } else { 0.0 } } else { a / b }
}

/// One component of a separable blend as measured from the reference app's renders, `a` below, `b` the layer. Where
/// it differs from [`crate::blend1`]:
///
/// - divide `min(1, a / b)` (`b = 0`: 1 where `a > 0`, else 0); inverse divide `min(1, b / a)`;
/// - soft light `b <= 0.5 ? 2ab + a²(1 - 2b) : 2a(1 - b) + √a (2b - 1)`;
/// - linear light `a + 2b - 1`, clamped to 0 where `b < 0.5` and to 1 elsewhere;
/// - colour dodge `min(1, a / (1 - b))` (`b >= 1`: 1 where `a > 0`, else 0) and colour burn `max(0, 1 - (1 - a) / b)`
///   (`b = 0`: 1 where `a >= 1`, else 0), with vivid light built from them.
///
/// Non-separable modes act as `Normal`.
pub fn measured1(mode: BlendMode, a: f32, b: f32) -> f32 {
    match mode {
        BlendMode::Divide => div(a, b).min(1.0),
        BlendMode::InverseDivide => div(b, a).min(1.0),
        BlendMode::SoftLight => {
            if b <= 0.5 {
                2.0 * a * b + a * a * (1.0 - 2.0 * b)
            } else {
                2.0 * a * (1.0 - b) + a.max(0.0).sqrt() * (2.0 * b - 1.0)
            }
        }
        BlendMode::ColorDodge => dodge(a, b),
        BlendMode::ColorBurn => burn(a, b),
        BlendMode::VividLight => {
            if b < 0.5 {
                burn(a, 2.0 * b)
            } else {
                dodge(a, 2.0 * b - 1.0)
            }
        }
        BlendMode::LinearLight => {
            let v = a + 2.0 * b - 1.0;
            if b < 0.5 { v.max(0.0) } else { v.min(1.0) }
        }
        _ => crate::blend1(mode, a, b),
    }
}

/// A normal (packed 0..1, decoded as a unit vector) weakened towards flat by `w`: `normalize(lerp(flat, n, w))`, packed. Under the measured
/// rules a normal-map mode blends this at full strength instead of mixing the blended normal back by `w`.
pub fn weaken_normal(n: [f32; 3], w: f32) -> [f32; 3] {
    let u = unit_normal(n);
    let flat = [0.0, 0.0, 1.0];
    let l = [0, 1, 2].map(|i| flat[i] + (u[i] - flat[i]) * w);
    let len = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
    if !(len.is_finite() && len > 1e-12) {
        return [0.5, 0.5, 1.0];
    }
    l.map(|v| v / len * 0.5 + 0.5)
}

/// A packed normal decoded to a unit vector (flat when it has no length).
fn unit_normal(n: [f32; 3]) -> [f32; 3] {
    let u = n.map(|v| v * 2.0 - 1.0);
    let len = (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt();
    if len.is_finite() && len > 1e-8 { u.map(|v| v / len) } else { [0.0, 0.0, 1.0] }
}

/// A layer laid over the stack under the measured rules. `below` is the stack's value composited over `empty` by its
/// coverage `a` (the value shown), `top` the layer's straight value at coverage `cov`, `strength` its opacity × mask.
/// Returns the new shown value and coverage. With `w = strength × cov`:
///
/// `t w (1 - a) + b a (1 - w) + f(b, t) w a + empty (1 - a')`, `a' = a + w (1 - a)`, with `b` the straight value below
/// and `f` clamped as in [`BlendRules::blended3`]. At `a = 1` this is `lerp(below, f, w)`, and for `Normal` it is
/// `lerp(below, t, w)` at any coverage. `Replace` lerps the value and the coverage to the layer's by `strength`,
/// `Disable` keeps the stack, and normal-map modes blend the weakened normal ([`weaken_normal`]) over what is shown.
#[allow(clippy::too_many_arguments)]
pub fn mix_over(
    mode: BlendMode,
    below: [f32; 3],
    a: f32,
    top: [f32; 3],
    cov: f32,
    strength: f32,
    empty: [f32; 3],
    unit: bool,
    comps: usize,
) -> ([f32; 3], f32) {
    let unit_or = |v: f32, or: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { or };
    let (a, cov, strength) = (unit_or(a, 1.0), unit_or(cov, 0.0), unit_or(strength, 0.0));
    if mode == BlendMode::Disable {
        return (below, a);
    }
    if mode == BlendMode::Replace {
        return ([0, 1, 2].map(|i| below[i] + (top[i] - below[i]) * strength), a + (cov - a) * strength);
    }
    let w = strength * cov;
    if w <= 0.0 {
        return (below, a);
    }
    let rules = BlendRules::Measured;
    let out_a = a + w * (1.0 - a);
    if matches!(mode, BlendMode::NormalMapCombine | BlendMode::NormalMapDetail | BlendMode::NormalMapInverseDetail) {
        // both normals decoded as unit vectors first (measured)
        let below = unit_normal(below).map(|v| v * 0.5 + 0.5);
        return (rules.blend3(mode, below, weaken_normal(top, w)), out_a);
    }
    let f = |b: [f32; 3]| if comps == 3 { rules.blended3(mode, b, top, unit) } else { [rules.blended1(mode, b[0], top[0], unit); 3] };
    if a >= 1.0 {
        let blended = f(below);
        return ([0, 1, 2].map(|i| below[i] + (blended[i] - below[i]) * w), 1.0);
    }
    let b = straight(below, a, empty);
    let blended = f(b);
    let c = [0, 1, 2].map(|i| top[i] * w * (1.0 - a) + b[i] * a * (1.0 - w) + blended[i] * w * a + empty[i] * (1.0 - out_a));
    (c, out_a)
}

/// The straight value under coverage `a` of a value composited over `empty`.
fn straight(shown: [f32; 3], a: f32, empty: [f32; 3]) -> [f32; 3] {
    if a <= 0.0 {
        return empty;
    }
    [0, 1, 2].map(|i| (shown[i] - empty[i] * (1.0 - a)) / a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn native_rules_are_the_default_and_match_the_plain_formulas() {
        assert!(CompositeRules::default().is_native());
        assert!(!CompositeRules::measured(HeightNormal::Backward).is_native());
        for m in BlendMode::every() {
            for (a, b) in [(0.2, 0.7), (0.9, 0.1), (0.0, 0.0), (1.0, 1.0), (-0.3, 0.6)] {
                assert_eq!(BlendRules::Native.blend1(m, a, b).to_bits(), crate::blend1(m, a, b).to_bits(), "{m:?}");
                assert_eq!(BlendRules::Native.blended1(m, a, b, true).to_bits(), crate::blend1(m, a, b).to_bits(), "native never clamps: {m:?}");
                let (x, y) = ([a, 0.4, b], [b, 0.3, a]);
                assert_eq!(BlendRules::Native.blend3(m, x, y), crate::blend3(m, x, y), "{m:?}");
            }
        }
    }

    #[test]
    fn measured_formulas_differ_where_measured() {
        let m = BlendRules::Measured;
        assert!(close(m.blend1(BlendMode::Divide, 0.8, 0.4), 1.0), "divide clamps at 1");
        assert!(close(m.blend1(BlendMode::Divide, 0.3, 0.0), 1.0) && close(m.blend1(BlendMode::Divide, 0.0, 0.0), 0.0));
        assert!(close(m.blend1(BlendMode::InverseDivide, 0.4, 0.8), 1.0));
        assert!(close(m.blend1(BlendMode::SoftLight, 0.25, 0.3), 2.0 * 0.25 * 0.3 + 0.0625 * 0.4));
        assert!(close(m.blend1(BlendMode::SoftLight, 0.25, 0.7), 2.0 * 0.25 * 0.3 + 0.5 * 0.4));
        assert!(close(m.blend1(BlendMode::LinearLight, 0.1, 0.2), 0.0) && close(m.blend1(BlendMode::LinearLight, 0.9, 0.8), 1.0));
        assert!(close(m.blend1(BlendMode::ColorDodge, -0.5, 0.5), -1.0), "no floor below 0");
        assert!(close(m.blend1(BlendMode::Multiply, 0.5, 0.4), 0.2), "the others are TextureCraft's own");
        assert!(close(m.blended1(BlendMode::LinearDodge, 0.8, 0.7, true), 1.0) && close(m.blended1(BlendMode::LinearDodge, 0.8, 0.7, false), 1.5));
        assert!(close(m.blended1(BlendMode::Normal, 0.8, 1.7, true), 1.7), "normal takes the layer's value as it is");
    }

    #[test]
    fn mix_over_shows_the_layer_over_nothing_and_blends_over_full_coverage() {
        let empty = [0.5; 3];
        // nothing below: a multiply shows its own value at its strength
        let (v, a) = mix_over(BlendMode::Multiply, empty, 0.0, [0.2; 3], 1.0, 1.0, empty, true, 1);
        assert!(close(v[0], 0.2) && close(a, 1.0));
        let (v, a) = mix_over(BlendMode::Multiply, empty, 0.0, [0.2; 3], 1.0, 0.5, empty, true, 1);
        assert!(close(v[0], 0.2 * 0.5 + 0.5 * 0.5) && close(a, 0.5));
        // full coverage: lerp to the clamped blend
        let (v, a) = mix_over(BlendMode::LinearDodge, [0.8; 3], 1.0, [0.7; 3], 1.0, 0.5, empty, true, 1);
        assert!(close(v[0], 0.9) && close(a, 1.0));
        // straight 0.6 at half coverage over empty 0: t w (1 - a) + b a (1 - w) + f w a
        let below = [0.3; 3];
        let (v, a) = mix_over(BlendMode::Multiply, below, 0.5, [0.5; 3], 1.0, 1.0, [0.0; 3], true, 1);
        assert!(close(v[0], 0.5 * 0.5 + 0.6 * 0.5 * 0.5) && close(a, 1.0), "{v:?}");
        assert_eq!(mix_over(BlendMode::Disable, below, 0.5, [0.9; 3], 1.0, 1.0, empty, true, 1), (below, 0.5));
    }

    #[test]
    fn weakened_normals_stay_unit_and_flat_at_zero() {
        assert_eq!(weaken_normal([0.9, 0.2, 0.7], 0.0), [0.5, 0.5, 1.0]);
        let n = weaken_normal([0.9, 0.2, 0.7], 0.5);
        let u = n.map(|v| v * 2.0 - 1.0);
        assert!(close(u[0] * u[0] + u[1] * u[1] + u[2] * u[2], 1.0));
        assert_eq!(weaken_normal([0.5, 0.5, 0.0], 0.5), [0.5, 0.5, 1.0], "half way to the opposite normal: a zero vector falls back to flat");
    }

    #[test]
    fn height_filters_follow_the_document_format() {
        assert_eq!(HeightNormal::for_document_format(59), HeightNormal::Sobel);
        assert_eq!(HeightNormal::for_document_format(61), HeightNormal::Sobel);
        assert_eq!(HeightNormal::for_document_format(64), HeightNormal::Backward);
        assert_eq!(HeightNormal::Native.gradient(|_, _| 1.0), None);
        // a ramp of 0.01 per texel in x: both filters see 0.01
        for f in [HeightNormal::Sobel, HeightNormal::Backward] {
            let (gx, gy) = f.gradient(|dx, _| dx as f32 * 0.01).unwrap_or_default();
            assert!(close(gx, 0.01) && close(gy, 0.0), "{f:?}");
        }
        // a single raised texel: backward differences read left and up only
        let bump = |dx: i64, dy: i64| if (dx, dy) == (0, 0) { 1.0 } else { 0.0 };
        assert_eq!(HeightNormal::Backward.gradient(bump), Some((1.0, 1.0)));
        assert_eq!(HeightNormal::Sobel.gradient(bump), Some((0.0, 0.0)));
    }

    #[test]
    fn rules_round_trip_through_json() {
        let r = CompositeRules::measured(HeightNormal::Sobel);
        let json = serde_json::to_value(r).unwrap_or_default();
        assert_eq!(json, serde_json::json!({"blend": "Measured", "height_normal": "Sobel"}));
        assert_eq!(serde_json::from_value::<CompositeRules>(json).ok(), Some(r));
        assert_eq!(serde_json::from_value::<CompositeRules>(serde_json::json!({})).ok(), Some(CompositeRules::default()));
    }
}
