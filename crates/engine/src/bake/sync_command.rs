//! Sharing bake settings between texture sets: live sync groups, and one-off copies from the bake menus

use serde_json::{Value, json};
use texturecraft_core::{BakeSettings, BakeSyncGroup, BakedMap};

use super::mesh_map_command::sets_param;
use crate::commands::{CommandSpec, always, bad, strp};
use crate::{Result, Session, cmd};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "bake.sync",
            "Sync bake settings",
            [],
            None,
            "{group: \"common\"|map id, ts: index|[indices]|\"all\"} → {group, ts}; exactly these texture sets share the group's settings from now on, and the ones added take the settings the group shares",
            always,
            sync
        ),
        cmd!(
            "bake.copy",
            "Copy bake settings",
            [],
            None,
            "{what: \"settings\"|\"checked\"|\"check\", map?: id, from?: index (default: the active set), ts: index|[indices]|\"all\"} → {ts}; settings: every setting and which maps are checked, or with map that map's settings; checked: which maps are checked; check: check map. Settings also reach the sets sharing a group with a target, whose checkboxes stay their own",
            always,
            copy
        ),
    ]
}

fn group_param(params: &Value, command: &str) -> Result<BakeSyncGroup> {
    match strp(params, "group") {
        Some("common") => Ok(BakeSyncGroup::Common),
        Some(id) => BakedMap::parse(id).map(BakeSyncGroup::Map).ok_or_else(|| bad(command, format!("unknown group {id}"))),
        None => Err(bad(command, "`group` must be \"common\" or a mesh map id")),
    }
}

fn group_id(group: BakeSyncGroup) -> &'static str {
    match group {
        BakeSyncGroup::Common => "common",
        BakeSyncGroup::Map(map) => map.id(),
    }
}

fn sync(session: &mut Session, params: &Value) -> Result<Value> {
    const COMMAND: &str = "bake.sync";
    let group = group_param(params, COMMAND)?;
    let members = sets_param(session, COMMAND, params)?;
    let sets = &session.doc.project.texture_sets;

    let source = session.doc.project.bake_sync_source(group, &members, session.ts());
    let source = source.and_then(|index| sets.get(index)).map(|set| set.bake.clone());

    let next: Vec<(usize, BakeSettings)> = sets
        .iter()
        .enumerate()
        .map(|(index, set)| {
            let mut settings = set.bake.clone();
            let joins = members.contains(&index);
            if let Some(source) = source.as_ref().filter(|_| joins && !settings.sync.shares(group)) {
                settings.copy_group(source, group);
            }
            settings.sync.set_shares(group, joins);
            (index, settings)
        })
        .collect();
    store(session, "Sync bake settings", next)?;
    Ok(json!({ "group": group_id(group), "ts": members }))
}

fn copy(session: &mut Session, params: &Value) -> Result<Value> {
    const COMMAND: &str = "bake.copy";

    let map = match params.get("map") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_str().and_then(BakedMap::parse).ok_or_else(|| bad(COMMAND, format!("invalid mesh map {value}")))?),
    };
    let from = match params.get("from") {
        None | Some(Value::Null) => session.ts(),
        Some(value) => value.as_u64().and_then(|index| usize::try_from(index).ok()).ok_or_else(|| bad(COMMAND, "`from` must be a texture set index"))?,
    };
    let source = session.doc.project.texture_set(from)?.bake.clone();
    let targets = sets_param(session, COMMAND, params)?;
    let maps: Vec<BakedMap> = map.map_or_else(|| BakedMap::ALL.to_vec(), |map| vec![map]);

    let mut next: Vec<BakeSettings> = session.doc.project.texture_sets.iter().map(|set| set.bake.clone()).collect();
    let mut explicit = vec![false; next.len()];
    for &index in &targets {
        explicit[index] = true;
    }

    match strp(params, "what") {
        Some("settings") => {
            let groups = map.map_or_else(
                || std::iter::once(BakeSyncGroup::Common).chain(maps.iter().map(|map| BakeSyncGroup::Map(*map))).collect(),
                |map| vec![BakeSyncGroup::Map(map)],
            );
            for group in groups {
                let shared = targets.iter().any(|&index| next[index].sync.shares(group));
                for (index, settings) in next.iter_mut().enumerate() {
                    if explicit[index] || (shared && settings.sync.shares(group)) {
                        settings.copy_group(&source, group);
                    }
                }
            }
            if map.is_none() {
                for (_, settings) in next.iter_mut().enumerate().filter(|(index, _)| explicit[*index]) {
                    for map in BakedMap::ALL {
                        settings.maps.set_enabled(map, source.maps.is_enabled(map));
                    }
                }
            }
        }
        Some(what @ ("checked" | "check")) => {
            if what == "check" && map.is_none() {
                return Err(bad(COMMAND, "`check` needs a `map`"));
            }

            for (_, settings) in next.iter_mut().enumerate().filter(|(index, _)| explicit[*index]) {
                for map in &maps {
                    settings.maps.set_enabled(*map, what == "check" || source.maps.is_enabled(*map));
                }
            }
        }
        _ => return Err(bad(COMMAND, "`what` must be settings, checked or check")),
    }

    store(session, "Copy bake settings", next.into_iter().enumerate().collect())?;
    Ok(json!({ "ts": targets }))
}

/// Write each set's `next` bake settings as one undo step labelled `label`; nothing when none changed
pub(super) fn store(session: &mut Session, label: &str, next: Vec<(usize, BakeSettings)>) -> Result<()> {
    let changed =
        |session: &Session| next.iter().any(|(index, settings)| session.doc.project.texture_sets.get(*index).is_some_and(|set| set.bake != *settings));
    if !changed(session) {
        return Ok(());
    }

    session.edit(label, |session| {
        for (index, settings) in next {
            session.doc.project.texture_set_mut(index)?.bake = settings;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{execute, sample_session};
    use texturecraft_core::BakeSettings;

    /// A dilation only presets may hold, above what an edit may set
    const LEGACY_DILATION: u32 = 200;

    fn session() -> Session {
        let session = sample_session(128, 32).unwrap();
        assert_eq!(session.set_count(), 2);
        session
    }

    fn bake(session: &Session, index: usize) -> &BakeSettings {
        &session.doc.project.texture_sets[index].bake
    }

    #[test]
    fn projects_saved_before_sync_share_everything() {
        let settings: BakeSettings = serde_json::from_value(json!({ "common": { "dilation": 4 } })).unwrap();
        assert!(settings.sync.common);
        assert!(BakedMap::ALL.iter().all(|map| settings.sync.shares(BakeSyncGroup::Map(*map))));
    }

    #[test]
    fn a_set_left_out_of_common_sync_keeps_its_own_common_settings() {
        let mut session = session();
        assert_eq!(session.doc.project.bake_synced_sets(0, BakeSyncGroup::Common), [0, 1]);

        execute(&mut session, "bake.sync", &json!({ "group": "common", "ts": [0] })).unwrap();

        assert_eq!(session.doc.project.bake_synced_sets(0, BakeSyncGroup::Common), [0]);
        assert_eq!(session.doc.project.bake_synced_sets(1, BakeSyncGroup::Common), [1]);
        assert_eq!(session.doc.project.bake_synced_sets(1, BakeSyncGroup::Map(BakedMap::Normal)), [0, 1], "map sync is separate");

        let reached = session.doc.project.bake_synced_sets(0, BakeSyncGroup::Common);
        execute(&mut session, "bake.settings", &json!({ "ts": reached, "patch": { "common": { "dilation": 7 } } })).unwrap();

        assert_eq!((bake(&session, 0).common.dilation, bake(&session, 1).common.dilation), (7, 32));
    }

    #[test]
    fn a_set_joining_a_group_takes_its_settings_and_keeps_its_own_inclusion() {
        let mut session = session();
        execute(&mut session, "bake.sync", &json!({ "group": "ambientOcclusion", "ts": [0] })).unwrap();
        session.doc.project.texture_sets[1].bake.maps.ao.parameters.rays.rays = 3;
        session.doc.project.texture_sets[1].bake.common.is_enabled = false;
        execute(&mut session, "bake.sync", &json!({ "group": "common", "ts": [0] })).unwrap();
        session.doc.project.texture_sets[1].bake.common.dilation = 5;

        execute(&mut session, "bake.sync", &json!({ "group": "ambientOcclusion", "ts": "all" })).unwrap();
        execute(&mut session, "bake.sync", &json!({ "group": "common", "ts": "all" })).unwrap();

        assert_eq!(bake(&session, 1).maps.ao.parameters, bake(&session, 0).maps.ao.parameters);
        assert_eq!(bake(&session, 1).common.dilation, bake(&session, 0).common.dilation);
        assert!(!bake(&session, 1).common.is_enabled, "whether the set bakes is its own");

        session.undo().unwrap();

        assert_eq!(bake(&session, 1).common.dilation, 5, "one undo per sync");
    }

    #[test]
    fn copies_check_maps_and_carry_settings_without_linking() {
        let mut session = session();
        execute(&mut session, "bake.sync", &json!({ "group": "height", "ts": [0] })).unwrap();
        session.doc.project.texture_sets[0].bake.maps.height.parameters.scaling_divisor = 9.0;

        execute(&mut session, "bake.copy", &json!({ "what": "check", "map": "height", "ts": "all" })).unwrap();

        assert!(bake(&session, 0).maps.height.is_enabled && bake(&session, 1).maps.height.is_enabled);

        execute(&mut session, "bake.copy", &json!({ "what": "settings", "map": "height", "from": 0, "ts": [1] })).unwrap();

        assert_eq!(bake(&session, 1).maps.height.parameters.scaling_divisor, 9.0);
        assert!(!bake(&session, 1).sync.shares(BakeSyncGroup::Map(BakedMap::Height)), "a copy does not link");

        session.doc.project.texture_sets[0].bake.maps.select([BakedMap::Normal]);
        execute(&mut session, "bake.copy", &json!({ "what": "checked", "from": 0, "ts": [1] })).unwrap();

        assert_eq!(bake(&session, 1).maps.enabled().collect::<Vec<_>>(), [BakedMap::Normal]);
    }

    #[test]
    fn a_copy_reaches_the_sets_sharing_the_group_with_its_target() {
        let mut session = session();
        let third = session.doc.project.texture_sets[1].clone();
        session.doc.project.texture_sets.push(third);
        execute(&mut session, "bake.sync", &json!({ "group": "ambientOcclusion", "ts": [0, 1] })).unwrap();
        session.doc.project.texture_sets[2].bake.maps.ao.parameters.rays.rays = 3;
        let height_checked = !bake(&session, 0).maps.is_enabled(BakedMap::Height);
        session.doc.project.texture_sets[2].bake.maps.set_enabled(BakedMap::Height, height_checked);

        execute(&mut session, "bake.copy", &json!({ "what": "settings", "from": 2, "ts": [0] })).unwrap();

        assert_eq!((bake(&session, 0).maps.ao.parameters.rays.rays, bake(&session, 1).maps.ao.parameters.rays.rays), (3, 3), "set 1 shares AO with set 0");
        assert!(!bake(&session, 2).sync.shares(BakeSyncGroup::Map(BakedMap::AmbientOcclusion)));
        assert_eq!(bake(&session, 0).maps.is_enabled(BakedMap::Height), height_checked, "every setting includes which maps are checked");
        assert_ne!(bake(&session, 1).maps.is_enabled(BakedMap::Height), height_checked, "the set sharing a group keeps its own checkboxes");
    }

    #[test]
    fn loading_a_preset_on_one_set_keeps_its_sync_groups_and_reaches_the_sets_sharing_them() {
        let mut session = session();
        execute(&mut session, "bake.sync", &json!({ "group": "ambientOcclusion", "ts": [1] })).unwrap();
        let mut preset = bake(&session, 0).clone();
        preset.common.dilation = 9;
        preset.maps.ao.parameters.rays.rays = 5;
        preset.sync = Default::default();

        let params = json!({ "ts": 0, "model": preset, "preserveEnabled": true, "preset": true });
        execute(&mut session, "bake.settings", &params).unwrap();

        assert!(!bake(&session, 0).sync.shares(BakeSyncGroup::Map(BakedMap::AmbientOcclusion)), "a preset is settings, not membership");
        assert_eq!(bake(&session, 1).common.dilation, 9, "set 1 shares Common Settings with set 0");
        assert_ne!(bake(&session, 1).maps.ao.parameters.rays.rays, 5, "set 1 no longer shares AO with set 0");
    }

    #[test]
    fn a_change_to_one_group_leaves_the_other_groups_of_the_sets_sharing_it_alone() {
        let mut session = session();
        session.doc.project.texture_sets[1].bake.common.dilation = 3;

        execute(&mut session, "bake.settings", &json!({ "ts": 0, "patch": { "maps": { "ao": { "parameters": { "rays": { "rays": 6 } } } } } })).unwrap();

        assert_eq!(bake(&session, 1).maps.ao.parameters.rays.rays, 6);
        assert_eq!(bake(&session, 1).common.dilation, 3);
    }

    #[test]
    fn an_edit_that_would_leave_a_synced_set_with_an_invalid_interval_changes_no_set() {
        let mut session = session();
        session.doc.project.texture_sets[0].bake.maps.ao.parameters.rays.max_distance = 0.5;
        session.doc.project.texture_sets[1].bake.maps.ao.parameters.rays.max_distance = 0.05;
        let before = session.doc.project.texture_sets.clone();

        let patch = json!({ "ts": 0, "patch": { "maps": { "ao": { "parameters": { "rays": { "min_distance": 0.1 } } } } } });
        assert!(execute(&mut session, "bake.settings", &patch).is_err(), "set 1 would get a minimum above its maximum");
        assert_eq!(session.doc.project.texture_sets, before);
    }

    #[test]
    fn a_preset_with_a_legacy_value_loads_on_a_set_sharing_it() {
        let mut session = session();
        let mut preset = bake(&session, 0).clone();
        preset.common.dilation = LEGACY_DILATION;

        execute(&mut session, "bake.settings", &json!({ "ts": 0, "model": preset, "preset": true })).unwrap();

        assert_eq!(bake(&session, 1).common.dilation, LEGACY_DILATION, "set 1 shares Common Settings with set 0");
    }

    #[test]
    fn a_settings_patch_cannot_change_sync_membership() {
        let mut session = session();
        let original_sync = bake(&session, 0).sync.clone();

        let result = execute(&mut session, "bake.settings", &json!({ "ts": 0, "patch": { "sync": { "common": false }, "common": { "dilation": 7 } } }));

        assert!(result.is_err(), "membership changes must go through bake.sync");
        assert_eq!(bake(&session, 0).sync, original_sync);
        assert_eq!(bake(&session, 0).common.dilation, bake(&session, 1).common.dilation);
    }

    #[test]
    fn invalid_copy_selectors_do_not_overwrite_other_settings() {
        let mut session = session();
        execute(&mut session, "bake.sync", &json!({ "group": "common", "ts": [0] })).unwrap();
        execute(&mut session, "bake.settings", &json!({ "ts": 1, "dilation": 7 })).unwrap();
        let before = session.doc.project.texture_sets.clone();

        for params in [json!({ "what": "settings", "ts": 1, "from": "invalid" }), json!({ "what": "settings", "ts": 1, "map": false })] {
            let result = execute(&mut session, "bake.copy", &params);

            assert!(result.is_err(), "invalid selector in {params}");
            assert_eq!(session.doc.project.texture_sets, before);
        }
    }

    #[test]
    fn running_a_bake_shares_settings_without_baking_or_checking_maps_on_partners() {
        let mut session = session();
        let previous_resolution = bake(&session, 0).common.resolution;
        let partner_maps = bake(&session, 1).maps.clone();
        session.doc.baked.clear();

        let result = execute(&mut session, "bake.run", &json!({ "ts": 0, "resolution": 64, "maps": ["opacity"], "cpu": true })).unwrap();

        assert_eq!(result["baked"], json!([0]));
        assert_ne!(bake(&session, 0).common.resolution, previous_resolution);
        assert_eq!(bake(&session, 1).common.resolution, bake(&session, 0).common.resolution);
        assert_eq!(bake(&session, 1).maps, partner_maps);
        assert!(!session.doc.baked.contains_key(&1));

        session.undo().unwrap();

        assert_eq!(bake(&session, 0).common.resolution, previous_resolution);
        assert_eq!(bake(&session, 1).common.resolution, previous_resolution);
        assert!(session.doc.baked.is_empty());
    }
}
