//! Source identities and parameter mappings for compiled filters with a verified native equivalent.

use texturecraft_core::{Channel, ChannelKind, Filter, GraphInput, GraphRef, GraphValue, StableHash};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeGraph {
    ScalarInvert,
}

struct Entry {
    package_version: &'static str,
    graph_path: &'static str,
    compiled_fingerprint: u128,
    translated_key: &'static str,
    replacement: NativeGraph,
}

const ENTRIES: &[Entry] = &[Entry {
    package_version: "408eadc6a9381617ea5c07cb62f20856939df6da",
    graph_path: "pkg://invert",
    compiled_fingerprint: 0x36b5cc01a5862e175939e2266a3236a7,
    translated_key: "56985cc99137f7944bfdbd8447cb14af",
    replacement: NativeGraph::ScalarInvert,
}];

pub(crate) fn recognize(version: &str, path: &str, key: &str, compiled: &[u8]) -> Option<NativeGraph> {
    ENTRIES.iter().find_map(|entry| {
        (entry.package_version == version
            && entry.graph_path == path
            && entry.translated_key == key
            && entry.compiled_fingerprint == StableHash::of(compiled).finish())
        .then_some(entry.replacement)
    })
}

impl NativeGraph {
    pub(crate) fn filter(self, graph: &GraphRef, channels: Option<&[Channel]>) -> Option<(Filter, &'static str)> {
        match self {
            // This version defaults to inverting its scalar input and passing its coverage through.
            Self::ScalarInvert => {
                let enabled = match graph.params.get("invert") {
                    None | Some(GraphValue::Int(1) | GraphValue::Bool(true)) => true,
                    Some(GraphValue::Float(value)) => *value == 1.0,
                    Some(GraphValue::Vector(values)) => values.as_slice() == [1.0],
                    _ => false,
                };
                (enabled
                    && graph.output.as_deref() == Some("output")
                    && graph.inputs.len() == 2
                    && graph.inputs.get(&0) == Some(&GraphInput::StackAlpha { channel: None })
                    && graph.inputs.get(&1) == Some(&GraphInput::Stack { channel: None })
                    && channels.is_none_or(|channels| {
                        channels.iter().all(|channel| {
                            matches!(channel.kind(), ChannelKind::Scalar | ChannelKind::Signed)
                                && graph.outputs.get(channel).is_some_and(|output| output == "output")
                                && graph.output_alphas.get(channel).is_some_and(|output| output == "output_Alpha")
                        })
                    }))
                .then_some((Filter::Invert, "standard scalar invert converted to TextureCraft's native Invert"))
            }
        }
    }
}
