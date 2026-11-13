//! How "By Mesh Name" pairs low-poly parts with high-poly parts, listed before a bake runs

use std::collections::{HashMap, HashSet};

use texturecraft_core::HighPolySettings;
use texturecraft_geom::Mesh;

use crate::map::part_key;

/// A low-poly part and the high-poly part its rays may hit under "By Mesh Name"; one side is missing when that part pairs with nothing
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamePair {
    /// The low-poly part's name as in its mesh
    pub low: Option<String>,
    /// The high-poly part's name as in its mesh
    pub high: Option<String>,
}

impl NamePair {
    /// Each part of `low` in `material` with every `high` part it pairs with (a row per pair), then the `high` parts pairing with none; keyed like the bake, low poly first
    pub fn list(low: &Mesh, material: usize, high: &Mesh, settings: &HighPolySettings) -> Vec<NamePair> {
        let backface_suffix = settings.ignore_backface_suffix.trim().to_lowercase();
        let mut keys = HashMap::new();
        let mut keyed = |mesh: &Mesh, suffix: &str, only_material: Option<usize>| {
            let mut seen = HashSet::new();
            let mut parts = Vec::new();
            for part in mesh.submeshes.iter().filter(|part| only_material.is_none_or(|material| part.material == material)) {
                let (key, ..) = part_key(&part.name, suffix, &backface_suffix, &mut keys);
                if seen.insert(part.name.as_str()) {
                    parts.push((key, part.name.clone()));
                }
            }
            parts
        };
        let low_parts = keyed(low, &settings.low_suffix, Some(material));
        let high_parts = keyed(high, &settings.high_suffix, None);

        let mut pairs = Vec::new();
        for (key, low_name) in &low_parts {
            let matches: Vec<&String> = high_parts.iter().filter(|(high_key, _)| high_key == key).map(|(_, name)| name).collect();
            if matches.is_empty() {
                pairs.push(NamePair { low: Some(low_name.clone()), high: None });
            }
            pairs.extend(matches.into_iter().map(|high_name| NamePair { low: Some(low_name.clone()), high: Some(high_name.clone()) }));
        }

        let low_keys: HashSet<u32> = low_parts.iter().map(|(key, _)| *key).collect();
        pairs.extend(high_parts.into_iter().filter(|(key, _)| !low_keys.contains(key)).map(|(_, name)| NamePair { low: None, high: Some(name) }));
        pairs
    }

    /// Whether one side is missing: a part the bake will leave unpaired
    pub fn unpaired(&self) -> bool {
        self.low.is_none() || self.high.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_geom::SubMesh;

    /// A mesh with one empty part per (name, material)
    fn parts(names: &[(&str, usize)]) -> Mesh {
        Mesh {
            submeshes: names.iter().map(|(name, material)| SubMesh { name: (*name).into(), material: *material, start: 0, count: 0 }).collect(),
            materials: vec!["first".into(), "second".into()],
            ..Default::default()
        }
    }

    fn pair(low: Option<&str>, high: Option<&str>) -> NamePair {
        NamePair { low: low.map(Into::into), high: high.map(Into::into) }
    }

    #[test]
    fn parts_pair_by_name_without_their_suffixes() {
        let low = parts(&[("Barrel_low", 0), ("Base_low", 0), ("Slide_low", 1)]);
        let high = parts(&[("Barrel_high", 0), ("Slide_high", 0), ("Grip_high", 0), ("BARREL_high_ignorebf", 0)]);
        let settings = HighPolySettings::default();

        assert_eq!(
            NamePair::list(&low, 0, &high, &settings),
            vec![
                pair(Some("Barrel_low"), Some("Barrel_high")),
                pair(Some("Barrel_low"), Some("BARREL_high_ignorebf")),
                pair(Some("Base_low"), None),
                pair(None, Some("Slide_high")),
                pair(None, Some("Grip_high")),
            ],
            "another material's parts are left out, so its high-poly part counts as unpaired here"
        );
        assert_eq!(NamePair::list(&low, 1, &high, &settings).iter().filter(|pair| !pair.unpaired()).count(), 1);
    }

    #[test]
    fn a_part_split_over_triangle_ranges_is_listed_once() {
        let low = parts(&[("Body_low", 0), ("Body_low", 0)]);
        let high = parts(&[("Body_high", 0), ("Body_high", 0)]);

        assert_eq!(NamePair::list(&low, 0, &high, &HighPolySettings::default()), vec![pair(Some("Body_low"), Some("Body_high"))]);
    }
}
