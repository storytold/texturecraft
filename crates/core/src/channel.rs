//! Material channels of a texture set.

use serde::{Deserialize, Serialize};

/// A material channel. Order is the display order in the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Channel {
    BaseColor,
    Metallic,
    Roughness,
    Normal,
    Height,
    Emissive,
    Opacity,
    AmbientOcclusion,
    SpecularLevel,
    /// Specular/glossiness workflow: the F0 color.
    Specular,
    /// Specular/glossiness workflow: 1 − roughness.
    Glossiness,
    /// Clear coat: weight of a GGX dielectric layer (IOR 1.5) over the base.
    CoatWeight,
    /// Clear coat roughness.
    CoatRoughness,
    /// Sheen (cloth fuzz) layer weight.
    SheenWeight,
    /// Sheen tint.
    SheenColor,
    /// Sheen roughness.
    SheenRoughness,
    /// Anisotropy: how much the specular highlight stretches along the tangent direction.
    AnisotropyLevel,
    /// Anisotropy direction: 0..1 turns the tangent by 0..π.
    AnisotropyAngle,
    /// Subsurface scattering weight (scatter color and radius are project shader settings).
    ScatterWeight,
    /// Transmission: how much of the dielectric base lets light through (refraction at the
    /// project's IOR, tinted by the base color) instead of reflecting it diffusely.
    Transmission,
    /// Tangent-space normal of the clear coat (relative to the mesh normal).
    CoatNormal,
    /// How much of the base layer shows (stored and exported: our shader does not read it)
    BaseWeight,
    /// Roughness of the diffuse base (stored and exported: our shader does not read it)
    BaseDiffuseRoughness,
    /// Strength of the specular lobe (stored and exported: our shader does not read it)
    SpecularWeight,
    /// Tint of the specular lobe (stored and exported: our shader does not read it)
    SpecularColor,
    /// Tint of transmitted light (stored and exported: our shader does not read it)
    TransmissionColor,
    /// Subsurface scattering color (stored and exported: our shader does not read it)
    SubsurfaceColor,
    /// Scattering distance per color (stored and exported: our shader does not read it)
    SubsurfaceRadiusScale,
    /// Clear coat tint (stored and exported: our shader does not read it)
    CoatColor,
    /// Clear coat anisotropy (stored and exported: our shader does not read it)
    CoatAnisotropy,
    /// Clear coat reflectance (stored and exported: our shader does not read it)
    CoatSpecularLevel,
    /// Tangent-space direction of the clear coat anisotropy, packed like a normal (stored and exported: our shader does not read it)
    CoatTangent,
    /// Thin-film iridescence weight (stored and exported: our shader does not read it)
    ThinFilmWeight,
    /// Tangent-space anisotropy direction, packed like a normal (stored and exported: our shader does not read it)
    Tangent,
    /// Mask for blending materials in an engine (stored and exported only)
    BlendingMask,
    /// Diffuse color of older shaders (stored and exported only)
    Diffuse,
    /// Signed displacement for render engines (stored and exported only)
    Displacement,
    /// Index of refraction, 0..1 as the target shader maps it (stored and exported only)
    Ior,
    /// Reflection amount of older shaders (stored and exported only)
    Reflection,
    /// Transmissive color of older shaders (stored and exported only)
    Transmissive,
    /// Free channels for anything a target shader needs (grey, stored and exported)
    User0,
    User1,
    User2,
    User3,
    User4,
    User5,
    User6,
    User7,
    User8,
    User9,
    User10,
    User11,
    User12,
    User13,
    User14,
    User15,
    User16,
    User17,
    User18,
    User19,
    User20,
    User21,
    User22,
    User23,
    User24,
    User25,
    User26,
    User27,
    User28,
    User29,
    User30,
    User31,
}

/// How a channel's values are stored and interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    /// Linear RGB color, displayed and exported sRGB-encoded.
    Color,
    /// Tangent-space normal packed into 0..1 (`n * 0.5 + 0.5`).
    NormalMap,
    /// A 0..1 scalar.
    Scalar,
    /// A signed scalar (height).
    Signed,
}

impl Channel {
    pub const ALL: [Channel; 72] = [
        Channel::BaseColor,
        Channel::Metallic,
        Channel::Roughness,
        Channel::Normal,
        Channel::Height,
        Channel::Emissive,
        Channel::Opacity,
        Channel::AmbientOcclusion,
        Channel::SpecularLevel,
        Channel::Specular,
        Channel::Glossiness,
        Channel::CoatWeight,
        Channel::CoatRoughness,
        Channel::SheenWeight,
        Channel::SheenColor,
        Channel::SheenRoughness,
        Channel::AnisotropyLevel,
        Channel::AnisotropyAngle,
        Channel::ScatterWeight,
        Channel::Transmission,
        Channel::CoatNormal,
        Channel::BaseWeight,
        Channel::BaseDiffuseRoughness,
        Channel::SpecularWeight,
        Channel::SpecularColor,
        Channel::TransmissionColor,
        Channel::SubsurfaceColor,
        Channel::SubsurfaceRadiusScale,
        Channel::CoatColor,
        Channel::CoatAnisotropy,
        Channel::CoatSpecularLevel,
        Channel::CoatTangent,
        Channel::ThinFilmWeight,
        Channel::Tangent,
        Channel::BlendingMask,
        Channel::Diffuse,
        Channel::Displacement,
        Channel::Ior,
        Channel::Reflection,
        Channel::Transmissive,
        Channel::User0,
        Channel::User1,
        Channel::User2,
        Channel::User3,
        Channel::User4,
        Channel::User5,
        Channel::User6,
        Channel::User7,
        Channel::User8,
        Channel::User9,
        Channel::User10,
        Channel::User11,
        Channel::User12,
        Channel::User13,
        Channel::User14,
        Channel::User15,
        Channel::User16,
        Channel::User17,
        Channel::User18,
        Channel::User19,
        Channel::User20,
        Channel::User21,
        Channel::User22,
        Channel::User23,
        Channel::User24,
        Channel::User25,
        Channel::User26,
        Channel::User27,
        Channel::User28,
        Channel::User29,
        Channel::User30,
        Channel::User31,
    ];

    /// How many channels, from the start of [`Channel::ALL`], our shader reads
    pub const SHADED_COUNT: usize = 21;

    /// The extra shading lobes (clear coat, sheen, anisotropy, subsurface scattering,
    /// transmission). Their defaults switch the lobes off, so a set without them shades exactly
    /// as before.
    pub const LOBES: [Channel; 10] = [
        Channel::CoatWeight,
        Channel::CoatRoughness,
        Channel::SheenWeight,
        Channel::SheenColor,
        Channel::SheenRoughness,
        Channel::AnisotropyLevel,
        Channel::AnisotropyAngle,
        Channel::ScatterWeight,
        Channel::Transmission,
        Channel::CoatNormal,
    ];

    /// The channels of the default "PBR - Metallic Roughness" template.
    pub const DEFAULT_SET: [Channel; 5] = [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Normal, Channel::Height];

    pub fn kind(self) -> ChannelKind {
        match self {
            Channel::BaseColor | Channel::Emissive | Channel::Specular | Channel::SheenColor => ChannelKind::Color,
            Channel::SpecularColor
            | Channel::TransmissionColor
            | Channel::SubsurfaceColor
            | Channel::SubsurfaceRadiusScale
            | Channel::CoatColor
            | Channel::Diffuse
            | Channel::Transmissive => ChannelKind::Color,
            Channel::Normal | Channel::CoatNormal | Channel::Tangent | Channel::CoatTangent => ChannelKind::NormalMap,
            Channel::Height | Channel::Displacement => ChannelKind::Signed,
            _ => ChannelKind::Scalar,
        }
    }

    /// Value components (without coverage): 3 for colors and normals, 1 for scalars.
    pub fn components(self) -> usize {
        match self.kind() {
            ChannelKind::Color | ChannelKind::NormalMap => 3,
            ChannelKind::Scalar | ChannelKind::Signed => 1,
        }
    }

    /// The value a texel has when no layer covers it.
    pub fn default_value(self) -> [f32; 3] {
        match self {
            Channel::BaseColor => [0.5, 0.5, 0.5],
            Channel::Metallic | Channel::Height | Channel::Emissive => [0.0; 3],
            Channel::CoatWeight
            | Channel::SheenWeight
            | Channel::AnisotropyLevel
            | Channel::AnisotropyAngle
            | Channel::ScatterWeight
            | Channel::Transmission => [0.0; 3],
            Channel::CoatRoughness => [0.1; 3],
            Channel::SheenColor => [1.0; 3],
            Channel::SheenRoughness => [0.5; 3],
            Channel::Roughness => [0.5; 3],
            Channel::Normal | Channel::CoatNormal => [0.5, 0.5, 1.0],
            Channel::Opacity | Channel::AmbientOcclusion => [1.0; 3],
            Channel::SpecularLevel | Channel::Glossiness => [0.5; 3],

            // F0 of a common dielectric (4 %)
            Channel::Specular => [0.04; 3],
            Channel::BaseWeight | Channel::SpecularWeight | Channel::SpecularColor | Channel::TransmissionColor | Channel::CoatColor => [1.0; 3],
            Channel::BaseDiffuseRoughness
            | Channel::CoatAnisotropy
            | Channel::ThinFilmWeight
            | Channel::BlendingMask
            | Channel::Displacement
            | Channel::Reflection
            | Channel::Transmissive => [0.0; 3],
            Channel::SubsurfaceColor => [0.8; 3],
            Channel::SubsurfaceRadiusScale => [1.0, 0.5, 0.25],
            Channel::CoatSpecularLevel | Channel::Diffuse | Channel::Ior => [0.5; 3],
            Channel::CoatTangent | Channel::Tangent => [1.0, 0.5, 0.5],
            Channel::User0
            | Channel::User1
            | Channel::User2
            | Channel::User3
            | Channel::User4
            | Channel::User5
            | Channel::User6
            | Channel::User7
            | Channel::User8
            | Channel::User9
            | Channel::User10
            | Channel::User11
            | Channel::User12
            | Channel::User13
            | Channel::User14
            | Channel::User15
            | Channel::User16
            | Channel::User17
            | Channel::User18
            | Channel::User19
            | Channel::User20
            | Channel::User21
            | Channel::User22
            | Channel::User23
            | Channel::User24
            | Channel::User25
            | Channel::User26
            | Channel::User27
            | Channel::User28
            | Channel::User29
            | Channel::User30
            | Channel::User31 => [0.0; 3],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Channel::BaseColor => "Base color",
            Channel::Metallic => "Metallic",
            Channel::Roughness => "Roughness",
            Channel::Normal => "Normal",
            Channel::Height => "Height",
            Channel::Emissive => "Emissive",
            Channel::Opacity => "Opacity",
            Channel::AmbientOcclusion => "Ambient occlusion",
            Channel::SpecularLevel => "Specular level",
            Channel::Specular => "Specular",
            Channel::Glossiness => "Glossiness",
            Channel::CoatWeight => "Coat weight",
            Channel::CoatRoughness => "Coat roughness",
            Channel::SheenWeight => "Sheen weight",
            Channel::SheenColor => "Sheen color",
            Channel::SheenRoughness => "Sheen roughness",
            Channel::AnisotropyLevel => "Anisotropy level",
            Channel::AnisotropyAngle => "Anisotropy angle",
            Channel::ScatterWeight => "Scattering weight",
            Channel::Transmission => "Transmission",
            Channel::CoatNormal => "Coat normal",
            Channel::BaseWeight => "Base weight",
            Channel::BaseDiffuseRoughness => "Base diffuse roughness",
            Channel::SpecularWeight => "Specular weight",
            Channel::SpecularColor => "Specular color",
            Channel::TransmissionColor => "Transmission color",
            Channel::SubsurfaceColor => "Subsurface color",
            Channel::SubsurfaceRadiusScale => "Subsurface radius scale",
            Channel::CoatColor => "Coat color",
            Channel::CoatAnisotropy => "Coat anisotropy",
            Channel::CoatSpecularLevel => "Coat specular level",
            Channel::CoatTangent => "Coat tangent",
            Channel::ThinFilmWeight => "Thin-film weight",
            Channel::Tangent => "Tangent",
            Channel::BlendingMask => "Blending mask",
            Channel::Diffuse => "Diffuse",
            Channel::Displacement => "Displacement",
            Channel::Ior => "Ior",
            Channel::Reflection => "Reflection",
            Channel::Transmissive => "Transmissive",
            Channel::User0 => "User0",
            Channel::User1 => "User1",
            Channel::User2 => "User2",
            Channel::User3 => "User3",
            Channel::User4 => "User4",
            Channel::User5 => "User5",
            Channel::User6 => "User6",
            Channel::User7 => "User7",
            Channel::User8 => "User8",
            Channel::User9 => "User9",
            Channel::User10 => "User10",
            Channel::User11 => "User11",
            Channel::User12 => "User12",
            Channel::User13 => "User13",
            Channel::User14 => "User14",
            Channel::User15 => "User15",
            Channel::User16 => "User16",
            Channel::User17 => "User17",
            Channel::User18 => "User18",
            Channel::User19 => "User19",
            Channel::User20 => "User20",
            Channel::User21 => "User21",
            Channel::User22 => "User22",
            Channel::User23 => "User23",
            Channel::User24 => "User24",
            Channel::User25 => "User25",
            Channel::User26 => "User26",
            Channel::User27 => "User27",
            Channel::User28 => "User28",
            Channel::User29 => "User29",
            Channel::User30 => "User30",
            Channel::User31 => "User31",
        }
    }

    /// Short label used on material channel toggle buttons.
    pub fn short(self) -> &'static str {
        match self {
            Channel::BaseColor => "color",
            Channel::Metallic => "metal",
            Channel::Roughness => "rough",
            Channel::Normal => "nrm",
            Channel::Height => "height",
            Channel::Emissive => "emiss",
            Channel::Opacity => "op",
            Channel::AmbientOcclusion => "ao",
            Channel::SpecularLevel => "spec",
            Channel::Specular => "specular",
            Channel::Glossiness => "gloss",
            Channel::CoatWeight => "coat",
            Channel::CoatRoughness => "coat rough",
            Channel::SheenWeight => "sheen",
            Channel::SheenColor => "sheen color",
            Channel::SheenRoughness => "sheen rough",
            Channel::AnisotropyLevel => "aniso",
            Channel::AnisotropyAngle => "aniso angle",
            Channel::ScatterWeight => "scatter",
            Channel::Transmission => "transm",
            Channel::CoatNormal => "coat nrm",
            Channel::BaseWeight => "base weight",
            Channel::BaseDiffuseRoughness => "diffuse rough",
            Channel::SpecularWeight => "spec weight",
            Channel::SpecularColor => "spec color",
            Channel::TransmissionColor => "transm color",
            Channel::SubsurfaceColor => "sss color",
            Channel::SubsurfaceRadiusScale => "sss radius",
            Channel::CoatColor => "coat color",
            Channel::CoatAnisotropy => "coat aniso",
            Channel::CoatSpecularLevel => "coat spec",
            Channel::CoatTangent => "coat tangent",
            Channel::ThinFilmWeight => "thin film",
            Channel::Tangent => "tangent",
            Channel::BlendingMask => "blend mask",
            Channel::Diffuse => "diffuse",
            Channel::Displacement => "displace",
            Channel::Ior => "ior",
            Channel::Reflection => "reflection",
            Channel::Transmissive => "transmissive",
            Channel::User0 => "user0",
            Channel::User1 => "user1",
            Channel::User2 => "user2",
            Channel::User3 => "user3",
            Channel::User4 => "user4",
            Channel::User5 => "user5",
            Channel::User6 => "user6",
            Channel::User7 => "user7",
            Channel::User8 => "user8",
            Channel::User9 => "user9",
            Channel::User10 => "user10",
            Channel::User11 => "user11",
            Channel::User12 => "user12",
            Channel::User13 => "user13",
            Channel::User14 => "user14",
            Channel::User15 => "user15",
            Channel::User16 => "user16",
            Channel::User17 => "user17",
            Channel::User18 => "user18",
            Channel::User19 => "user19",
            Channel::User20 => "user20",
            Channel::User21 => "user21",
            Channel::User22 => "user22",
            Channel::User23 => "user23",
            Channel::User24 => "user24",
            Channel::User25 => "user25",
            Channel::User26 => "user26",
            Channel::User27 => "user27",
            Channel::User28 => "user28",
            Channel::User29 => "user29",
            Channel::User30 => "user30",
            Channel::User31 => "user31",
        }
    }

    /// Stable machine id (commands, export templates, file names).
    pub fn id(self) -> &'static str {
        match self {
            Channel::BaseColor => "baseColor",
            Channel::Metallic => "metallic",
            Channel::Roughness => "roughness",
            Channel::Normal => "normal",
            Channel::Height => "height",
            Channel::Emissive => "emissive",
            Channel::Opacity => "opacity",
            Channel::AmbientOcclusion => "ambientOcclusion",
            Channel::SpecularLevel => "specularLevel",
            Channel::Specular => "specular",
            Channel::Glossiness => "glossiness",
            Channel::CoatWeight => "coatWeight",
            Channel::CoatRoughness => "coatRoughness",
            Channel::SheenWeight => "sheenWeight",
            Channel::SheenColor => "sheenColor",
            Channel::SheenRoughness => "sheenRoughness",
            Channel::AnisotropyLevel => "anisotropyLevel",
            Channel::AnisotropyAngle => "anisotropyAngle",
            Channel::ScatterWeight => "scatterWeight",
            Channel::Transmission => "transmission",
            Channel::CoatNormal => "coatNormal",
            Channel::BaseWeight => "baseWeight",
            Channel::BaseDiffuseRoughness => "baseDiffuseRoughness",
            Channel::SpecularWeight => "specularWeight",
            Channel::SpecularColor => "specularColor",
            Channel::TransmissionColor => "transmissionColor",
            Channel::SubsurfaceColor => "subsurfaceColor",
            Channel::SubsurfaceRadiusScale => "subsurfaceRadiusScale",
            Channel::CoatColor => "coatColor",
            Channel::CoatAnisotropy => "coatAnisotropy",
            Channel::CoatSpecularLevel => "coatSpecularLevel",
            Channel::CoatTangent => "coatTangent",
            Channel::ThinFilmWeight => "thinFilmWeight",
            Channel::Tangent => "tangent",
            Channel::BlendingMask => "blendingMask",
            Channel::Diffuse => "diffuse",
            Channel::Displacement => "displacement",
            Channel::Ior => "ior",
            Channel::Reflection => "reflection",
            Channel::Transmissive => "transmissive",
            Channel::User0 => "user0",
            Channel::User1 => "user1",
            Channel::User2 => "user2",
            Channel::User3 => "user3",
            Channel::User4 => "user4",
            Channel::User5 => "user5",
            Channel::User6 => "user6",
            Channel::User7 => "user7",
            Channel::User8 => "user8",
            Channel::User9 => "user9",
            Channel::User10 => "user10",
            Channel::User11 => "user11",
            Channel::User12 => "user12",
            Channel::User13 => "user13",
            Channel::User14 => "user14",
            Channel::User15 => "user15",
            Channel::User16 => "user16",
            Channel::User17 => "user17",
            Channel::User18 => "user18",
            Channel::User19 => "user19",
            Channel::User20 => "user20",
            Channel::User21 => "user21",
            Channel::User22 => "user22",
            Channel::User23 => "user23",
            Channel::User24 => "user24",
            Channel::User25 => "user25",
            Channel::User26 => "user26",
            Channel::User27 => "user27",
            Channel::User28 => "user28",
            Channel::User29 => "user29",
            Channel::User30 => "user30",
            Channel::User31 => "user31",
        }
    }

    /// Parse an id, label or short label, ignoring case, spaces, underscores and hyphens.
    pub fn parse(s: &str) -> Option<Channel> {
        if let Some(channel) = Channel::ALL.into_iter().find(|channel| channel.id().eq_ignore_ascii_case(s)) {
            return Some(channel);
        }

        let normalize = |name: &str| name.to_ascii_lowercase().replace([' ', '_', '-'], "");
        let wanted = normalize(s);
        Channel::ALL.into_iter().find(|channel| {
            normalize(channel.id()) == wanted
                || normalize(channel.label()) == wanted
                || normalize(channel.short()) == wanted
                || normalize(&format!("{channel:?}")) == wanted
        })
    }

    /// Position in [`Channel::ALL`] (the declaration order)
    pub fn index(self) -> usize {
        self as usize
    }

    /// Whether our shader reads the channel; the others are stored and exported for other shaders
    pub fn is_shaded(self) -> bool {
        self.index() < Channel::SHADED_COUNT
    }
}

/// Storage format of a channel in a texture set (what export and the UI report).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelFormat {
    Srgb8,
    L8,
    Rgb8,
    L16,
    Rgb16,
    L16F,
    Rgb16F,
    L32F,
    Rgb32F,
}

impl ChannelFormat {
    pub fn default_for(c: Channel) -> ChannelFormat {
        match c.kind() {
            ChannelKind::Color => ChannelFormat::Srgb8,
            ChannelKind::NormalMap => ChannelFormat::Rgb16F,
            ChannelKind::Scalar => ChannelFormat::L8,
            ChannelKind::Signed => ChannelFormat::L16F,
        }
    }

    /// Every format, in the order the channel settings list them
    pub const ALL: [ChannelFormat; 9] = [
        ChannelFormat::Srgb8,
        ChannelFormat::L8,
        ChannelFormat::Rgb8,
        ChannelFormat::L16,
        ChannelFormat::Rgb16,
        ChannelFormat::L16F,
        ChannelFormat::Rgb16F,
        ChannelFormat::L32F,
        ChannelFormat::Rgb32F,
    ];

    /// A format by its label, ignoring case ("srgb8", "L16F")
    pub fn parse(label: &str) -> Option<ChannelFormat> {
        ChannelFormat::ALL.into_iter().find(|format| format.label().eq_ignore_ascii_case(label.trim()))
    }

    /// Grey formats keep one value per texel: a colour channel stored in one turns grey
    pub fn is_grey(self) -> bool {
        matches!(self, ChannelFormat::L8 | ChannelFormat::L16 | ChannelFormat::L16F | ChannelFormat::L32F)
    }

    /// The 16-bit format holding everything an 8-bit one does (itself for the others)
    pub fn at_least_16_bit(self) -> ChannelFormat {
        match self {
            ChannelFormat::Srgb8 | ChannelFormat::Rgb8 => ChannelFormat::Rgb16,
            ChannelFormat::L8 => ChannelFormat::L16,
            other => other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ChannelFormat::Srgb8 => "sRGB8",
            ChannelFormat::L8 => "L8",
            ChannelFormat::Rgb8 => "RGB8",
            ChannelFormat::L16 => "L16",
            ChannelFormat::Rgb16 => "RGB16",
            ChannelFormat::L16F => "L16F",
            ChannelFormat::Rgb16F => "RGB16F",
            ChannelFormat::L32F => "L32F",
            ChannelFormat::Rgb32F => "RGB32F",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_ids_labels_and_shorts() {
        for c in Channel::ALL {
            assert_eq!(Channel::parse(c.id()), Some(c));
            assert_eq!(Channel::parse(c.label()), Some(c));
            assert_eq!(Channel::parse(c.short()), Some(c));
        }

        assert_eq!(Channel::parse("base color"), Some(Channel::BaseColor));
        assert_eq!(Channel::parse("nope"), None);
    }

    #[test]
    fn names_are_unique_and_lobes_default_off() {
        let norm = |x: &str| x.to_ascii_lowercase().replace([' ', '_', '-'], "");
        let mut seen = std::collections::BTreeMap::new();
        for c in Channel::ALL {
            for name in [c.id(), c.label(), c.short()] {
                let prev = seen.insert(norm(name), c);
                assert!(prev.is_none_or(|p| p == c), "`{name}` names both {prev:?} and {c:?}");
            }
        }

        for c in Channel::LOBES {
            assert!(Channel::ALL.contains(&c));

            // Tints, roughnesses and the flat coat normal only matter once their lobe has weight
            let off =
                matches!(c, Channel::CoatRoughness | Channel::SheenColor | Channel::SheenRoughness | Channel::CoatNormal) || c.default_value() == [0.0; 3];
            assert!(off, "{c:?} must default to an inactive lobe");
        }

        assert_eq!(Channel::parse("sheen color"), Some(Channel::SheenColor));
        assert_eq!(Channel::parse("coatWeight"), Some(Channel::CoatWeight));
    }

    #[test]
    fn formats_parse_by_label() {
        for f in ChannelFormat::ALL {
            assert_eq!(ChannelFormat::parse(&f.label().to_lowercase()), Some(f));
            assert!(f.at_least_16_bit().is_grey() == f.is_grey());
        }

        assert_eq!(ChannelFormat::parse("RGB4"), None);
    }

    #[test]
    fn unshaded_channels_round_trip_through_ids() {
        // Project files and commands name channels by id: every added channel must come back as itself
        for c in Channel::ALL.into_iter().filter(|c| !c.is_shaded()) {
            assert_eq!(Channel::parse(c.id()), Some(c));
            let json = serde_json::to_string(&c).unwrap_or_default();
            assert_eq!(serde_json::from_str::<Channel>(&json).ok(), Some(c));
        }

        assert!(Channel::LOBES.iter().chain(&Channel::DEFAULT_SET).all(|c| c.is_shaded()));
        assert_eq!(Channel::parse("User31"), Some(Channel::User31));
        assert_eq!(Channel::parse("thin-film weight"), Some(Channel::ThinFilmWeight));
    }

    #[test]
    fn index_matches_all_order() {
        for (i, c) in Channel::ALL.iter().enumerate() {
            assert_eq!(c.index(), i);
        }
    }
}
