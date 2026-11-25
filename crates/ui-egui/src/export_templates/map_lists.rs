//! The maps the Output templates tab lists for dragging onto output channels. Maps TextureCraft
//! has carry its own names; the input maps it does not have yet carry OpenPBR parameter names
//! (OpenPBR Surface specification, Academy Software Foundation) and are listed greyed.

use std::sync::LazyLock;

use texturecraft_engine::core::{BakedMap, Channel};
use texturecraft_engine::formats::{SourceRef, VirtualMap};

use crate::i18n::n_;

/// One entry of a map list: its name and what it reads (None: not available yet).
pub type MapEntry = (&'static str, Option<SourceRef>);

/// A group of the Input maps pane: its name and entries.
pub type MapGroup = (&'static str, Vec<MapEntry>);

/// A TextureCraft channel, under its own name
fn channel(channel: Channel) -> MapEntry {
    (channel.label(), Some(SourceRef::Channel { channel, component: 0 }))
}

/// An OpenPBR input TextureCraft has no channel for yet
fn missing(name: &'static str) -> MapEntry {
    (name, None)
}

/// Input maps in the OpenPBR parameter groups, then TextureCraft's channels outside OpenPBR
pub static INPUT_MAPS: LazyLock<Vec<MapGroup>> = LazyLock::new(|| {
    vec![
        (n_("Base"), vec![missing(n_("Base weight")), channel(Channel::BaseColor), channel(Channel::Metallic), missing(n_("Base diffuse roughness"))]),
        (
            n_("Specular"),
            vec![
                missing(n_("Specular weight")),
                missing(n_("Specular color")),
                channel(Channel::Roughness),
                channel(Channel::SpecularLevel),
                channel(Channel::AnisotropyLevel),
                channel(Channel::AnisotropyAngle),
            ],
        ),
        (n_("Transmission"), vec![channel(Channel::Transmission), missing(n_("Transmission color"))]),
        (n_("Subsurface"), vec![channel(Channel::ScatterWeight), missing(n_("Subsurface color")), missing(n_("Subsurface radius"))]),
        (
            n_("Coat"),
            vec![
                channel(Channel::CoatWeight),
                missing(n_("Coat color")),
                channel(Channel::CoatRoughness),
                missing(n_("Coat roughness anisotropy")),
                missing(n_("Coat IOR")),
                channel(Channel::CoatNormal),
                missing(n_("Coat tangent")),
            ],
        ),
        (n_("Sheen"), vec![channel(Channel::SheenWeight), channel(Channel::SheenColor), channel(Channel::SheenRoughness)]),
        (n_("Emission"), vec![channel(Channel::Emissive)]),
        (n_("Thin film"), vec![missing(n_("Thin film weight")), missing(n_("Thin film thickness"))]),
        (
            n_("Geometry"),
            vec![channel(Channel::Opacity), channel(Channel::Height), channel(Channel::Normal), missing(n_("Tangent")), channel(Channel::AmbientOcclusion)],
        ),
        (n_("Other"), vec![channel(Channel::Glossiness), channel(Channel::Specular)]),
    ]
});

/// Mesh maps, under the names the bake panel gives them.
pub static MESH_MAPS: LazyLock<Vec<MapEntry>> =
    LazyLock::new(|| BakedMap::ALL.iter().map(|&map| (crate::i18n::baked_map_label(map), Some(SourceRef::Baked { map, component: 0 }))).collect());

/// Converted maps: the maps TextureCraft computes from others when it exports.
pub static CONVERTED_MAPS: LazyLock<Vec<MapEntry>> =
    LazyLock::new(|| VirtualMap::ALL.iter().map(|&map| (map.label(), Some(SourceRef::Virtual { map, component: 0 }))).collect());

/// The listed name of `source` (any component), or None for a constant.
pub fn name_of(source: &SourceRef) -> Option<&'static str> {
    let lists = INPUT_MAPS.iter().flat_map(|(_, entries)| entries.iter()).chain(MESH_MAPS.iter()).chain(CONVERTED_MAPS.iter());
    let mut entries = lists.filter_map(|(name, entry)| entry.as_ref().map(|entry| (*name, entry)));
    entries.find(|(_, entry)| super::same_source(entry, source)).map(|(name, _)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_channel_and_map_can_be_dragged() {
        let entries = INPUT_MAPS.iter().flat_map(|(_, entries)| entries.iter()).chain(MESH_MAPS.iter()).chain(CONVERTED_MAPS.iter());
        let listed: Vec<SourceRef> = entries.filter_map(|(_, source)| source.clone()).collect();

        for channel in Channel::ALL {
            assert!(listed.contains(&SourceRef::Channel { channel, component: 0 }), "{channel:?}");
        }
        for map in BakedMap::ALL {
            assert!(listed.contains(&SourceRef::Baked { map, component: 0 }), "{map:?}");
        }
        for map in VirtualMap::ALL {
            assert!(listed.contains(&SourceRef::Virtual { map, component: 0 }), "{map:?}");
        }
    }
}
