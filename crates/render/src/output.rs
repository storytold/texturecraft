//! Output resolution of scene image exports and renders: an explicit width × height, a named
//! preset or the viewport's size, times a render multiplier (1×, 2×, 4×), checked against a
//! ceiling (the GPU's largest texture, or the export limit).
//!
//! Pure arithmetic with no GPU or UI dependency, shared by the headless `render` (CLI, control
//! protocol, MCP) and the app's image export so both resolve a request the same way.

use std::fmt;

/// Render multipliers offered (the viewport or the chosen size times 1, 2 or 4 per side).
pub const SCALES: [u32; 3] = [1, 2, 4];
/// Smallest edge of an output image.
pub const MIN_SIDE: u32 = 16;

/// A named output size (landscape or square; no trademarks, just common sizes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    pub id: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Export presets, by id.
pub const PRESETS: [Preset; 5] = [
    Preset { id: "hd", width: 1920, height: 1080 },
    Preset { id: "qhd", width: 2560, height: 1440 },
    Preset { id: "4k", width: 3840, height: 2160 },
    Preset { id: "square-1k", width: 1024, height: 1024 },
    Preset { id: "square-2k", width: 2048, height: 2048 },
];

/// The preset called `id` (case-insensitive).
pub fn preset(id: &str) -> Option<Preset> {
    PRESETS.iter().copied().find(|p| p.id.eq_ignore_ascii_case(id))
}

/// What the caller asked for; every field is optional ("auto").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SizeRequest {
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// A preset id; an explicit `width` or `height` overrides the matching edge.
    pub preset: Option<String>,
    /// Render multiplier, 1 (default), 2 or 4.
    pub scale: Option<u32>,
}

/// Why a request cannot be met; the text says what to change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SizeError {
    UnknownPreset(String),
    BadScale(u32),
    TooSmall { side: u32 },
    TooLarge { width: u32, height: u32, max: u32, limit: &'static str },
}

impl fmt::Display for SizeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SizeError::UnknownPreset(id) => {
                let ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
                write!(f, "unknown size preset `{id}` (one of {})", ids.join(", "))
            }
            SizeError::BadScale(s) => write!(f, "render scale {s} must be 1, 2 or 4"),
            SizeError::TooSmall { side } => write!(f, "render size {side} must be at least {MIN_SIDE} pixels"),
            SizeError::TooLarge { width, height, max, limit } => {
                write!(f, "render size {width}x{height} is over the {limit} of {max} pixels a side; lower the size or the render scale")
            }
        }
    }
}

impl std::error::Error for SizeError {}

/// The output size for `req`. `viewport` is the session viewport (the default size and the aspect
/// ratio kept when only one edge is given); `max_side` the ceiling per edge and `limit` its
/// name for the error ("GPU texture limit", "export limit").
pub fn resolve(viewport: [f32; 2], req: &SizeRequest, max_side: u32, limit: &'static str) -> Result<[u32; 2], SizeError> {
    let scale = req.scale.unwrap_or(1);
    if !SCALES.contains(&scale) {
        return Err(SizeError::BadScale(scale));
    }
    let base = match &req.preset {
        Some(id) => Some(preset(id).ok_or_else(|| SizeError::UnknownPreset(id.clone()))?),
        None => None,
    };
    let [vw, vh] = viewport.map(|v| if v.is_finite() { v.max(1.0) } else { 1.0 });
    // The reference size: the preset, else the viewport.
    let (rw, rh) = base.map_or((vw, vh), |p| (p.width as f32, p.height as f32));
    let (w, h) = match (req.width, req.height) {
        (Some(w), Some(h)) => (w as f32, h as f32),
        // One edge: the other follows the reference's aspect ratio.
        (Some(w), None) => (w as f32, w as f32 * rh / rw),
        (None, Some(h)) => (h as f32 * rw / rh, h as f32),
        (None, None) => (rw, rh),
    };
    let edge = |v: f32| (v.round().max(1.0) as u64).saturating_mul(u64::from(scale));
    let (w, h) = (edge(w), edge(h));
    for side in [w, h] {
        if side < u64::from(MIN_SIDE) {
            return Err(SizeError::TooSmall { side: side as u32 });
        }
    }
    let clamp = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
    if w > u64::from(max_side) || h > u64::from(max_side) {
        return Err(SizeError::TooLarge { width: clamp(w), height: clamp(h), max: max_side, limit });
    }
    Ok([clamp(w), clamp(h)])
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: [f32; 2] = [1280.0, 800.0];

    fn go(req: SizeRequest, max: u32) -> Result<[u32; 2], SizeError> {
        resolve(VIEW, &req, max, "export limit")
    }

    #[test]
    fn defaults_to_the_viewport_and_keeps_its_aspect() {
        assert_eq!(go(SizeRequest::default(), 4096), Ok([1280, 800]));
        assert_eq!(go(SizeRequest { width: Some(640), ..Default::default() }, 4096), Ok([640, 400]));
        assert_eq!(go(SizeRequest { height: Some(400), ..Default::default() }, 4096), Ok([640, 400]));
        assert_eq!(go(SizeRequest { width: Some(300), height: Some(300), ..Default::default() }, 4096), Ok([300, 300]));
    }

    #[test]
    fn the_multiplier_scales_every_source() {
        for (scale, w, h) in [(1, 1280, 800), (2, 2560, 1600), (4, 5120, 3200)] {
            assert_eq!(go(SizeRequest { scale: Some(scale), ..Default::default() }, 8192), Ok([w, h]));
        }
        assert_eq!(go(SizeRequest { preset: Some("hd".into()), scale: Some(2), ..Default::default() }, 8192), Ok([3840, 2160]));
        assert_eq!(go(SizeRequest { width: Some(100), height: Some(50), scale: Some(4), ..Default::default() }, 8192), Ok([400, 200]));
    }

    #[test]
    fn presets_resolve_and_explicit_edges_override() {
        assert_eq!(go(SizeRequest { preset: Some("4K".into()), ..Default::default() }, 8192), Ok([3840, 2160]));
        assert_eq!(go(SizeRequest { preset: Some("hd".into()), width: Some(960), ..Default::default() }, 4096), Ok([960, 540]));
        assert!(matches!(go(SizeRequest { preset: Some("nope".into()), ..Default::default() }, 4096), Err(SizeError::UnknownPreset(_))));
        assert!(PRESETS.iter().all(|p| p.width >= MIN_SIDE && p.height >= MIN_SIDE));
    }

    #[test]
    fn over_the_cap_is_a_clear_error() {
        let e = go(SizeRequest { scale: Some(4), ..Default::default() }, 4096).unwrap_err();
        assert_eq!(e, SizeError::TooLarge { width: 5120, height: 3200, max: 4096, limit: "export limit" });
        let text = e.to_string();
        assert!(text.contains("5120x3200") && text.contains("4096") && text.contains("export limit"), "{text}");
        assert!(go(SizeRequest { scale: Some(2), ..Default::default() }, 2559).is_err());
        assert!(go(SizeRequest { scale: Some(2), ..Default::default() }, 2560).is_ok());
    }

    #[test]
    fn hostile_numbers_are_errors_not_panics() {
        assert_eq!(go(SizeRequest { scale: Some(3), ..Default::default() }, 4096), Err(SizeError::BadScale(3)));
        assert_eq!(go(SizeRequest { scale: Some(0), ..Default::default() }, 4096), Err(SizeError::BadScale(0)));
        assert!(matches!(go(SizeRequest { width: Some(0), height: Some(0), ..Default::default() }, 4096), Err(SizeError::TooSmall { .. })));
        assert!(matches!(
            go(SizeRequest { width: Some(u32::MAX), height: Some(u32::MAX), scale: Some(4), ..Default::default() }, 4096),
            Err(SizeError::TooLarge { .. })
        ));
        assert!(matches!(go(SizeRequest { width: Some(u32::MAX), ..Default::default() }, 4096), Err(SizeError::TooLarge { .. })));
        assert!(resolve([f32::NAN, f32::INFINITY], &SizeRequest::default(), 4096, "x").is_err(), "a NaN viewport collapses to 1 px: too small");
        assert!(resolve([0.0, -5.0], &SizeRequest { width: Some(64), ..Default::default() }, 4096, "x").is_ok());
    }
}
