//! Texture set overrides saved before the reference's output templates arrived: the per-map
//! settings are kept by map index, and several built-in templates changed their map order.

use texturecraft_engine::core::{BakedMap, Channel};
use texturecraft_engine::formats::{SourceRef, VirtualMap, builtin_preset};

use super::ExportUi;

/// The version of the built-in template list that saved export settings refer to.
pub const TEMPLATES_VERSION: u32 = 1;

/// The built-in templates whose maps moved with version 1: (their old name, the first source of
/// each of their maps in the old order).
fn moved_templates() -> Vec<(&'static str, Vec<SourceRef>)> {
    let channel = |channel| SourceRef::Channel { channel, component: 0 };
    let converted = |map| SourceRef::Virtual { map, component: 0 };

    // The old default: every channel but the normal and AO, then those two
    let mut default: Vec<SourceRef> = Channel::ALL.into_iter().filter(|c| !matches!(c, Channel::Normal | Channel::AmbientOcclusion)).map(channel).collect();
    default.extend([channel(Channel::Normal), converted(VirtualMap::AoMixed)]);
    vec![
        ("Document channels + Normal + AO (No Alpha)", default.clone()),
        ("Document channels + Normal + AO", default),
        (
            "Blender (Principled BSDF)",
            vec![
                channel(Channel::BaseColor),
                channel(Channel::Metallic),
                channel(Channel::Roughness),
                converted(VirtualMap::NormalOpenGl),
                channel(Channel::Height),
                channel(Channel::Emissive),
                channel(Channel::SpecularLevel),
            ],
        ),
        ("Mesh maps", BakedMap::ALL.into_iter().map(|map| SourceRef::Baked { map, component: 0 }).collect()),
        (
            "Unity Standard (Metallic Smoothness)",
            vec![channel(Channel::BaseColor), channel(Channel::Metallic), converted(VirtualMap::NormalOpenGl), converted(VirtualMap::AoMixed)],
        ),
    ]
}

impl ExportUi {
    /// Bring settings saved with an older template list up to date: template names take their
    /// current form, and per-map settings follow their map to its new index (by the map's first
    /// source; maps that went away lose theirs).
    pub fn update_saved_templates(&mut self) {
        if self.templates_version >= TEMPLATES_VERSION {
            return;
        }

        let moved = moved_templates();
        for set in self.sets.values_mut() {
            if let Some(template) = set.preset.as_deref().and_then(builtin_preset) {
                set.preset = Some(template.name);
            }
            let Some(template) = builtin_preset(&set.maps_template) else { continue };

            if let Some((_, sources)) = moved.iter().find(|(name, _)| name.eq_ignore_ascii_case(&set.maps_template)) {
                let new_index = |source: &SourceRef| template.maps.iter().position(|map| map.channels.first().is_some_and(|(_, first)| first == source));
                set.maps = std::mem::take(&mut set.maps).into_iter().filter_map(|(index, map)| Some((new_index(sources.get(index)?)?, map))).collect();
            }
            set.maps_template = template.name;
        }
        self.sets.retain(|_, set| !set.is_default());
        self.templates_version = TEMPLATES_VERSION;
    }
}
