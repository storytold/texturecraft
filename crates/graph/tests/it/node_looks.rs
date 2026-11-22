//! Render hashes of the generator nodes whose looks live in `grunge` / `noise` functions (extracted from the node
//! code, roadmap step 23). The hashes were recorded before the extraction: the node outputs must stay bit-identical.
//! They were recorded again once when the engine's transcendental functions moved to one implementation for every
//! platform (`texturecraft_graph::math`): since then they are the same on every machine.

use texturecraft_graph::graph::Graph;

/// FNV-1a over the bits of every pixel component.
fn hash(px: &[[f32; 4]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in px {
        for c in p {
            for b in c.to_bits().to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    h
}

const NODES: &[&str] = &[
    "grunge variant=0 seed=3",
    "grunge variant=0 seed=3 dirt_specks_opacity=0.6 scratches_opacity=0.5 sharpen=0.4",
    "grunge variant=0 seed=4 dirt_specks_opacity=0.6 invert_dirt=1 balance=0.3",
    "grunge variant=1 seed=5 quantity=5 scratch_width=0.3 scratch_length=0.2",
    "grunge variant=2 seed=6 details=30 coverage=0.5 coverage_contrast=0.1",
    "grunge variant=3 seed=7 disorder=0.4",
    "voronoi seed=2",
    "voronoi seed=2 mode=2 distance_mode=1 distortion=0.6 aspect=2 offset_x=0.25",
    "voronoi_fractal seed=2",
    "voronoi_fractal seed=9 blend=1 opacity=0.7 min_level=1 max_level=4 color_seed_mode=1 color_seed=5",
    "voronoi_fractal seed=9 blend=2 roughness=0.8 lacunarity=3 mode=1 edge_thickness=0.1 rounded=0.5",
    "cells variant=0 seed=1 luminance_random=0.5",
    "cells variant=1 seed=1",
    "cells variant=1 seed=1 color_source=1 pseudorandom_seed=7",
    "cells variant=2 seed=8 edge_width=0.1 jitter=0.6",
    "cells variant=3 seed=8 hardness=4 disorder=0.3 anisotropy=0.5",
    "crystal variant=0 seed=4",
    "crystal variant=1 seed=4 scale=9 disorder=0.2",
    "bnw_spots variant=0 seed=3",
    "bnw_spots variant=1 seed=3",
    "bnw_spots variant=2 seed=3 scale=12",
    "gaussian_spots seed=5 size=0.7 size_variation=0.3",
    "triangle_grid seed=3",
    "triangle_grid seed=3 count=5 border=0.1 variation=0.4",
    "triangle_grid style=1 seed=3",
    "triangle_grid style=1 seed=3 thin_lines=1 output=1 triangulation=1 quincunx_offset_x=0.5",
    "triangle_grid style=1 seed=3 output=2 triangulation=2 x_amount=7 y_amount=9 height_random=0.4",
    "weave count=6 width=0.8",
    "weave count=9 width=0.5",
    "weave_generator",
    "weave_generator tile_x=2 tile_y=3 weave=3 shape=0 gap_x=0.2 gap_y=0",
    "weave_generator shape=0.8 gap_x=0 gap_y=0.3",
    "starburst seed=3",
    "starburst seed=3 count=5 rays=12 sharpness=4 size=0.8 rotation=0.2 rotation_random=0.6",
];

const EXPECTED: &[u64] = &[
    0x7e47396cd9cc8a54,
    0x7f4682c5a4851331,
    0xbdcdf3162a86e37a,
    0xa4b7bcd07ea0bbed,
    0x9e4d5d4c7f3f7928,
    0x38568f21232e646c,
    0x0f5a094e2c1c5ec0,
    0x6fd73747db80d65a,
    0x312c3afa28ecee14,
    0x7ff4612dfc9e7e2b,
    0xa91aaa459730454e,
    0x1391ab2106ded146,
    0xb4453bd7a7a0ed37,
    0x792df994ea401ce8,
    0xae4eb3df5d623ee2,
    0x72eefb7e2a072284,
    0xb0f51369fd14ed7b,
    0x5849833a7bf193fd,
    0x4395196f18135ff4,
    0xb26dffd75b40818d,
    0xbb856a9bb7c23047,
    0xfb7b7deb8ec02877,
    0x9e6be0f37ba60fbd,
    0xf67cb4f88946bc9d,
    0x1f6355b313e44c86,
    0xca968903ad3f5790,
    0x4ddd807bb109ccf7,
    0x01fd11aa83ca1145,
    0x01fc39e6dbb3f045,
    0x5c0178b4c7c9b325,
    0x4c9aa702fc152325,
    0x6a3f5dbc853ff6a5,
    0x2f69ff1be31cf2f6,
    0x75fc5a82da7849be,
];

#[test]
fn extracted_looks_render_unchanged() {
    let mut got = Vec::new();
    for spec in NODES {
        let g = Graph::parse(&format!("size 64\nnode 1 {spec}\noutput a 1\n")).unwrap();
        let t = g.evaluate_node(1, 64).unwrap();
        got.push(hash(&t.px));
    }
    let listing: Vec<String> = got.iter().map(|h| format!("    0x{h:016x},")).collect();
    assert_eq!(got.as_slice(), EXPECTED, "render hashes changed; now:\n{}", listing.join("\n"));
}
