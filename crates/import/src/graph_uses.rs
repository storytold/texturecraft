//! Every place the file runs a compiled graph, one function per use: procedural fills of content channels, mask
//! generators, filters, brush alphas and material fills.
//!
//! TextureCraft cannot run compiled graphs yet, so each use returns a [`GraphUse`] holding a placeholder effect (it
//! composites as identity) and the blocker `graph:<name>` that locks the layer. When graph sources arrive, each function
//! returns the graph effect or source instead and the callers stay as they are: they place [`GraphUse::effect`] where
//! the use sits, merge [`GraphUse::status`] into the layer's status, and record [`GraphUse::graph`] in the report.
//!
//! A few filters have a close built-in counterpart ([`native_filter`]); they are used only when
//! [`crate::ImportOptions::native_filters`] asks for it.

use texturecraft_core::{EffectKind, Filter};
use texturecraft_import_reader::blockers::Blocker;
use texturecraft_import_reader::model::{FilterEffect, Procedural, ProceduralSource, TweakValue};

use crate::status::Status;

/// Why graph uses are placeholders, for the effect's `reason`.
pub const NOT_COMPUTED: &str = "compiled graphs are not computed yet";

/// One use of a compiled graph in the imported stack.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphUse {
    /// What stands where the graph is used.
    pub effect: EffectKind,
    /// Its status: not computable by the graph's blocker while graphs are placeholders.
    pub status: Status,
    /// The graph's name (`?` when the file names none).
    pub graph: String,
}

fn name_of(p: Option<&Procedural>) -> String {
    p.map_or("?", Procedural::name).to_string()
}

fn placeholder(label: String, graph: String) -> GraphUse {
    let blocker = Blocker::graph(&graph, label.clone());
    GraphUse { effect: EffectKind::Placeholder { label, reason: NOT_COMPUTED.into() }, status: Status::not_computable(blocker), graph }
}

/// A procedural source of a fill in layer content, writing `channels` (their names, for the label).
pub fn procedural_fill(source: &ProceduralSource, channels: &[String]) -> GraphUse {
    let graph = name_of(source.procedural.as_ref());
    let what = if source.mode == 1 { "material fill" } else { "procedural fill" };
    let label = if channels.is_empty() { format!("{what} \"{graph}\"") } else { format!("{what} \"{graph}\" ({})", channels.join(", ")) };
    placeholder(label, graph)
}

/// A procedural source of a fill in a mask (a mask generator).
pub fn mask_generator(source: &ProceduralSource) -> GraphUse {
    let graph = name_of(source.procedural.as_ref());
    placeholder(format!("mask generator \"{graph}\""), graph)
}

/// A filter effect, in content (`in_mask` false) or in a mask.
pub fn filter(f: &FilterEffect, in_mask: bool) -> GraphUse {
    let graph = name_of(f.procedural.as_ref());
    let label = if in_mask { format!("mask filter \"{graph}\"") } else { format!("filter \"{graph}\"") };
    placeholder(label, graph)
}

/// The brush alpha of brush strokes, made by a compiled graph.
pub fn brush_alpha(source: &ProceduralSource) -> GraphUse {
    let graph = name_of(source.procedural.as_ref());
    placeholder(format!("brush alpha \"{graph}\""), graph)
}

/// A graph feeding an image input of another graph or a reference.
pub fn nested(p: &Procedural) -> GraphUse {
    let graph = p.name().to_string();
    placeholder(format!("graph input \"{graph}\""), graph)
}

/// A float tweak of a procedural (its first component).
fn tweak(p: &Procedural, id: &str) -> Option<f32> {
    p.tweaks.iter().find(|t| t.identifier == id).and_then(|t| match &t.value {
        TweakValue::Floats(v) => v.first().copied().filter(|x| x.is_finite()),
        TweakValue::Int(i) => Some(*i as f32),
        _ => None,
    })
}

/// A built-in filter close to a compiled filter graph, with why it is only close; `None` for graphs without one. Sizes
/// convert by first-guess factors (blur), and parameters the file does not store keep their usual defaults.
pub fn native_filter(f: &FilterEffect) -> Option<(Filter, String)> {
    let p = f.procedural.as_ref()?;
    let (filter, note) = match p.name() {
        "invert" => {
            if tweak(p, "invert").is_some_and(|v| v < 0.5) {
                return None;
            }
            (Filter::Invert, "built-in invert")
        }
        "clamp" => (Filter::Clamp { min: tweak(p, "min").unwrap_or(0.0), max: tweak(p, "max").unwrap_or(1.0) }, "built-in clamp"),
        "blur" | "Blur" | "fx_blur" => {
            (Filter::Blur { radius: tweak(p, "intensity").unwrap_or(10.0) * 0.004 * 2048.0 }, "built-in blur (size converted by a first-guess factor)")
        }
        "sharpen" => (Filter::Sharpen { amount: tweak(p, "intensity").unwrap_or(1.0) }, "built-in sharpen"),
        "hsl_perceptive" => {
            let centred = |id: &str, k: f32| tweak(p, id).map_or(0.0, |v| (v - 0.5) * k);
            (
                Filter::Hsl { hue: centred("hue", 1.0), saturation: centred("saturation", 2.0), lightness: centred("luminosity", 2.0) },
                "built-in HSL (perceptive mode left out)",
            )
        }
        _ => return None,
    };
    Some((filter, format!("filter \"{}\" replaced by the {note}", p.name())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_import_reader::model::{EffectHeader, Tweak};

    fn procedural(url: &str, tweaks: Vec<Tweak>) -> Procedural {
        Procedural { uid: None, url: url.into(), tweaks, outputs: vec![], inputs: vec![], alpha_output: String::new(), mono_output: 0 }
    }

    fn source(mode: u64, p: Option<Procedural>) -> ProceduralSource {
        ProceduralSource { uid: None, channels: Default::default(), mode, stamp_cycle: 0, stroke_wires: 0, procedural: p }
    }

    #[test]
    fn every_use_is_a_locked_placeholder_keyed_by_graph() {
        let p = procedural("resource://shelf/mg_dirt/mg_dirt?version=1", vec![]);
        let fill = procedural_fill(&source(1, Some(p.clone())), &["Base Color".into()]);
        assert_eq!(fill.graph, "mg_dirt");
        assert!(matches!(&fill.effect, EffectKind::Placeholder { label, .. } if label == "material fill \"mg_dirt\" (Base Color)"));
        assert_eq!(fill.status.lock_reasons()[0].key, "graph:mg_dirt");
        assert_eq!(mask_generator(&source(0, None)).graph, "?");
        let f = FilterEffect { header: EffectHeader::default(), procedural: Some(p), inputs: vec![], input_alphas: vec![] };
        assert!(filter(&f, true).status.blockers()[0].text.starts_with("mask filter"));
        assert!(native_filter(&f).is_none());
    }

    #[test]
    fn native_filters_follow_their_switches() {
        let with = |name: &str, tweaks| FilterEffect {
            header: EffectHeader::default(),
            procedural: Some(procedural(&format!("/{name}?version=1"), tweaks)),
            inputs: vec![],
            input_alphas: vec![],
        };
        assert_eq!(native_filter(&with("invert", vec![])).map(|f| f.0), Some(Filter::Invert));
        let off = Tweak { identifier: "invert".into(), value: TweakValue::Int(0) };
        assert!(native_filter(&with("invert", vec![off])).is_none(), "a switched-off invert changes nothing");
        let clamp = native_filter(&with("clamp", vec![Tweak { identifier: "max".into(), value: TweakValue::Floats(vec![0.8]) }]));
        assert_eq!(clamp.map(|f| f.0), Some(Filter::Clamp { min: 0.0, max: 0.8 }));
    }
}
