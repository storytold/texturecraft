//! [`ImportReport`]: what a project import carried over exactly, approximated or only baked, kept with the project
//! so agents and tests can read it back (`project.inspect`). There is no report window: the report is project data.
//!
//! Besides the item lines, the report holds one [`LayerReport`] per imported layer: its fidelity, the safety-net
//! flags that explain why it may differ from the original, and its role in the stack (an imported layer, or a layer
//! the import added such as the original's stored result, see [`original_result_layer`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Channel;
use crate::layer::{Layer, LayerId, LayerKind, Lock, LockReason};

/// Name of the layer holding the original file's stored result, shown as the result.
pub const ORIGINAL_RESULT_NAME: &str = "Original result";
/// Name of the layer holding the original file's stored result, kept hidden as a reference above the imported layers.
pub const ORIGINAL_REFERENCE_NAME: &str = "Original result (reference)";
/// Lock reason key of an original result layer (its pixels are the original's, not computed).
pub const ORIGINAL_RESULT_KEY: &str = "original-result";

/// How faithfully one item or layer was carried over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum Fidelity {
    /// The same result as in the original.
    #[default]
    Exact,
    /// Translated to the nearest equivalent; the result may differ.
    Approximated,
    /// Kept only as pixels (a procedural or filter flattened to an image); no longer editable as such.
    Baked,
}

impl Fidelity {
    pub const ALL: [Fidelity; 3] = [Fidelity::Exact, Fidelity::Approximated, Fidelity::Baked];

    pub fn id(self) -> &'static str {
        match self {
            Fidelity::Exact => "exact",
            Fidelity::Approximated => "approximated",
            Fidelity::Baked => "baked",
        }
    }

    pub fn parse(id: &str) -> Option<Fidelity> {
        Fidelity::ALL.into_iter().find(|f| f.id().eq_ignore_ascii_case(id) || format!("{f:?}").eq_ignore_ascii_case(id))
    }
}

/// One line of the report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportEntry {
    /// What the line is about, such as `texture set "Body" / layer stack`.
    pub item: String,
    pub fidelity: Fidelity,
    /// Detail for people; may be empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// What a safety-net flag is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SafetyFlagKind {
    /// A setting of the original the import does not handle, away from its default (`key` names it).
    UnhandledSetting,
    /// A field of the original the reader does not know (`key` names it).
    UnknownSetting,
    /// The original shows the layer but its mask cannot be computed: imported hidden and without that mask.
    MaskNotComputable,
    /// The content cannot be computed: kept as stored pixels or a placeholder, and the layer locked.
    ContentNotComputable,
    /// A value the original does not store had to be guessed (a blend or normal map mode).
    Guessed,
}

/// One reason a layer's result may differ from the original's, found by the import's safety net.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyFlag {
    pub kind: SafetyFlagKind,
    /// A stable key for grouping (a setting's `class.field`, a graph key); may be empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    pub text: String,
}

impl SafetyFlag {
    pub fn new(kind: SafetyFlagKind, key: impl Into<String>, text: impl Into<String>) -> SafetyFlag {
        SafetyFlag { kind, key: key.into(), text: text.into() }
    }
}

/// Where a layer of an imported stack came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum LayerRole {
    /// A layer of the original file.
    #[default]
    Imported,
    /// The original file's stored result as pixels (see [`original_result_layer`]).
    OriginalResult,
    /// The original's result on the texels the imported layers do not reproduce.
    Correction,
    /// The channel values under every layer, added where the imported layers do not cover a channel.
    ChannelDefaults,
}

/// The report of one layer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LayerReport {
    #[serde(default)]
    pub role: LayerRole,
    pub fidelity: Fidelity,
    /// The folder path and name the layer had in the original, such as `Rust / Fill layer 3`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<SafetyFlag>,
}

/// The report of a project import.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ImportReport {
    /// What was imported, such as `robot.ext (saved with version 9.0.0)`.
    pub source: String,
    /// What imported it, such as `texturecraft-import 0.4.0`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub importer: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<ReportEntry>,
    /// Per layer, by id. Layers deleted since the import keep their line until the report is rewritten.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub layers: BTreeMap<LayerId, LayerReport>,
}

impl ImportReport {
    pub fn new(source: impl Into<String>, importer: impl Into<String>) -> ImportReport {
        ImportReport { source: source.into(), importer: importer.into(), ..ImportReport::default() }
    }

    pub fn add(&mut self, item: impl Into<String>, fidelity: Fidelity, note: impl Into<String>) {
        self.entries.push(ReportEntry { item: item.into(), fidelity, note: note.into() });
    }

    /// The report of layer `id`, created as an exact imported layer when missing.
    pub fn layer_mut(&mut self, id: LayerId) -> &mut LayerReport {
        self.layers.entry(id).or_default()
    }

    /// Items and layers with `fidelity`.
    pub fn count(&self, fidelity: Fidelity) -> usize {
        self.entries.iter().filter(|e| e.fidelity == fidelity).count() + self.layers.values().filter(|l| l.fidelity == fidelity).count()
    }

    /// True if every item and layer was carried over exactly.
    pub fn is_exact(&self) -> bool {
        self.entries.iter().all(|e| e.fidelity == Fidelity::Exact) && self.layers.values().all(|l| l.fidelity == Fidelity::Exact)
    }

    /// The layers holding the original's stored result.
    pub fn original_results(&self) -> impl Iterator<Item = LayerId> + '_ {
        self.layers.iter().filter(|(_, l)| l.role == LayerRole::OriginalResult).map(|(id, _)| *id)
    }
}

/// A paint layer for the original file's stored result (its pixels are stored by the caller under the layer id):
/// locked, named [`ORIGINAL_RESULT_NAME`] when `shown`, else hidden and named [`ORIGINAL_REFERENCE_NAME`]. Record it
/// in the report with [`LayerRole::OriginalResult`] and [`Fidelity::Baked`].
pub fn original_result_layer(id: LayerId, channels: &[Channel], shown: bool) -> Layer {
    let name = if shown { ORIGINAL_RESULT_NAME } else { ORIGINAL_REFERENCE_NAME };
    let mut l = Layer::new(id, name, LayerKind::Paint, channels);
    l.visible = shown;
    l.lock = Some(Lock::import(vec![LockReason::new(ORIGINAL_RESULT_KEY, "the original file's stored result")]));
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_cover_items_and_layers() {
        let mut r = ImportReport::new("a.ext (saved with version 1.0)", "texturecraft-import 0.1.0");
        assert!(r.is_exact());
        r.add("texture set \"Body\" / layer stack", Fidelity::Exact, "");
        r.layer_mut(LayerId(4)).fidelity = Fidelity::Baked;
        r.add("texture set \"Body\" / settings", Fidelity::Approximated, "setting x not handled");
        assert_eq!((r.count(Fidelity::Exact), r.count(Fidelity::Approximated), r.count(Fidelity::Baked)), (1, 1, 1));
        assert!(!r.is_exact());
        for f in Fidelity::ALL {
            assert_eq!(Fidelity::parse(f.id()), Some(f));
        }
        assert_eq!(Fidelity::parse("kept_raw"), None, "raw foreign data is never kept");
    }

    #[test]
    fn original_result_layers_are_locked_paint_layers_with_the_neutral_name() {
        let shown = original_result_layer(LayerId(7), &[Channel::BaseColor], true);
        assert_eq!((shown.name.as_str(), shown.visible), (ORIGINAL_RESULT_NAME, true));
        assert!(matches!(shown.kind, LayerKind::Paint) && shown.is_locked());
        assert_eq!(shown.lock.as_ref().map(|l| l.keys()), Some(vec![ORIGINAL_RESULT_KEY]));
        let hidden = original_result_layer(LayerId(8), &[Channel::BaseColor], false);
        assert_eq!((hidden.name.as_str(), hidden.visible), (ORIGINAL_REFERENCE_NAME, false));
        let mut r = ImportReport::default();
        r.layers.insert(LayerId(8), LayerReport { role: LayerRole::OriginalResult, fidelity: Fidelity::Baked, ..LayerReport::default() });
        r.layer_mut(LayerId(9));
        assert_eq!(r.original_results().collect::<Vec<_>>(), [LayerId(8)]);
    }

    #[test]
    fn reports_round_trip_and_tolerate_missing_parts() {
        let mut r = ImportReport::new("a.ext", "");
        r.add("item", Fidelity::Approximated, "note");
        let l = r.layer_mut(LayerId(3));
        l.fidelity = Fidelity::Approximated;
        l.path = "Folder / Layer".into();
        l.flags.push(SafetyFlag::new(SafetyFlagKind::UnhandledSetting, "Class.field", "setting Class.field = 2 not handled"));
        l.flags.push(SafetyFlag::new(SafetyFlagKind::MaskNotComputable, "", "mask generator not computable"));
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<ImportReport>(&json).unwrap(), r);
        let bare: ImportReport = serde_json::from_str(r#"{"source": "b.ext"}"#).unwrap();
        assert_eq!(bare, ImportReport::new("b.ext", ""));
        let layer: LayerReport = serde_json::from_str(r#"{"fidelity": "Baked"}"#).unwrap();
        assert_eq!((layer.role, layer.fidelity), (LayerRole::Imported, Fidelity::Baked));
    }
}
