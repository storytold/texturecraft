use super::{Candidate, MapKind};
use std::collections::BTreeMap;

/// The maps found among a list of files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PbrSet {
    /// The best file per kind.
    pub chosen: BTreeMap<MapKind, Candidate>,
    /// Every file that names each kind, best first (the chosen one is first).
    pub candidates: BTreeMap<MapKind, Vec<Candidate>>,
    /// Files not used, with why (`not an image`, `preview image`, `unknown map name`,
    /// `unused map`, `another file is used for <kind>`).
    pub ignored: Vec<(String, String)>,
}

impl PbrSet {
    /// True when no map was recognised.
    pub fn is_empty(&self) -> bool {
        self.chosen.is_empty()
    }
}
