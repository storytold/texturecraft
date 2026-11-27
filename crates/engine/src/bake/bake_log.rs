use std::collections::HashSet;

use texturecraft_bake::NamePair;
use texturecraft_core::{BakedMap, MeshMatchMode, Project};

use super::{BakeHighState, BakeLogHigh, BakeLogMap, BakeLogMatching, BakeMapState, BakeMesh};
use crate::Session;

/// The latest bake map by map for the Baking Log, kept from the bake's own state, never from Log lines
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakeLog {
    pub maps: Vec<BakeLogMap>,
    /// Each high poly file of each texture set that bakes from the high polys, in bake order
    pub highs: Vec<BakeLogHigh>,
    /// How the parts paired by name; `None` when no set of the bake matches "By Mesh Name"
    pub matching: Option<BakeLogMatching>,
}

impl BakeLog {
    /// Start over for a bake of `sets`: every checked map of each set and the `high_polys` of each set that bakes from them, waiting, in bake order
    pub fn begin(&mut self, project: &Project, high_polys: &[BakeMesh], sets: &[usize]) {
        self.maps = sets
            .iter()
            .filter_map(|index| project.texture_sets.get(*index).map(|set| (*index, set)))
            .flat_map(|(index, set)| {
                set.bake.maps.enabled().map(move |map| BakeLogMap { texture_set: index, set_name: set.name.clone(), map, state: BakeMapState::Waiting })
            })
            .collect();
        self.highs = sets
            .iter()
            .filter_map(|index| project.texture_sets.get(*index).map(|set| (*index, set)))
            .filter(|(_, set)| !set.bake.high_poly.use_low_as_high)
            .flat_map(|(index, set)| {
                high_polys.iter().map(move |high| BakeLogHigh {
                    texture_set: index,
                    set_name: set.name.clone(),
                    file: high.name.clone(),
                    state: BakeHighState::Waiting,
                })
            })
            .collect();
        self.matching = None;
    }

    /// `set` starts baking: its high polys load and its first map is the one in progress
    pub fn set_started(&mut self, set: usize) {
        for row in self.highs.iter_mut().filter(|row| row.texture_set == set && row.state == BakeHighState::Waiting) {
            row.state = BakeHighState::Loading;
        }
        self.start_next(set);
    }

    /// `set`'s bake has its high polys ready
    pub fn high_polys_loaded(&mut self, set: usize) {
        for row in self.highs.iter_mut().filter(|row| row.texture_set == set && row.state == BakeHighState::Loading) {
            row.state = BakeHighState::Loaded;
        }
    }

    /// `map` of `set` is baked; the set's next map starts
    pub fn map_baked(&mut self, set: usize, map: BakedMap) {
        if let Some(row) = self.maps.iter_mut().find(|row| row.texture_set == set && row.map == map) {
            row.state = BakeMapState::Baked;
        }
        self.start_next(set);
    }

    /// `set`'s bake was kept: a map it did not hand over is not baked
    pub fn set_finished(&mut self, set: usize) {
        for row in self.maps.iter_mut().filter(|row| row.texture_set == set && row.state.is_pending()) {
            row.state = BakeMapState::Canceled;
        }
    }

    /// Nothing of `set` was kept: every one of its maps failed for `reason`
    pub fn set_failed(&mut self, set: usize, reason: &str) {
        for row in self.maps.iter_mut().filter(|row| row.texture_set == set) {
            row.state = BakeMapState::Failed(reason.to_owned());
        }

        // A file that loaded is not what failed
        for row in self.highs.iter_mut().filter(|row| row.texture_set == set && row.state.is_pending()) {
            row.state = BakeHighState::Failed(reason.to_owned());
        }
    }

    /// The bake stopped: every map still waiting or baking is canceled, and so are the finished maps of `discarded` (the set being baked, when they were not kept)
    pub fn cancel(&mut self, discarded: Option<usize>) {
        for row in &mut self.maps {
            if row.state.is_pending() || (Some(row.texture_set) == discarded && row.state == BakeMapState::Baked) {
                row.state = BakeMapState::Canceled;
            }
        }
        self.cancel_pending_highs();
    }

    /// The whole bake was rolled back: every map that did not fail is canceled
    pub fn discard(&mut self) {
        for row in self.maps.iter_mut().filter(|row| !matches!(row.state, BakeMapState::Failed(_))) {
            row.state = BakeMapState::Canceled;
        }
        self.cancel_pending_highs();
    }

    /// Whether a map is still waiting or baking
    pub fn is_running(&self) -> bool {
        self.maps.iter().any(|row| row.state.is_pending())
    }

    /// Whether a high poly file is still waiting or loading
    pub fn is_loading_high_polys(&self) -> bool {
        self.highs.iter().any(|row| row.state.is_pending())
    }

    fn cancel_pending_highs(&mut self) {
        for row in self.highs.iter_mut().filter(|row| row.state.is_pending()) {
            row.state = BakeHighState::Canceled;
        }
    }

    fn start_next(&mut self, set: usize) {
        if self.maps.iter().any(|row| row.texture_set == set && row.state == BakeMapState::Baking) {
            return;
        }

        if let Some(row) = self.maps.iter_mut().find(|row| row.texture_set == set && row.state == BakeMapState::Waiting) {
            row.state = BakeMapState::Baking;
        }
    }
}

impl Session {
    /// Start the Baking Log over for a bake of `sets` ([`BakeLog::begin`]), with how their parts pair by name
    pub fn begin_bake_log(&mut self, sets: &[usize]) {
        let matching = self.bake_log_matching(sets);
        self.bake_log.begin(&self.doc.project, &self.doc.bake_meshes.high, sets);
        self.bake_log.matching = matching;
    }

    /// Start the Baking Log for a bake of only `maps` of each of `sets`, checked or not
    pub fn begin_map_bake_log(&mut self, sets: &[usize], maps: &[BakedMap]) {
        self.begin_bake_log(sets);
        let project = &self.doc.project;
        self.bake_log.maps = sets
            .iter()
            .filter_map(|&texture_set| Some((texture_set, project.texture_sets.get(texture_set)?.name.clone())))
            .flat_map(|(texture_set, set_name)| {
                maps.iter().map(move |&map| BakeLogMap { texture_set, set_name: set_name.clone(), map, state: BakeMapState::Waiting })
            })
            .collect();
    }

    /// The name pairs of each of `sets` that matches "By Mesh Name"; a high poly part counts as unpaired only when none of them pairs with it and no set matching "Always" bakes from every part
    fn bake_log_matching(&self, sets: &[usize]) -> Option<BakeLogMatching> {
        let mut matching: Option<BakeLogMatching> = None;
        let (mut paired_highs, mut unpaired_highs) = (HashSet::new(), Vec::new());
        let mut every_high_used = false;
        for &set in sets {
            let Ok(texture_set) = self.doc.project.texture_set(set) else { continue };

            let settings = &texture_set.bake.high_poly;
            every_high_used |= !settings.use_low_as_high && settings.match_mode == MeshMatchMode::Always;
            let Ok(Some(pairs)) = self.bake_name_pairs(set) else { continue };

            let matching = matching.get_or_insert_with(|| BakeLogMatching {
                low_suffix: settings.low_suffix.clone(),
                high_suffix: settings.high_suffix.clone(),
                pairs: Vec::new(),
            });
            for pair in pairs {
                match (&pair.low, &pair.high) {
                    (None, Some(high)) if !unpaired_highs.contains(high) => unpaired_highs.push(high.clone()),
                    (None, _) => {}
                    (Some(_), high) => {
                        paired_highs.extend(high.clone());
                        matching.pairs.push(pair);
                    }
                }
            }
        }

        if let Some(matching) = matching.as_mut().filter(|_| !every_high_used) {
            let unpaired = unpaired_highs.into_iter().filter(|high| !paired_highs.contains(high));
            matching.pairs.extend(unpaired.map(|high| NamePair { low: None, high: Some(high) }));
        }
        matching
    }
}
