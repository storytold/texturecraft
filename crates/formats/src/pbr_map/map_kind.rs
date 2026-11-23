/// The kinds of map a PBR folder holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MapKind {
    BaseColor,
    Roughness,
    /// Glossiness is the inverse of roughness; the importer inverts it.
    Glossiness,
    Metallic,
    Normal,
    Height,
    AmbientOcclusion,
    Emissive,
    Opacity,
    /// One image: occlusion in red, roughness in green, metallic in blue (`ORM`, `ARM`).
    PackedOrm,
    /// Recognised, but TextureCraft has no channel for it (cavity, specular, ...): reported,
    /// not imported.
    Unused,
}

impl MapKind {
    const IMPORTED: [MapKind; 10] = [
        MapKind::BaseColor,
        MapKind::Roughness,
        MapKind::Glossiness,
        MapKind::Metallic,
        MapKind::Normal,
        MapKind::Height,
        MapKind::AmbientOcclusion,
        MapKind::Emissive,
        MapKind::Opacity,
        MapKind::PackedOrm,
    ];

    /// Stable id for JSON.
    pub fn id(self) -> &'static str {
        match self {
            MapKind::BaseColor => "baseColor",
            MapKind::Roughness => "roughness",
            MapKind::Glossiness => "glossiness",
            MapKind::Metallic => "metallic",
            MapKind::Normal => "normal",
            MapKind::Height => "height",
            MapKind::AmbientOcclusion => "ambientOcclusion",
            MapKind::Emissive => "emissive",
            MapKind::Opacity => "opacity",
            MapKind::PackedOrm => "orm",
            MapKind::Unused => "unused",
        }
    }

    /// The kind with this JSON id.
    pub fn from_id(id: &str) -> Option<MapKind> {
        Self::IMPORTED.into_iter().find(|k| k.id() == id)
    }
}
