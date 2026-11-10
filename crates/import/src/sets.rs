//! [`SetBinding`]: which mesh material slot, document material record and texture-cache folder make up each texture
//! set.
//!
//! The link is the material uid: the mesh stores it per material, the document as the set's scene material uid, and
//! the texture cache uses it as folder name. Names cannot be used, as texture sets can be renamed (the mesh slot
//! `Material.001` may be the set `CERAMIC`). Several mesh slots can share one uid (collapsed materials); they become one
//! texture set. Documents also keep materials of meshes since replaced; those have no slot and are reported as unused.

use std::collections::BTreeSet;

use texturecraft_import_reader::decode::assbin::MaterialSummary;
use texturecraft_import_reader::decode::document::MaterialRecord;

/// One texture set to build.
#[derive(Debug, Clone, PartialEq)]
pub struct SetSource {
    /// The document's record; a stand-in named after the mesh material when the document has none.
    pub record: MaterialRecord,
    /// True when `record` came from the document.
    pub in_document: bool,
    /// First mesh material slot painted by this set.
    pub slot: usize,
    /// Further mesh slots collapsed into this set.
    pub merged_slots: Vec<usize>,
    /// Folder name under the texture cache, if the set has rendered channels.
    pub cache_set: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SetBinding {
    pub sets: Vec<SetSource>,
    /// Document materials no mesh slot uses.
    pub unused: Vec<MaterialRecord>,
}

impl SetBinding {
    pub fn bind(mesh: &[MaterialSummary], records: &[MaterialRecord], cache_sets: &BTreeSet<String>) -> SetBinding {
        let mut binding = SetBinding::default();
        let mut used = vec![false; records.len()];
        for (slot, material) in mesh.iter().enumerate() {
            let found = find_record(material, records);
            if let Some(i) = found {
                if used.get(i).copied().unwrap_or(false)
                    && let Some(set) = binding.sets.iter_mut().find(|s| Some(&s.record) == records.get(i))
                {
                    set.merged_slots.push(slot);
                    continue;
                }
                if let Some(u) = used.get_mut(i) {
                    *u = true;
                }
            }
            let (record, in_document) = match found.and_then(|i| records.get(i)) {
                Some(r) => (r.clone(), true),
                None => (stand_in(material), false),
            };
            let cache_set = cache_folder(&record, material, mesh.len(), cache_sets);
            binding.sets.push(SetSource { record, in_document, slot, merged_slots: Vec::new(), cache_set });
        }
        binding.unused = records.iter().zip(&used).filter(|(_, u)| !**u).map(|(r, _)| r.clone()).collect();
        binding
    }
}

/// By uid; by name only for meshes without uids.
fn find_record(material: &MaterialSummary, records: &[MaterialRecord]) -> Option<usize> {
    match material.uid {
        Some(uid) => records.iter().position(|r| r.scene_uid == Some(uid)),
        None => records.iter().position(|r| r.name == material.name),
    }
}

fn stand_in(material: &MaterialSummary) -> MaterialRecord {
    MaterialRecord { name: material.name.clone(), scene_uid: material.uid, ..MaterialRecord::default() }
}

/// The cache folder of the set's uid; a lone cache folder of a single-slot mesh is taken as is.
fn cache_folder(record: &MaterialRecord, material: &MaterialSummary, slots: usize, cache: &BTreeSet<String>) -> Option<String> {
    match record.scene_uid.or(material.uid).map(|u| u.to_string()) {
        Some(uid) if cache.contains(&uid) => Some(uid),
        _ if slots == 1 && cache.len() == 1 => cache.iter().next().cloned(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material(name: &str, uid: Option<u64>) -> MaterialSummary {
        MaterialSummary { name: name.into(), uid }
    }

    fn record(name: &str, uid: u64) -> MaterialRecord {
        MaterialRecord { name: name.into(), scene_uid: Some(uid), ..MaterialRecord::default() }
    }

    #[test]
    fn sets_bind_by_uid_and_collapse_shared_uids() {
        let mesh = [material("Material.001", Some(7)), material("Material.002", Some(7)), material("Other", Some(9))];
        let records = [record("CERAMIC", 7), record("Stale", 3)];
        let cache: BTreeSet<String> = ["7".to_string()].into();
        let b = SetBinding::bind(&mesh, &records, &cache);
        assert_eq!(b.sets.len(), 2);
        assert_eq!((b.sets[0].record.name.as_str(), b.sets[0].merged_slots.as_slice(), b.sets[0].cache_set.as_deref()), ("CERAMIC", &[1][..], Some("7")));
        assert!(!b.sets[1].in_document && b.sets[1].record.name == "Other" && b.sets[1].cache_set.is_none());
        assert_eq!(b.unused.len(), 1);
        // a single slot takes the lone cache folder whatever its name
        let lone = SetBinding::bind(&[material("m", None)], &[], &["42".to_string()].into());
        assert_eq!(lone.sets[0].cache_set.as_deref(), Some("42"));
    }
}
