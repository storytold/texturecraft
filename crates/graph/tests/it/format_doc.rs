//! `docs/import.md` stays in step with the runtime: its example parses and renders, and it lists every node kind.

use texturecraft_graph::graph::{self, Graph};

const DOC: &str = include_str!("../../../../docs/import.md");

#[test]
fn the_example_renders() {
    let start = DOC.find("```text\n").expect("an example") + "```text\n".len();
    let len = DOC.get(start..).and_then(|r| r.find("```")).expect("the example ends");
    let g = Graph::parse(&DOC[start..start + len]).expect("the example parses");
    assert_eq!(g.format, graph::FORMAT_VERSION);
    let out = g.evaluate(Some(16)).expect("the example renders");
    assert!(out.contains_key("height"));
}

#[test]
fn every_node_kind_is_listed() {
    let all = graph::SPECS
        .iter()
        .chain(texturecraft_graph::compiled_ops::SPECS)
        .chain(texturecraft_graph::atomic::SPECS)
        .chain(texturecraft_graph::func::FUNC_SPECS)
        .chain(texturecraft_graph::fxmap::FXM_SPECS)
        .chain(texturecraft_graph::patterns::SPECS);
    let missing: Vec<&str> = all.map(|s| s.kind).filter(|k| !DOC.contains(&format!("| `{k}` |"))).collect();
    assert!(missing.is_empty(), "docs/import.md does not list {missing:?}");
}
