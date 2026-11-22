//! [`GraphRef`]: a node graph run as a fill source ([`super::Source::Graph`]), a mask generator or a filter
//! ([`super::EffectKind::Graph`]). The graph's text is not held here: a project stores each distinct text once, named by
//! its [`text_key`] (the project file's `graphs/<key>.txt`), and a reference names it by that key. What the graph is
//! run with is held here: values for its parameters, what feeds its image inputs, which outputs are read and the seed.
//!
//! A graph whose inputs read nothing of the stack (baked mesh maps, constants, images, other graphs) renders once per
//! texture set and is sampled like an image. One that reads the stack below it or an anchor point
//! ([`GraphRef::reads_stack`]) is a filter: it renders over the whole texture set from what is below it.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{LayerId, Levels};
use crate::{BakedMap, Channel};

/// How deep graphs may feed each other's inputs ([`GraphInput::Nested`]). Deeper references are refused where they are
/// evaluated (a hostile file can nest as deep as its parser allows).
pub const MAX_GRAPH_NESTING: usize = 8;

/// The value of one graph parameter, typed as the graph declares it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GraphValue {
    Float(#[serde(deserialize_with = "number")] f32),
    /// Two to four lanes (a vector or a colour).
    Vector(#[serde(deserialize_with = "numbers")] Vec<f32>),
    Int(i32),
    Int2([i32; 2]),
    Bool(bool),
    /// A text parameter (a font graph's text).
    Text(String),
}

/// A number as JSON holds it: a value that is not a number (written as `null`) reads 0.
fn number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    Ok(Option::<f32>::deserialize(d)?.unwrap_or(0.0))
}

fn numbers<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<f32>, D::Error> {
    Ok(Vec::<Option<f32>>::deserialize(d)?.into_iter().map(|v| v.unwrap_or(0.0)).collect())
}

fn colour<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[f32; 3], D::Error> {
    let [r, g, b] = <[Option<f32>; 3]>::deserialize(d)?;
    Ok([r.unwrap_or(0.0), g.unwrap_or(0.0), b.unwrap_or(0.0)])
}

impl GraphValue {
    /// The value as the lanes the graph runtime takes (one to four numbers; none for a text); non-finite lanes read 0,
    /// lanes past the fourth are left out.
    pub fn lanes(&self) -> Vec<f32> {
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        match self {
            GraphValue::Float(v) => vec![finite(*v)],
            GraphValue::Vector(v) => v.iter().take(4).copied().map(finite).collect(),
            GraphValue::Int(v) => vec![*v as f32],
            GraphValue::Int2([a, b]) => vec![*a as f32, *b as f32],
            GraphValue::Bool(b) => vec![if *b { 1.0 } else { 0.0 }],
            GraphValue::Text(_) => Vec::new(),
        }
    }
}

/// What feeds one image input of a graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GraphInput {
    /// A baked mesh map of the texture set.
    Baked(BakedMap),
    /// One colour everywhere (the graph's own values, 0..1).
    Constant(#[serde(deserialize_with = "colour")] [f32; 3]),
    /// An image resource of the project, by name.
    Image(String),
    /// Another graph's output ([`GraphRef::output`], else its first).
    Nested(Box<GraphRef>),
    /// The stack below the graph: the mask it is in, or a layer channel (`None`: the channel it runs on).
    Stack { channel: Option<Channel> },
    /// The coverage of what [`GraphInput::Stack`] with the same channel reads (grey; 0 where nothing is below).
    StackAlpha { channel: Option<Channel> },
    /// The stack recorded by an anchor point effect, in `channel` (`None`: the channel it runs on), read through
    /// `levels`.
    Anchor {
        anchor: LayerId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<Channel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        levels: Option<Levels>,
    },
}

/// A graph run as a source, a generator or a filter. See the module docs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphRef {
    /// The [`text_key`] of the graph's text.
    pub key: String,
    /// What the graph is called, for labels.
    #[serde(default)]
    pub name: String,
    /// Parameter values by the graph's parameter name; parameters not listed keep the graph's defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, GraphValue>,
    /// What feeds the image inputs, by input index; inputs not listed read black.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<u32, GraphInput>,
    /// The output read where nothing more specific is named (a source, a mask, a nested graph); `None`: the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// A filter's output per layer channel. A filter naming outputs here leaves the other channels as they are.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<Channel, String>,
    /// The output that is the coverage of a source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha_output: Option<String>,
    /// A filter's coverage output per layer channel.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub output_alphas: BTreeMap<Channel, String>,
    /// Outputs that hold colour (the others are grey), as the graph declares them.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub colour_outputs: BTreeSet<String>,
    /// Outputs rendered together with the ones read (so that the channels of one fill, each reading its own output,
    /// share one render).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub keep: BTreeSet<String>,
    /// Added to the graph's random seed.
    #[serde(default)]
    pub seed: u32,
    /// Read through a projection at one point per texel (bilinear) even where its output is minified, not prefiltered
    /// through mip levels: a fill set to bilinear filtering.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unfiltered: bool,
}

impl GraphRef {
    /// A material fill may represent one graph instance by a source for each output channel.
    pub fn shares_parameters_with(&self, other: &Self) -> bool {
        self.key == other.key
            && self.name == other.name
            && self.params == other.params
            && self.inputs == other.inputs
            && self.keep == other.keep
            && self.alpha_output == other.alpha_output
            && self.seed == other.seed
    }

    /// A reference to the graph with text key `key`, everything else at its defaults.
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> GraphRef {
        GraphRef {
            key: key.into(),
            name: name.into(),
            params: BTreeMap::new(),
            inputs: BTreeMap::new(),
            output: None,
            outputs: BTreeMap::new(),
            alpha_output: None,
            output_alphas: BTreeMap::new(),
            colour_outputs: BTreeSet::new(),
            keep: BTreeSet::new(),
            seed: 0,
            unfiltered: false,
        }
    }

    /// The graphs this one reads through its inputs, itself first, down to [`MAX_GRAPH_NESTING`].
    pub fn graphs(&self) -> Vec<&GraphRef> {
        let mut out = vec![self];
        let mut at = 0;
        let mut depth = vec![0usize];
        while let Some(g) = out.get(at).copied() {
            let d = depth.get(at).copied().unwrap_or(0);
            at += 1;
            if d >= MAX_GRAPH_NESTING {
                continue;
            }
            for input in g.inputs.values() {
                if let GraphInput::Nested(inner) = input {
                    out.push(inner);
                    depth.push(d + 1);
                }
            }
        }
        out
    }

    /// The text keys of this graph and every graph feeding it.
    pub fn text_keys(&self) -> BTreeSet<&str> {
        self.graphs().into_iter().map(|g| g.key.as_str()).collect()
    }

    /// True if the graph reads the stack below it or an anchor point (directly; a nested graph reading the stack
    /// cannot be evaluated and reads black).
    pub fn reads_stack(&self) -> bool {
        self.inputs.values().any(|i| matches!(i, GraphInput::Stack { .. } | GraphInput::StackAlpha { .. } | GraphInput::Anchor { .. }))
    }

    /// The anchor points the graph reads.
    pub fn anchors(&self) -> impl Iterator<Item = LayerId> + '_ {
        self.inputs.values().filter_map(|i| match i {
            GraphInput::Anchor { anchor, .. } => Some(*anchor),
            _ => None,
        })
    }

    /// Every output read (by any channel, as coverage or as the default), sorted; empty when only the first is read.
    pub fn outputs_read(&self) -> BTreeSet<&str> {
        let mut out: BTreeSet<&str> = self.outputs.values().chain(self.output_alphas.values()).map(String::as_str).collect();
        out.extend(self.output.as_deref());
        out.extend(self.alpha_output.as_deref());
        out
    }

    /// The outputs one render computes: those read and those kept with them, sorted; empty when only the first is
    /// read.
    pub fn rendered(&self) -> BTreeSet<&str> {
        let mut out = self.outputs_read();
        out.extend(self.keep.iter().map(String::as_str));
        out
    }

    /// The output a filter writes into `channel`: `None` when it leaves the channel as it is.
    pub fn output_for(&self, channel: Channel) -> Option<Option<&str>> {
        match self.outputs.get(&channel) {
            Some(o) => Some(Some(o.as_str())),
            None if !self.outputs.is_empty() => None,
            None => Some(self.output.as_deref()),
        }
    }
}

/// The key a graph text is stored and referenced under: 32 hex digits of a 128-bit FNV-1a hash of its bytes. The same
/// on every machine and in every build.
pub fn text_key(text: &str) -> String {
    format!("{:032x}", StableHash::of(text.as_bytes()).finish())
}

/// True if `key` has the form of a [`text_key`] (or of any key made by [`StableHash`]): 32 lowercase hex digits.
pub fn is_key(key: &str) -> bool {
    key.len() == 32 && key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 128-bit FNV-1a: a hash that is the same on every platform and in every build (unlike the standard library's
/// hashers), for keys stored in project files.
#[derive(Clone, Copy, Debug)]
pub struct StableHash(u128);

const FNV_OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
const FNV_PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

impl Default for StableHash {
    fn default() -> Self {
        StableHash(FNV_OFFSET)
    }
}

impl StableHash {
    pub fn new() -> StableHash {
        StableHash::default()
    }

    pub fn of(bytes: &[u8]) -> StableHash {
        let mut h = StableHash::new();
        h.bytes(bytes);
        h
    }

    pub fn bytes(&mut self, bytes: &[u8]) -> &mut Self {
        for b in bytes {
            self.0 ^= u128::from(*b);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
        self
    }

    /// A string, length first (so that two strings never run together).
    pub fn str(&mut self, s: &str) -> &mut Self {
        self.u64(s.len() as u64).bytes(s.as_bytes())
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.bytes(&v.to_le_bytes())
    }

    /// A float by its bits (−0 and NaN as they are).
    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.bytes(&v.to_bits().to_le_bytes())
    }

    pub fn finish(&self) -> u128 {
        self.0
    }

    /// The hash as a key: 32 hex digits.
    pub fn key(&self) -> String {
        format!("{:032x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested(depth: usize) -> GraphRef {
        let mut g = GraphRef::new(text_key("leaf"), "leaf");
        for d in 0..depth {
            let mut outer = GraphRef::new(text_key(&format!("g{d}")), "outer");
            outer.inputs.insert(0, GraphInput::Nested(Box::new(g)));
            g = outer;
        }
        g
    }

    #[test]
    fn text_keys_are_stable_and_shaped() {
        // FNV-1a 128 of the empty input is its offset basis
        assert_eq!(text_key(""), "6c62272e07bb014262b821756295c58d");
        assert_ne!(text_key("a"), text_key("b"));
        assert!(is_key(&text_key("node 1 color")));
        assert!(!is_key("../../etc/passwd") && !is_key("ABCDEF") && !is_key(&"g".repeat(32)));
        let mut a = StableHash::new();
        a.str("ab").str("c");
        let mut b = StableHash::new();
        b.str("a").str("bc");
        assert_ne!(a.finish(), b.finish(), "strings never run together");
    }

    #[test]
    fn nesting_is_walked_to_its_bound() {
        assert_eq!(nested(3).graphs().len(), 4);
        assert_eq!(nested(3).text_keys().len(), 4);
        assert_eq!(nested(MAX_GRAPH_NESTING + 5).graphs().len(), MAX_GRAPH_NESTING + 1);
    }

    #[test]
    fn filters_name_their_outputs_per_channel() {
        let mut g = GraphRef::new(text_key("x"), "x");
        assert_eq!(g.output_for(Channel::Roughness), Some(None), "no outputs named: the first everywhere");
        g.output = Some("grey".into());
        assert_eq!(g.output_for(Channel::Roughness), Some(Some("grey")));
        g.outputs.insert(Channel::BaseColor, "colour".into());
        assert_eq!(g.output_for(Channel::Roughness), None, "a channel not named is left as it is");
        assert_eq!(g.output_for(Channel::BaseColor), Some(Some("colour")));
        g.alpha_output = Some("mask".into());
        assert_eq!(g.outputs_read().into_iter().collect::<Vec<_>>(), ["colour", "grey", "mask"]);
        assert!(!g.reads_stack());
        g.inputs.insert(1, GraphInput::Anchor { anchor: LayerId(4), channel: None, levels: None });
        assert!(g.reads_stack());
        assert_eq!(g.anchors().collect::<Vec<_>>(), [LayerId(4)]);
    }

    #[test]
    fn values_become_lanes_and_round_trip() {
        assert_eq!(GraphValue::Vector(vec![1.0, f32::NAN, 3.0, 4.0, 5.0]).lanes(), [1.0, 0.0, 3.0, 4.0]);
        assert_eq!(GraphValue::Int2([2, -3]).lanes(), [2.0, -3.0]);
        assert_eq!(GraphValue::Bool(true).lanes(), [1.0]);
        let mut g = nested(1);
        g.params.insert("scale".into(), GraphValue::Float(2.5));
        g.inputs.insert(3, GraphInput::Stack { channel: Some(Channel::Height) });
        g.outputs.insert(Channel::Height, "height".into());
        let json = serde_json::to_value(&g).unwrap();
        assert_eq!(serde_json::from_value::<GraphRef>(json.clone()).unwrap(), g);
        // only what is set is written
        assert!(json.get("output_alphas").is_none() && json.get("alpha_output").is_none());
        // values that are not numbers are written as null and read back as 0
        let nan = GraphValue::Vector(vec![f32::NAN, 1.0]);
        assert_eq!(serde_json::from_value::<GraphValue>(serde_json::to_value(&nan).unwrap()).unwrap(), GraphValue::Vector(vec![0.0, 1.0]));
        let c = GraphInput::Constant([f32::INFINITY, 0.5, 0.5]);
        assert_eq!(serde_json::from_value::<GraphInput>(serde_json::to_value(&c).unwrap()).unwrap(), GraphInput::Constant([0.0, 0.5, 0.5]));
        let minimal: GraphRef = serde_json::from_value(serde_json::json!({"key": text_key("y")})).unwrap();
        assert_eq!(minimal, GraphRef::new(text_key("y"), ""));
    }
}
