//! The decompiler's output run by the graph runtime: synthetic expressions, descriptions and images built in code (no
//! files of the reference app in the repository). These need both the reader and the runtime, so they live here.

use texturecraft_graph::graph::{Graph, b64_encode};
use texturecraft_import_reader::decode::graph_package::Description;
use texturecraft_import_reader::decode::graph_package::compiled::{Expr, Section};
use texturecraft_import_reader::decompile::bitmaps;
use texturecraft_import_reader::decompile::functions::Functions;

fn expr(words: &[u32]) -> Expr {
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    Expr::parse(&Section::new(&bytes, 0), 0).unwrap()
}

fn description() -> Description {
    // the package description's root element is file data, named once in the reader
    let root = texturecraft_import_reader::format_names::DESCRIPTION_ELEMENT;
    let xml = format!(
        r#"<{root}><graphs><graph pkgurl="pkg://t" label="T"><inputs>
        <input uid="4660" identifier="invert" type="4" default="0"/></inputs></graph></graphs></{root}>"#
    );
    Description::parse(xml.as_bytes()).unwrap()
}

#[test]
fn a_select_becomes_a_function_graph_that_parses_and_runs() {
    // (invert ? 1.0 : 0.0)
    let e = expr(&[0x0842_0004, 0x1234, 0x0000_0d00, 0x3f80_0000, 0x0000_0d00, 0, 0x0000_0d09, 0x0002_0001]);
    let mut fns = Functions::default();
    let name = fns.function(&e, &description(), None);
    assert_eq!(fns.function(&e, &description(), None), name, "identical expressions share a function");
    let (_, body) = &fns.subgraphs[0];
    let text = format!("size 8\nsubgraph {name}\n{body}end\nnode 1 input_value name=\"invert\" type=1 v=1\nnode 2 color fn.r=\"{name}\"\noutput o 2\n");
    let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let out = g.evaluate(Some(8)).unwrap();
    assert_eq!(out["o"].px[0][0], 1.0);
}

/// A synthetic 8x8 grayscale JPEG, a ramp from 0 to 252.
const JPEG_RAMP: &str = "ffd8ffe000104a46494600010100000100010000ffdb0043000201010101010201010102020202020403020202020504040304060506060605060606070908060709070606080b08090a0a0a0a0a06080b0c0b0a0c090a0a0affc0000b080008000801011100ffc4001f0000010501010101010100000000000000000102030405060708090a0bffc400b5100002010303020403050504040000017d01020300041105122131410613516107227114328191a1082342b1c11552d1f02433627282090a161718191a25262728292a3435363738393a434445464748494a535455565758595a636465666768696a737475767778797a838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae1e2e3e4e5e6e7e8e9eaf1f2f3f4f5f6f7f8f9faffda0008010100003f00aff0dbfe0db8ff0057ff001417a7fcbb7ff5abffd9";

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

#[test]
fn an_embedded_jpeg_renders_from_the_graph_text_alone() {
    let png = bitmaps::jpeg_to_png(&hex(JPEG_RAMP)).unwrap();
    let text = format!("size 8\nresource bitmap_4.png {}\nnode 1 bitmap file=\"bitmap_4.png\" gray=1\noutput o 1\n", b64_encode(&png));
    let t = &Graph::parse(&text).unwrap().evaluate(Some(8)).unwrap()["o"];
    assert!(t.px[0][0] < 0.05, "{}", t.px[0][0]);
    assert!(t.px[63][0] > 0.9, "{}", t.px[63][0]);
}

#[test]
fn an_embedded_colour_image_takes_its_second_jpeg_as_alpha() {
    let ramp = hex(JPEG_RAMP);
    let png = bitmaps::jpeg_with_alpha_to_png(&ramp, &ramp).unwrap();
    let text = format!("size 8\nresource bitmap_4.png {}\nnode 1 bitmap file=\"bitmap_4.png\"\noutput o 1\n", b64_encode(&png));
    let t = &Graph::parse(&text).unwrap().evaluate(Some(8)).unwrap()["o"];
    // the alpha follows the ramp, dark and transparent at the start, bright and opaque at the end
    assert!(t.px[0][3] < 0.05, "{:?}", t.px[0]);
    assert!(t.px[63][3] > 0.9, "{:?}", t.px[63]);
}

#[test]
fn a_value_stored_by_one_expression_is_read_by_a_later_one() {
    use texturecraft_import_reader::decompile::functions::ValueNode;
    // stores 0.5 at word 3: const, store
    let set = expr(&[0x0900_0002, 0x3f00_0000, 0x0000_0906, 0x0000_0003]);
    let mut fns = Functions::default();
    fns.values = vec![ValueNode { offset: 3, words: 1, expr: set, source: 0, seed: None, reads_images: false, inputs: Vec::new() }];
    // reads word 3
    let read = expr(&[0x0503_0001, 3]);
    let name = fns.function(&read, &description(), None);
    let mut text = String::from("size 8\n");
    for (n, body) in &fns.subgraphs {
        text += &format!("subgraph {n}\n{body}end\n");
    }
    text += &format!("node 1 color fn.r=\"{name}\"\noutput o 1\n");
    let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(g.evaluate(Some(8)).unwrap()["o"].px[0][0], 0.5);
}

#[test]
fn a_left_out_operand_of_and_reads_as_false() {
    // and(invert, left out)
    let e = expr(&[0x0842_0002, 0x1234, 0x0000_081a, 0x0000_ffff]);
    let mut fns = Functions::default();
    let name = fns.function(&e, &description(), None);
    let (_, body) = &fns.subgraphs[0];
    let text = format!("size 8\nsubgraph {name}\n{body}end\nnode 1 input_value name=\"invert\" type=1 v=1\nnode 2 color r=1 fn.r=\"{name}\"\noutput o 2\n");
    let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(g.evaluate(Some(8)).unwrap()["o"].px[0][0], 0.0);
}

mod values {
    use super::*;
    use texturecraft_import_reader::decompile::functions::{Functions, ValueNode};

    fn run(fns: &Functions, name: &str) -> f32 {
        let (_, body) = fns.subgraphs.iter().find(|(n, _)| n == name).unwrap();
        let text = format!("size 8\nsubgraph {name}\n{body}end\nnode 1 color fn.r=\"{name}\"\noutput o 1\n");
        let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        g.evaluate(Some(8)).unwrap()["o"].px[0][0]
    }

    /// const2 (0.25, 0.75): 1 instruction
    fn pair() -> Expr {
        expr(&[0x1140_0001, 0x3e80_0000, 0x3f40_0000])
    }

    #[test]
    fn a_value_is_computed_where_it_is_read_and_the_lane_follows_the_offset() {
        let mut fns = Functions::default();
        fns.values = vec![ValueNode { offset: 3, words: 2, expr: pair(), source: 0, seed: None, reads_images: false, inputs: Vec::new() }];
        // inputat1 #4 (token 0x0503): the second word of the value
        let name = fns.function(&expr(&[0x0503_0001, 4]), &description(), None);
        assert_eq!(run(&fns, &name), 0.75);
    }

    #[test]
    fn a_value_from_an_image_is_computed_in_a_host_through_slots_after_its_own() {
        let mut fns = Functions::default();
        fns.values = vec![ValueNode { offset: 3, words: 2, expr: pair(), source: 0, seed: None, reads_images: true, inputs: vec![7, 9] }];
        let (name, extra) = texturecraft_import_reader::decompile::functions::with_host(2, || fns.function(&expr(&[0x0503_0001, 4]), &description(), None));
        assert_eq!(run(&fns, &name), 0.75);
        assert_eq!(extra, vec![7, 9], "the value node's images, in the slots after the host's two");
        assert!(fns.warnings.is_empty(), "{:?}", fns.warnings);
    }

    #[test]
    fn a_value_from_an_image_outside_a_host_or_outside_every_node_reads_as_zero_and_warns() {
        let mut fns = Functions::default();
        fns.values = vec![ValueNode { offset: 0, words: 2, expr: pair(), source: 0, seed: None, reads_images: true, inputs: vec![7] }];
        let a = fns.function(&expr(&[0x0503_0001, 0]), &description(), None);
        assert_eq!(run(&fns, &a), 0.0);
        assert_eq!(fns.warnings.len(), 1);
        let mut fns = Functions::default();
        let b = fns.function(&expr(&[0x0503_0001, 9]), &description(), None);
        assert_eq!(run(&fns, &b), 0.0);
        assert_eq!(fns.warnings.len(), 1);
    }

    #[test]
    fn init_expressions_set_variables_before_the_function_reads_them() {
        let mut fns = Functions::default();
        // set1 (const 0.5, #2)
        fns.init = vec![expr(&[0x0900_0002, 0x3f00_0000, 0x0000_0907, 0x0002])];
        // get1 #2 [05]
        let name = fns.function(&expr(&[0x0504_0001, 2]), &description(), None);
        assert_eq!(run(&fns, &name), 0.5);
    }

    #[test]
    fn reading_two_lanes_of_a_variable_set_with_four_gives_the_first_two() {
        // vector4-valued set, then get2 and swizzle .y through a mul: the lanes must fit a vec2
        let mut fns = Functions::default();
        // const4 (0.1, 0.2, 0.3, 0.4); set4 #0
        fns.init = vec![expr(&[0x21c0_0002, 0x3dcc_cccd, 0x3e4c_cccd, 0x3e99_999a, 0x3ecc_cccd, 0x0000_09c7, 0])];
        // get2 #0 [05], const2 [11] (1,1) aligned after a 4-byte token + slot; mul2, swizzle1 .y
        let e = expr(&[0x0544_0004, 0x1140_0000, 0x3f80_0000, 0x3f80_0000, 0x0000_0954, 0x0910_0001, 0x0001_0002]);
        let name = fns.function(&e, &description(), None);
        assert!((run(&fns, &name) - 0.2).abs() < 1e-6);
    }
}

#[test]
fn a_float_cast_to_an_integer_takes_the_integer_below() {
    // const -1.5, cast int [token 0x0611]
    let e = expr(&[0x0900_0002, 0xbfc0_0000, 0x0000_0611]);
    let mut fns = Functions::default();
    let name = fns.function(&e, &description(), None);
    let (_, body) = &fns.subgraphs[0];
    assert!(body.contains("f_floor"), "{body}");
    let text = format!("size 8\nsubgraph {name}\n{body}end\nnode 1 color fn.r=\"{name}\"\noutput o 1\n");
    let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(g.evaluate(Some(8)).unwrap()["o"].px[0][0], -2.0);
}

#[test]
fn parameters_reach_the_graph_as_vectors_and_past_their_slider_range() {
    let text = "size 8\nnode 1 input_value name=\"size\" type=3 x=1 y=1\nnode 2 input_value name=\"rotation\" type=0 v=0\nexpose \"size\" 1 v 0 1\nexpose \"rotation\" 2 v 0 1\nnode 3 color r=0 g=0 b=0\noutput o 3\n";
    use texturecraft_core::GraphValue;
    let mut g = Graph::parse(text).unwrap();
    let values = [
        ("size".to_string(), GraphValue::Vector(vec![0.5, 2.0])),
        ("rotation".to_string(), GraphValue::Float(-0.25)),
        ("absent".to_string(), GraphValue::Float(1.0)),
    ];
    texturecraft_compose::graphs::apply_values(&mut g, &values.into_iter().collect());
    let n = |id: u32, k: &str| g.nodes[&id].nums[k];
    assert_eq!((n(1, "x"), n(1, "y")), (0.5, 2.0));
    assert_eq!(n(2, "v"), -0.25);
}

#[test]
fn a_missing_graph_renders_to_nothing() {
    let p = texturecraft_import_reader::model::Procedural {
        uid: None,
        url: "/nothing".into(),
        tweaks: Vec::new(),
        outputs: Vec::new(),
        inputs: Vec::new(),
        alpha_output: String::new(),
        mono_output: 0,
    };
    assert_eq!(texturecraft_import::graph_uses::render_grey(&Default::default(), &p, 8, &texturecraft_import::resources::ProjectImages::none()), None);
}

#[test]
fn a_left_out_sample_position_is_the_pixels_own() {
    // samplecol4(left out, image 0): the input where the pixel is, not at the origin
    let e = expr(&[0x09f4_0001, 0x0000_ffff]);
    let mut fns = Functions::default();
    let name = fns.function(&e, &description(), None);
    let (_, body) = &fns.subgraphs[0];
    assert!(!body.contains("65537"), "{body}");
    let text = format!(
        "size 8\nsubgraph ramp\nmeta kind \"function\"\nnode 2 f_pos\nnode 3 f_swizzle a=@2 mask=\"x\"\nnode 1 f_out in=@3\nend\nsubgraph {name}\n{body}end\nnode 1 pixel_processor fn=\"ramp\"\nnode 2 pixel_processor fn=\"{name}\" in=@1\noutput o 2\noutput r 1\n"
    );
    let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let out = g.evaluate(Some(8)).unwrap();
    for i in [0, 3, 7] {
        assert_eq!(out["o"].px[i][0], out["r"].px[i][0], "pixel {i}");
    }
    assert_ne!(out["o"].px[0][0], out["o"].px[7][0]);
}

#[test]
fn a_left_out_arm_of_a_vector_select_is_a_vector_zero() {
    // invert ? (0.25, 0.75) : left out, its second lane
    let e = expr(&[0x0842_0003, 0x0000_1234, 0x0000_0d40, 0.25f32.to_bits(), 0.75f32.to_bits(), 0x0000_0d49, 0xffff_0001]);
    for (invert, want) in [(1, 0.75), (0, 0.0)] {
        let mut fns = Functions::default();
        let name = fns.function(&e, &description(), Some(1));
        let (_, body) = &fns.subgraphs[0];
        let text =
            format!("size 8\nsubgraph {name}\n{body}end\nnode 1 input_value name=\"invert\" type=1 v={invert}\nnode 2 color r=1 fn.r=\"{name}\"\noutput o 2\n");
        let g = Graph::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(g.evaluate(Some(8)).unwrap_or_else(|e| panic!("{e}\n{text}"))["o"].px[0][0], want, "invert {invert}");
    }
}

/// Every name the decompiler's code tables write is one the graph engine reads, as the number the engine used before.
#[test]
fn the_code_tables_write_names_the_engine_knows() {
    use texturecraft_graph::names::{BLEND_MODES, LCG_SITES, OUTPUT_FORMATS, PATTERN_SHAPES, code_of};
    use texturecraft_import_reader::decode::graph_package::compiled::SampleDepth;
    use texturecraft_import_reader::decompile::functions::rand_mode;
    use texturecraft_import_reader::decompile::{blend_mode, output_format, programs};
    // the compiled format's blend codes and the engine's numbers they stood for in format 2
    for (code, engine) in [(0, 0), (1, 32), (2, 33), (3, 3), (4, 34), (5, 6), (6, 7), (7, 21), (8, 35), (9, 5), (10, 4), (11, 36), (12, 0)] {
        assert_eq!(code_of(BLEND_MODES, blend_mode(code)), Some(engine), "blend code {code}");
    }
    for (depth, engine) in [(SampleDepth::Int8, 5), (SampleDepth::Int16, 6), (SampleDepth::Float16, 7)] {
        assert_eq!(output_format(depth).and_then(|n| code_of(OUTPUT_FORMATS, n)), Some(engine));
    }
    assert_eq!(output_format(SampleDepth::Float32), None);
    for code in 0..=16u8 {
        assert_eq!(programs::pattern_shape(code).and_then(|n| code_of(PATTERN_SHAPES, n)), Some(u32::from(code)));
    }
    assert_eq!(programs::pattern_shape(17), None);
    for mode in 0..=20u8 {
        let named = rand_mode::site_name(mode).map(|n| code_of(LCG_SITES, n));
        assert!(named.is_none() || named == Some(Some(u32::from(mode))), "generator {mode}");
        assert_eq!(named.is_some(), matches!(mode, 1 | 2 | 3 | 5 | 10..=18), "generator {mode}");
    }
}
