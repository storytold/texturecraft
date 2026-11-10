//! The channel table: the file's channel bits (`DataChannel.type`, texture-cache leaf names, `channelTypes` masks) and
//! stored formats against TextureCraft's [`Channel`]s, the values the original shows where no layer covers a channel,
//! and [`SetChannels`], the channels one texture set declares seen from both sides.
//!
//! Values are kept as TextureCraft holds them: colours linear, scalars as they are, normals packed (`n · 0.5 + 0.5`),
//! height signed. The file stores uniform values linear too, so they carry over unchanged.

use texturecraft_core::{Channel, ChannelFormat, ChannelKind};
use texturecraft_import_reader::model::{ChannelMask, ProjectChannel};

/// How well a channel's mapping is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelConfidence {
    /// Read off the texture cache of real projects (bits, meaning and defaults).
    Measured,
    /// The bit is named after the outputs that feed it; the meaning matches TextureCraft's channel but was not measured.
    Named,
}

/// File channel bit, TextureCraft channel, the file's label and how sure the mapping is.
const CHANNELS: &[(u64, Channel, &str, ChannelConfidence)] = &[
    (1, Channel::BaseColor, "Base Color", ChannelConfidence::Measured),
    (2, Channel::Height, "Height", ChannelConfidence::Measured),
    (4, Channel::Specular, "Specular", ChannelConfidence::Named),
    (8, Channel::Opacity, "Opacity", ChannelConfidence::Measured),
    (16, Channel::Emissive, "Emissive", ChannelConfidence::Measured),
    (64, Channel::Glossiness, "Glossiness", ChannelConfidence::Named),
    (128, Channel::Roughness, "Roughness", ChannelConfidence::Measured),
    (512, Channel::AnisotropyAngle, "Anisotropy Angle", ChannelConfidence::Named),
    (8192, Channel::Metallic, "Metallic", ChannelConfidence::Measured),
    (4_194_304, Channel::Normal, "Normal", ChannelConfidence::Measured),
    (8_388_608, Channel::AmbientOcclusion, "Ambient Occlusion", ChannelConfidence::Named),
    (33_554_432, Channel::SpecularLevel, "Specular Level", ChannelConfidence::Named),
    (134_217_728, Channel::ScatterWeight, "Scattering", ChannelConfidence::Named),
];

// File-format data: `DataChannel.format`, the stored format of a channel (read off real projects).
const FORMAT_SRGB8: u64 = 0;
const FORMAT_L8: u64 = 1;
const FORMAT_L16: u64 = 3;
const FORMAT_L16F: u64 = 5;
const FORMAT_RGB16F: u64 = 6;

/// What the original shows where no layer covers base colour: 0.5 linear (0.735 as sRGB), measured on partially
/// covered texels of a real project.
const UNCOVERED_BASE_COLOR: f32 = 0.5;
/// Roughness where no layer covers it (the same texels).
const UNCOVERED_ROUGHNESS: f32 = 0.3;

/// TextureCraft's channel for a file channel bit.
pub fn channel_for_bit(bit: u64) -> Option<Channel> {
    CHANNELS.iter().find(|(b, ..)| *b == bit).map(|(_, c, ..)| *c)
}

/// The file's bit of one of TextureCraft's channels (0 for channels the file has no bit for).
pub fn bit_of(channel: Channel) -> u64 {
    CHANNELS.iter().find(|(_, c, ..)| *c == channel).map_or(0, |(b, ..)| *b)
}

/// How well the mapping of `channel` is known.
pub fn confidence(channel: Channel) -> ChannelConfidence {
    CHANNELS.iter().find(|(_, c, ..)| *c == channel).map_or(ChannelConfidence::Named, |(.., k)| *k)
}

/// True for the bits of the user channels (`user 0` … `user 7`).
pub fn is_user_channel(bit: u64) -> bool {
    bit.is_power_of_two() && (USER_FIRST_BIT..=USER_LAST_BIT).contains(&bit.trailing_zeros())
}

/// File-format data: the bits of the user channels in a channel mask (the reader maps newer files' bits back to these).
const USER_FIRST_BIT: u32 = 14;
const USER_LAST_BIT: u32 = 21;

/// The file's name for a channel bit, for the report (`user 3` for user channels, see the reader).
pub fn project_channel_name(bit: u64) -> String {
    match CHANNELS.iter().find(|(b, ..)| *b == bit) {
        Some((_, _, name, _)) => (*name).to_string(),
        None => ProjectChannel::from_bit(bit).to_string(),
    }
}

/// TextureCraft's storage format for a channel declared with the file's `DataChannel.format` (when known).
pub fn channel_format(channel: Channel, project_format: Option<u64>) -> ChannelFormat {
    let color = channel.components() == 3;
    match (project_format, color) {
        (Some(FORMAT_SRGB8), true) if channel.kind() == ChannelKind::Color => ChannelFormat::Srgb8,
        (Some(FORMAT_SRGB8 | FORMAT_L8), true) => ChannelFormat::Rgb8,
        (Some(FORMAT_SRGB8 | FORMAT_L8), false) => ChannelFormat::L8,
        (Some(FORMAT_L16), true) => ChannelFormat::Rgb16,
        (Some(FORMAT_L16), false) => ChannelFormat::L16,
        (Some(FORMAT_L16F | FORMAT_RGB16F), true) => ChannelFormat::Rgb16F,
        (Some(FORMAT_L16F | FORMAT_RGB16F), false) => ChannelFormat::L16F,
        _ => ChannelFormat::default_for(channel),
    }
}

/// What the original shows where no layer covers `channel`, as TextureCraft holds it (measured for base colour,
/// roughness, metallic, normal and height; TextureCraft's own default for the rest).
pub fn project_channel_default(channel: Channel) -> [f32; 3] {
    match channel {
        Channel::BaseColor => [UNCOVERED_BASE_COLOR; 3],
        Channel::Roughness => [UNCOVERED_ROUGHNESS; 3],
        Channel::Metallic | Channel::Height | Channel::Emissive => [0.0; 3],
        Channel::Normal => [0.5, 0.5, 1.0],
        Channel::Opacity | Channel::AmbientOcclusion => [1.0; 3],
        other => other.default_value(),
    }
}

/// One declared channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclaredChannel {
    pub channel: Channel,
    pub format: ChannelFormat,
}

/// The channels one texture set declares, in declaration order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetChannels {
    declared: Vec<DeclaredChannel>,
}

impl SetChannels {
    pub fn new(declared: Vec<DeclaredChannel>) -> SetChannels {
        SetChannels { declared }
    }

    /// Adds `channel` unless it is declared already.
    pub fn declare(&mut self, channel: Channel, format: ChannelFormat) {
        if !self.has(channel) {
            self.declared.push(DeclaredChannel { channel, format });
        }
    }

    pub fn declared(&self) -> &[DeclaredChannel] {
        &self.declared
    }

    pub fn all(&self) -> impl Iterator<Item = Channel> + '_ {
        self.declared.iter().map(|d| d.channel)
    }

    pub fn has(&self, channel: Channel) -> bool {
        self.declared.iter().any(|d| d.channel == channel)
    }

    /// TextureCraft's channel for a file channel, if the set declares it.
    pub fn ours(&self, channel: ProjectChannel) -> Option<Channel> {
        channel_for_bit(channel.bit()).filter(|c| self.has(*c))
    }

    /// The file channel of one of TextureCraft's.
    pub fn project_channel(&self, channel: Channel) -> ProjectChannel {
        ProjectChannel::from_bit(bit_of(channel))
    }

    /// True if `mask` (an effect's `channelTypes`) includes `channel`; "all" includes every channel.
    pub fn mask_has(&self, mask: ChannelMask, channel: Channel) -> bool {
        mask.is_all() || mask.contains(self.project_channel(channel))
    }

    /// The file's name of one of TextureCraft's channels, for the report.
    pub fn label(&self, channel: Channel) -> String {
        project_channel_name(bit_of(channel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_map_both_ways_and_unknown_ones_are_named() {
        assert_eq!(channel_for_bit(4_194_304), Some(Channel::Normal));
        assert_eq!(bit_of(Channel::Roughness), 128);
        assert_eq!(channel_for_bit(16_777_216), None, "diffuse has no counterpart");
        assert_eq!(project_channel_name(1 << 15), "user 1");
        assert_eq!(project_channel_name(1), "Base Color");
        assert!(is_user_channel(1 << 15) && !is_user_channel(256) && !is_user_channel(0));
        for (bit, channel, ..) in CHANNELS {
            assert_eq!(channel_for_bit(*bit), Some(*channel));
            assert_eq!(bit_of(*channel), *bit);
        }
    }

    #[test]
    fn formats_follow_the_declaration() {
        assert_eq!(channel_format(Channel::BaseColor, Some(0)), ChannelFormat::Srgb8);
        assert_eq!(channel_format(Channel::Roughness, Some(0)), ChannelFormat::L8);
        assert_eq!(channel_format(Channel::Height, Some(5)), ChannelFormat::L16F);
        assert_eq!(channel_format(Channel::Normal, Some(6)), ChannelFormat::Rgb16F);
        assert_eq!(channel_format(Channel::Metallic, None), ChannelFormat::L8);
        assert_eq!(channel_format(Channel::Metallic, Some(99)), ChannelFormat::L8);
    }

    #[test]
    fn declared_channels_answer_for_masks() {
        let mut s = SetChannels::default();
        s.declare(Channel::BaseColor, ChannelFormat::Srgb8);
        s.declare(Channel::Height, ChannelFormat::L16F);
        s.declare(Channel::Height, ChannelFormat::L8);
        assert_eq!(s.declared().len(), 2);
        assert_eq!(s.ours(ProjectChannel::Height), Some(Channel::Height));
        assert_eq!(s.ours(ProjectChannel::Metallic), None, "not declared");
        assert!(s.mask_has(ChannelMask(u64::MAX), Channel::BaseColor));
        assert!(!s.mask_has(ChannelMask(2), Channel::BaseColor));
        assert_eq!(s.label(Channel::BaseColor), "Base Color");
        assert_eq!(project_channel_default(Channel::Roughness), [0.3; 3]);
        assert_eq!(project_channel_default(Channel::SheenColor), Channel::SheenColor.default_value());
    }
}
