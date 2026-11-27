//! Auto-rebake: the one mesh map per texture set that is baked again whenever what it is baked from changes

use serde_json::{Value, json};
use texturecraft_core::{BakeSource, BakedMap};

use crate::commands::{CommandSpec, always, bad, strp, u64p};
use crate::{Result, Session, cmd};

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "bake.autoRebake",
        "Auto-rebake",
        [],
        None,
        "{ts?: index (default: the active set), map: id | null} → {ts, autoRebake: id | null}. Turn on Auto-rebake for one mesh map of the set (it replaces the set's previous one), or turn it off with null. The app then rebakes that map in the background once what it is baked from changes",
        always,
        set_auto_rebake
    )]
}

fn set_auto_rebake(session: &mut Session, params: &Value) -> Result<Value> {
    const COMMAND: &str = "bake.autoRebake";
    let texture_set = match params.get("ts") {
        None => session.ts(),
        Some(_) => u64p(params, "ts")
            .and_then(|index| usize::try_from(index).ok())
            .filter(|index| *index < session.set_count())
            .ok_or_else(|| bad(COMMAND, "invalid `ts`"))?,
    };
    let map = match params.get("map") {
        Some(Value::Null) => None,
        Some(_) => Some(strp(params, "map").and_then(BakedMap::parse).ok_or_else(|| bad(COMMAND, "`map` must be a mesh map id or null"))?),
        None => return Err(bad(COMMAND, "`map` is required (a mesh map id or null)")),
    };

    if session.doc.project.texture_set(texture_set)?.auto_rebake != map {
        session.edit("Auto-rebake", |session| {
            session.doc.project.texture_set_mut(texture_set)?.auto_rebake = map;
            Ok(())
        })?;
    }

    Ok(json!({ "ts": texture_set, "autoRebake": map.map(BakedMap::id) }))
}

impl Session {
    /// The auto-rebake maps that are out of date now, as (texture set, map, what a bake would read now)
    pub fn auto_rebake_due(&self) -> Vec<(usize, BakedMap, BakeSource)> {
        let sets = self.doc.project.texture_sets.iter().enumerate();
        sets.filter_map(|(index, set)| {
            let map = set.auto_rebake?;
            let record = set.bake_sources.get(&map).filter(|_| set.baked.contains(&map))?;
            let current = self.bake_source(index, map).ok()?;
            (*record != current).then_some((index, map, current))
        })
        .collect()
    }

    /// Bake the selected map alone, or include every checked map if the atlas changed size
    pub fn auto_rebake_maps(&self, texture_set: usize, map: BakedMap) -> Vec<BakedMap> {
        let Ok(set) = self.doc.project.texture_set(texture_set) else { return vec![map] };

        let (tile_width, tile_height) = set.bake.common.output_size();
        let (columns, rows) = set.tile_grid().map_or((1, 1), |grid| (grid.cols, grid.rows));
        let (width, height) = (tile_width * columns, tile_height * rows);
        let stored = self.doc.baked.get(&texture_set);
        let resized = stored.is_some_and(|stored| stored.resolution != width || stored.maps.keys().any(|baked| stored.height(*baked) != height));
        match resized {
            true => BakedMap::ALL.into_iter().filter(|baked| *baked == map || set.bake.maps.is_enabled(*baked)).collect(),
            false => vec![map],
        }
    }
}
