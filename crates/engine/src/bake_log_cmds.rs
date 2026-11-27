//! The `bake.log` query: the Baking Log's rows for automation

use serde_json::{Value, json};

use crate::commands::{CommandSpec, always, bad};
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "bake.log",
        "Baking log",
        [],
        None,
        "{} → {running, maps: [{textureSet, setName, map, state: \"waiting\" | \"baking\" | \"baked\" | \"failed\" | \"canceled\", reason?}], highPolys: [{textureSet, setName, file, state: \"waiting\" | \"loading\" | \"loaded\" | \"failed\" | \"canceled\", reason?}], matching: null | {lowSuffix, highSuffix, pairs: [{low: name | null, high: name | null}]}}. The latest bake map by map in bake order, the high poly files each set bakes from, and how parts paired by name, as the Baking Log shows them; `reason` says why a map or file failed; a `null` side of a pair is a part with no partner; `matching` is null when no set matched by mesh name. Empty before the first bake",
        always,
        log
    )]
}

fn log(session: &mut Session, _: &Value) -> Result<Value> {
    let log = &session.bake_log;
    let maps = log.maps.iter().map(|row| entry(&row.state, row.texture_set, &row.set_name, "map", json!(row.map.id()))).collect::<Result<Vec<Value>>>()?;
    let high_polys = log.highs.iter().map(|row| entry(&row.state, row.texture_set, &row.set_name, "file", json!(row.file))).collect::<Result<Vec<Value>>>()?;
    let matching = log.matching.as_ref().map(|matching| {
        let pairs: Vec<Value> = matching.pairs.iter().map(|pair| json!({ "low": pair.low, "high": pair.high })).collect();
        json!({ "lowSuffix": matching.low_suffix, "highSuffix": matching.high_suffix, "pairs": pairs })
    });

    Ok(json!({ "running": log.is_running() || log.is_loading_high_polys(), "maps": maps, "highPolys": high_polys, "matching": matching }))
}

/// One row: its `state` (and `reason`), its texture set, and an item identified by `key`
fn entry(state: &impl serde::Serialize, texture_set: usize, set_name: &str, key: &str, value: Value) -> Result<Value> {
    let mut entry = serde_json::to_value(state).map_err(|error| bad("bake.log", error.to_string()))?;

    if let Some(fields) = entry.as_object_mut() {
        fields.insert("textureSet".into(), json!(texture_set));
        fields.insert("setName".into(), json!(set_name));
        fields.insert(key.into(), value);
    }

    Ok(entry)
}
