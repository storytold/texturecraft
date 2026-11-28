//! The groups the channel manager lists material channels under

use crate::Channel;

/// A group of material channels, in the manager's reading order
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelGroup {
    Base,
    Specular,
    Transmission,
    Subsurface,
    Coat,
    Sheen,
    Emission,
    ThinFilm,
    Geometry,
    Miscellaneous,
    User,
}

impl ChannelGroup {
    pub const ALL: [ChannelGroup; 11] = [
        ChannelGroup::Base,
        ChannelGroup::Specular,
        ChannelGroup::Transmission,
        ChannelGroup::Subsurface,
        ChannelGroup::Coat,
        ChannelGroup::Sheen,
        ChannelGroup::Emission,
        ChannelGroup::ThinFilm,
        ChannelGroup::Geometry,
        ChannelGroup::Miscellaneous,
        ChannelGroup::User,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ChannelGroup::Base => "Base",
            ChannelGroup::Specular => "Specular",
            ChannelGroup::Transmission => "Transmission",
            ChannelGroup::Subsurface => "Subsurface",
            ChannelGroup::Coat => "Coat",
            ChannelGroup::Sheen => "Fuzz / Sheen",
            ChannelGroup::Emission => "Emission",
            ChannelGroup::ThinFilm => "Thin-film",
            ChannelGroup::Geometry => "Geometry",
            ChannelGroup::Miscellaneous => "Miscellaneous",
            ChannelGroup::User => "User",
        }
    }

    /// The group's channels, in the order the manager lists them
    pub fn channels(self) -> &'static [Channel] {
        use Channel as C;
        match self {
            ChannelGroup::Base => &[C::BaseWeight, C::BaseColor, C::BaseDiffuseRoughness, C::Metallic],
            ChannelGroup::Specular => &[C::SpecularWeight, C::SpecularColor, C::Roughness, C::SpecularLevel, C::AnisotropyLevel, C::AnisotropyAngle],
            ChannelGroup::Transmission => &[C::Transmission, C::TransmissionColor],
            ChannelGroup::Subsurface => &[C::ScatterWeight, C::SubsurfaceColor, C::SubsurfaceRadiusScale],
            ChannelGroup::Coat => &[C::CoatWeight, C::CoatColor, C::CoatRoughness, C::CoatAnisotropy, C::CoatSpecularLevel, C::CoatNormal, C::CoatTangent],
            ChannelGroup::Sheen => &[C::SheenWeight, C::SheenColor, C::SheenRoughness],
            ChannelGroup::Emission => &[C::Emissive],
            ChannelGroup::ThinFilm => &[C::ThinFilmWeight],
            ChannelGroup::Geometry => &[C::Opacity, C::Normal, C::Height, C::AmbientOcclusion, C::Tangent],
            ChannelGroup::Miscellaneous => &[C::BlendingMask, C::Diffuse, C::Displacement, C::Ior, C::Reflection, C::Specular, C::Glossiness, C::Transmissive],
            ChannelGroup::User => &[
                C::User0,
                C::User1,
                C::User2,
                C::User3,
                C::User4,
                C::User5,
                C::User6,
                C::User7,
                C::User8,
                C::User9,
                C::User10,
                C::User11,
                C::User12,
                C::User13,
                C::User14,
                C::User15,
                C::User16,
                C::User17,
                C::User18,
                C::User19,
                C::User20,
                C::User21,
                C::User22,
                C::User23,
                C::User24,
                C::User25,
                C::User26,
                C::User27,
                C::User28,
                C::User29,
                C::User30,
                C::User31,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_channel_is_listed_in_exactly_one_group() {
        // A channel added to `Channel` but left out here could never be added from the manager
        let mut listed: Vec<Channel> = ChannelGroup::ALL.iter().flat_map(|group| group.channels().iter().copied()).collect();
        let count = listed.len();
        listed.sort();
        listed.dedup();
        assert_eq!(listed.len(), count, "a channel is listed twice");
        assert_eq!(listed, Channel::ALL.to_vec());
    }
}
