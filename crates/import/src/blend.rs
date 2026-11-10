//! The blend table: the file's stored blend mode (`DataBlending.blendingMode`) as TextureCraft's [`BlendMode`], with how
//! sure each entry is, the per-channel defaults a sparse blending list leaves out, and [`ResolvedBlend`], what one
//! channel of one layer or effect gets.
//!
//! The numbers are the same in every file format seen; each was read off a layer named through the reference app's
//! public API. The separable modes' formulas are measured to within rounding. The four colour modes (tint, saturation,
//! colour, value) fit an HSV form with doubled value, close but not within one 8-bit step everywhere, so they are
//! approximated. On a channel of one value per texel, tint, saturation and colour keep what is below (value and the
//! normal-map modes act as normal). TextureCraft's own formulas for some modes differ from the measured ones (divide,
//! linear light, soft light, dodge and burn edges); those differences show in the safety net, not here.

use texturecraft_core::BlendMode;
use texturecraft_import_reader::model::{ChannelBlending, ProjectBlendMode, ProjectChannel, blending_for};

use crate::status::Status;

/// How well a mapping is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidence {
    /// The number's name and the formula are both measured.
    Identified,
    /// Name measured; the formula is close but misses the acceptance limit.
    Close,
    /// A nearest guess for a number outside the known list.
    Guessed,
}

impl Confidence {
    /// True if a layer using the mode can still be called exact.
    pub fn is_exact(self) -> bool {
        self == Confidence::Identified
    }
}

/// One mapped mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendMapping {
    pub mode: BlendMode,
    pub confidence: Confidence,
}

/// The file's mode → TextureCraft's, by stored number (0 to 31).
const PROJECT_BLEND_MODES: &[(ProjectBlendMode, BlendMode, Confidence)] = &[
    (ProjectBlendMode::Disable, BlendMode::Disable, Confidence::Identified),
    (ProjectBlendMode::Replace, BlendMode::Replace, Confidence::Identified),
    (ProjectBlendMode::Normal, BlendMode::Normal, Confidence::Identified),
    (ProjectBlendMode::Darken, BlendMode::Darken, Confidence::Identified),
    (ProjectBlendMode::Multiply, BlendMode::Multiply, Confidence::Identified),
    (ProjectBlendMode::ColorBurn, BlendMode::ColorBurn, Confidence::Identified),
    (ProjectBlendMode::LinearBurn, BlendMode::LinearBurn, Confidence::Identified),
    (ProjectBlendMode::Lighten, BlendMode::Lighten, Confidence::Identified),
    (ProjectBlendMode::Screen, BlendMode::Screen, Confidence::Identified),
    (ProjectBlendMode::ColorDodge, BlendMode::ColorDodge, Confidence::Identified),
    (ProjectBlendMode::LinearDodge, BlendMode::LinearDodge, Confidence::Identified),
    (ProjectBlendMode::Overlay, BlendMode::Overlay, Confidence::Identified),
    (ProjectBlendMode::SoftLight, BlendMode::SoftLight, Confidence::Identified),
    (ProjectBlendMode::HardLight, BlendMode::HardLight, Confidence::Identified),
    (ProjectBlendMode::VividLight, BlendMode::VividLight, Confidence::Identified),
    (ProjectBlendMode::LinearLight, BlendMode::LinearLight, Confidence::Identified),
    (ProjectBlendMode::PinLight, BlendMode::PinLight, Confidence::Identified),
    (ProjectBlendMode::Difference, BlendMode::Difference, Confidence::Identified),
    (ProjectBlendMode::Exclusion, BlendMode::Exclusion, Confidence::Identified),
    (ProjectBlendMode::Tint, BlendMode::HsvTint, Confidence::Close),
    (ProjectBlendMode::Saturation, BlendMode::HsvSaturation, Confidence::Close),
    (ProjectBlendMode::Color, BlendMode::HsvColor, Confidence::Close),
    (ProjectBlendMode::Value, BlendMode::HsvValue, Confidence::Close),
    (ProjectBlendMode::Divide, BlendMode::Divide, Confidence::Identified),
    (ProjectBlendMode::InverseDivide, BlendMode::InverseDivide, Confidence::Identified),
    (ProjectBlendMode::Passthrough, BlendMode::Passthrough, Confidence::Identified),
    (ProjectBlendMode::NormalMapCombine, BlendMode::NormalMapCombine, Confidence::Identified),
    (ProjectBlendMode::NormalMapDetail, BlendMode::NormalMapDetail, Confidence::Identified),
    (ProjectBlendMode::NormalMapInverseDetail, BlendMode::NormalMapInverseDetail, Confidence::Identified),
    (ProjectBlendMode::Subtract, BlendMode::Subtract, Confidence::Identified),
    (ProjectBlendMode::InverseSubtract, BlendMode::InverseSubtract, Confidence::Identified),
    (ProjectBlendMode::SignedAddition, BlendMode::SignedAddition, Confidence::Identified),
];

/// The mapping for a channel of colours or vectors.
pub fn map(mode: ProjectBlendMode) -> BlendMapping {
    PROJECT_BLEND_MODES.iter().find(|(m, ..)| *m == mode).map_or(
        BlendMapping { mode: BlendMode::Normal, confidence: Confidence::Guessed },
        |(_, mode, confidence)| BlendMapping { mode: *mode, confidence: *confidence },
    )
}

/// The mapping for a channel of one value per texel, where some modes behave otherwise (measured).
pub fn map_single_valued(mode: ProjectBlendMode) -> BlendMapping {
    let measured = |mode| BlendMapping { mode, confidence: Confidence::Identified };
    match mode {
        ProjectBlendMode::Tint | ProjectBlendMode::Saturation | ProjectBlendMode::Color => measured(BlendMode::Disable),
        ProjectBlendMode::Value | ProjectBlendMode::NormalMapCombine | ProjectBlendMode::NormalMapDetail | ProjectBlendMode::NormalMapInverseDetail => {
            measured(BlendMode::Normal)
        }
        _ => map(mode),
    }
}

/// What holds a blending list; decides the default for a channel the list leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    /// A layer, folder or content effect.
    Layer,
    /// An effect group (`DataActionGroup`).
    EffectGroup,
    /// A filter-like effect that transforms what is below it (levels, filters): normal at full strength.
    Filter,
}

/// The file's default mode for a channel missing from a blending list (newer formats store only entries that differ):
/// normal except height = linear dodge, normal = normal map detail, ambient occlusion = multiply; effect groups default
/// to passthrough, filters to normal.
pub fn default_mode(channel: ProjectChannel, holder: Holder) -> ProjectBlendMode {
    match (holder, channel) {
        (Holder::EffectGroup, _) => ProjectBlendMode::Passthrough,
        (Holder::Filter, _) => ProjectBlendMode::Normal,
        (_, ProjectChannel::Height) => ProjectBlendMode::LinearDodge,
        (_, ProjectChannel::Normal) => ProjectBlendMode::NormalMapDetail,
        (_, ProjectChannel::AmbientOcclusion) => ProjectBlendMode::Multiply,
        _ => ProjectBlendMode::Normal,
    }
}

/// The blend mode and opacity one channel gets from a stored blending list.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedBlend {
    /// As stored (or the file's default for the channel).
    pub project_mode: ProjectBlendMode,
    pub mapping: BlendMapping,
    /// 0..1.
    pub opacity: f32,
    /// True when the list had no entry for the channel (the default was taken).
    pub defaulted: bool,
}

impl ResolvedBlend {
    /// The blending of `channel` in `entries` (a layer's, folder's or effect's list).
    pub fn of(entries: &[ChannelBlending], channel: ProjectChannel, holder: Holder) -> ResolvedBlend {
        let (project_mode, opacity, defaulted) = match blending_for(entries, channel) {
            Some(b) => (b.mode, b.opacity, false),
            None => (default_mode(channel, holder), 1.0, true),
        };
        let mapping = if channel.is_single_valued() { map_single_valued(project_mode) } else { map(project_mode) };
        let mut resolved = ResolvedBlend { project_mode, mapping, opacity: unit(opacity), defaulted };
        // Normal-map modes were only measured on the normal channel.
        if is_normal_map(resolved.mode()) && channel != ProjectChannel::Normal {
            resolved.mapping.confidence = Confidence::Close;
        }
        resolved
    }

    /// The grey entry of a mask effect: the entry without channel bits, else the first one.
    pub fn of_mask(entries: &[ChannelBlending]) -> ResolvedBlend {
        let (project_mode, opacity) = match entries.iter().find(|b| b.channels.is_empty()).or(entries.first()) {
            Some(b) => (b.mode, b.opacity),
            None => (ProjectBlendMode::Normal, 1.0),
        };
        ResolvedBlend { project_mode, mapping: map_single_valued(project_mode), opacity: unit(opacity), defaulted: entries.is_empty() }
    }

    pub fn mode(&self) -> BlendMode {
        self.mapping.mode
    }

    /// False when the entry leaves the channel untouched (disabled or at zero opacity).
    pub fn contributes(&self) -> bool {
        self.mode() != BlendMode::Disable && self.opacity > 0.0
    }

    /// Exact unless the mode is close or a guess.
    pub fn status(&self) -> Status {
        match self.mapping.confidence {
            Confidence::Identified => Status::Exact,
            Confidence::Close => Status::approximated(format!("blend mode {} imported as {:?} (formula close, not exact)", self.project_mode, self.mode())),
            Confidence::Guessed => Status::approximated(format!("unknown blend {} imported as {:?}", self.project_mode, self.mode())),
        }
    }
}

fn is_normal_map(mode: BlendMode) -> bool {
    matches!(mode, BlendMode::NormalMapCombine | BlendMode::NormalMapDetail | BlendMode::NormalMapInverseDetail)
}

/// Clamps an opacity to 0..1 (non-finite values from a corrupt file count as 1).
pub fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_import_reader::model::ChannelMask;

    fn entry(channels: u64, mode: u64, opacity: f32) -> ChannelBlending {
        ChannelBlending { channels: ChannelMask(channels), mode: ProjectBlendMode::from_raw(mode), opacity }
    }

    #[test]
    fn every_stored_number_maps_and_others_are_guessed() {
        for raw in 0..32 {
            let m = map(ProjectBlendMode::from_raw(raw));
            assert_ne!(m.confidence, Confidence::Guessed, "value {raw}");
        }
        assert_eq!(map(ProjectBlendMode::from_raw(10)).mode, BlendMode::LinearDodge);
        assert_eq!(map(ProjectBlendMode::from_raw(31)).mode, BlendMode::SignedAddition);
        assert_eq!(map(ProjectBlendMode::from_raw(19)).mode, BlendMode::HsvTint);
        assert_eq!(map(ProjectBlendMode::from_raw(99)), BlendMapping { mode: BlendMode::Normal, confidence: Confidence::Guessed });
        assert_eq!(map_single_valued(ProjectBlendMode::from_raw(21)).mode, BlendMode::Disable);
        assert_eq!(map_single_valued(ProjectBlendMode::from_raw(22)).mode, BlendMode::Normal);
        assert!(map_single_valued(ProjectBlendMode::from_raw(22)).confidence.is_exact());
    }

    #[test]
    fn stored_entries_win_and_missing_ones_take_defaults() {
        let entries = [entry(1 | 16_777_216, 4, 0.5)];
        let base = ResolvedBlend::of(&entries, ProjectChannel::BaseColor, Holder::Layer);
        assert_eq!((base.mode(), base.opacity, base.defaulted), (BlendMode::Multiply, 0.5, false));
        let height = ResolvedBlend::of(&entries, ProjectChannel::Height, Holder::Layer);
        assert_eq!((height.mode(), height.defaulted), (BlendMode::LinearDodge, true));
        assert!(ResolvedBlend::of(&[], ProjectChannel::Normal, Holder::Layer).status().is_exact());
        assert_eq!(ResolvedBlend::of(&[], ProjectChannel::Height, Holder::EffectGroup).mode(), BlendMode::Passthrough);
        let detail_on_colour = ResolvedBlend::of(&[entry(1, 27, 1.0)], ProjectChannel::BaseColor, Holder::Layer);
        assert!(!detail_on_colour.status().is_exact(), "normal-map modes are only measured on normals");
    }

    #[test]
    fn masks_read_the_grey_entry_and_bad_opacity_is_clamped() {
        let m = ResolvedBlend::of_mask(&[entry(1, 2, 1.0), entry(0, 4, 2.0)]);
        assert_eq!((m.mode(), m.opacity), (BlendMode::Multiply, 1.0));
        assert_eq!(unit(f32::NAN), 1.0);
        assert!(!ResolvedBlend::of_mask(&[entry(0, 0, 1.0)]).contributes());
        assert!(!ResolvedBlend::of_mask(&[entry(0, 2, 0.0)]).contributes());
    }
}
