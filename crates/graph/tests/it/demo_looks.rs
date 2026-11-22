//! Golden renders: a hash of every output of every node kind's demo graph (the graphs `graph::demo_graph_for` writes,
//! one per kind, original to this crate), at a size that is not the demo graphs' own. The hashes cover every pixel's
//! bits, so any change to what a kind computes shows here. When a change is meant, the failure lists the new table.

use texturecraft_graph::graph::{self, Graph};

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

const SIDE: usize = 48;

/// (kind, hash over its outputs in name order, or 0 when the demo graph doesn't render).
fn renders() -> Vec<(&'static str, u64)> {
    let mut out = Vec::new();
    for s in graph::SPECS {
        let Some(text) = graph::demo_graph_for(s.kind) else { continue };
        let h = match Graph::parse(&text).and_then(|g| g.evaluate(Some(SIDE))) {
            Ok(outs) => outs.values().fold(0u64, |acc, t| acc.rotate_left(7) ^ hash(&t.px)),
            Err(_) => 0,
        };
        out.push((s.kind, h));
    }
    out
}

const EXPECTED: &[(&str, u64)] = &[
    ("color", 0x734649a31b7bc325),
    ("white_noise", 0xeac9376d31895ff2),
    ("gaussian_noise", 0x998ff8f93b9433c7),
    ("perlin", 0xee480f35072e465a),
    ("fbm", 0x2a973ea15be2cec0),
    ("voronoi", 0x834847880ce57a5a),
    ("voronoi_fractal", 0x38c15b971705d4e9),
    ("gradient", 0x0ec3450646db6b25),
    ("circle", 0x44cefbc6c61b3719),
    ("rect", 0x8a709c9557d7d3cd),
    ("polygon", 0x5ea97fad6cb80c11),
    ("checker", 0x61b84bcab00f198b),
    ("bricks", 0xad23a53eb10e0fc1),
    ("fx_map", 0x8142c6ffc2d0c325),
    ("curve", 0x482132feca990292),
    ("svg", 0xb680d55c9f4b741b),
    ("text", 0x23da1dc725f56c2e),
    ("channel_shuffle", 0x001d3c4350ddc346),
    ("input_value", 0x734649a31b7bc325),
    ("pixel_processor", 0x678a832d9af47445),
    ("value_processor", 0x734649a31b7bc325),
    ("sdf_shape", 0xe77ee4c533565926),
    ("sdf_op", 0xba2b5e903c8b7a8d),
    ("sdf_mask", 0xc7158247d45713a9),
    ("sdf_form", 0x8c4d425db417f8f5),
    ("shape", 0x5c17b2dd933bf229),
    ("tile_sampler", 0x9f196dc15cf2d925),
    ("tile_generator", 0x16e5260a18dbc325),
    ("splatter", 0x6c850e20a71f1585),
    ("splatter_circular", 0x4536bc4835598c06),
    ("clouds", 0x30fcc6899eef370b),
    ("bnw_spots", 0x566f4fd667914628),
    ("crystal", 0x3d253ec74fca3f58),
    ("directional_noise", 0x3b180df4a45f1804),
    ("grunge", 0x9646f0309b786c74),
    ("plasma", 0x301d977fe85642de),
    ("stripes", 0xd9a3b702a1d42425),
    ("weave", 0xfbdafd2c55800985),
    ("star", 0x49a1c4608f590c37),
    ("gradient_axial", 0x0ec3450646db6b25),
    ("gradient_circular", 0xebf5c15a02bbb043),
    ("switch", 0x352426de8c82bb39),
    ("flood_fill", 0x180a14799a7a8559),
    ("flood_fill_random", 0x6b25c03d029311f2),
    ("flood_fill_color", 0x310a104c702cea2d),
    ("flood_fill_to_index", 0xf33b81947acdb8a5),
    ("flood_fill_to_position", 0x2032bb053317e80a),
    ("flood_fill_to_bbox_size", 0xc73b073ab264e6ba),
    ("flood_fill_gradient", 0xca5a6818e63b2883),
    ("vector_warp", 0x0303a3ee223a52b6),
    ("slope_blur", 0x9153a7f808d80a72),
    ("blur_hq", 0x0e41b26d1e09a308),
    ("histogram_range", 0x681f54e85497ea5d),
    ("bevel", 0x299845ac3179b1a8),
    ("distance", 0xa47d1406adb041da),
    ("color_match", 0x001d3c4350ddc346),
    ("curvature_smooth", 0x4874ae5d49f00d1b),
    ("height_blend", 0xdf9d50e00e40d7fd),
    ("make_it_tile_photo", 0x7c54b92c85bf14ab),
    ("make_it_tile_patch", 0x7ab91ce329ae726e),
    ("clamp", 0x001d3c4350ddc346),
    ("highpass", 0x6a276fed00b16370),
    ("swirl", 0xee746e4ffe98b076),
    ("edge_wear", 0x0d21cae8b49cc325),
    ("dirt", 0xd4d427483ef296a1),
    ("dust", 0x8254ed55022c0345),
    ("blur", 0x58a42ff97320c592),
    ("directional_blur", 0xbbbf4e65dda5914e),
    ("sharpen", 0x72e599d5262526fa),
    ("levels", 0x001d3c4350ddc346),
    ("adjust", 0x001d3c4350ddc346),
    ("invert", 0x7ec04aa25b67e043),
    ("threshold", 0x11706f6fd25c08c8),
    ("histogram_scan", 0x4dc9a8e2770c7fa1),
    ("quantize", 0x1c374190d6c72b04),
    ("to_gray", 0x12549caf537f9501),
    ("channel", 0x001d3c4350ddc346),
    ("merge", 0x11a1a133e6e8dbd1),
    ("tint", 0x001d3c4350ddc346),
    ("colorize", 0x6d81230ccd06e2da),
    ("hsl", 0x001d3c4350ddc346),
    ("transform", 0xeea4c3b40eb6ba3c),
    ("mirror", 0x42648fcbcc12a829),
    ("warp", 0xe870523b7c040d4b),
    ("directional_warp", 0x4c30f8ca6fcc94b3),
    ("blend", 0xc5c2b364874b479f),
    ("normal", 0x4f9df70ee8fa508e),
    ("normal_blend", 0x3705cd46d6694a2c),
    ("ao", 0xa5a5bcd1d4c96299),
    ("quantize_color", 0x9339aa0c366dbdcf),
    ("curvature", 0xfb4acb6c4f03cca4),
    ("edge_detect", 0x7b8e36e4215bdc42),
    ("emboss", 0xe6946a38d345b5c1),
    ("non_uniform_blur", 0xc485cf5bddb6d143),
    ("radial_blur", 0x3ccc5c3e53f950b9),
    ("anisotropic_blur", 0xbbbf4e65dda5914e),
    ("symmetry", 0x12163d0a963c7dfa),
    ("skew", 0x0573fa1b32f276e6),
    ("auto_levels", 0x790c0a0a927c35be),
    ("channel_mixer", 0x001d3c4350ddc346),
    ("alpha_merge", 0x63f3cbb7030e6d33),
    ("color_to_mask", 0xb73222423f64b547),
    ("replace_color", 0x0b05629ba9a37209),
    ("pow", 0xb290d01bfdf9b089),
    ("histogram_shift", 0x001d3c4350ddc346),
    ("grayscale_conversion", 0xe15ef3988d2ce7fb),
    ("cells", 0x12584904d25ce6f3),
    ("fractal_sum", 0xb69838638c49bed2),
    ("waveform", 0x66162d717d53c8d5),
    ("gaussian_spots", 0xce28ef1e3f277307),
    ("normal_invert", 0x8caf63ff8721b4a1),
    ("normal_normalize", 0x108c5a7b3cffd320),
    ("normal_vector_rotation", 0x7bc877bcd7648a6a),
    ("normal_to_height", 0x74f93fb40d246e6c),
    ("sobel", 0x18221e0c1fd77b9c),
    ("glow", 0x10c5340e1fc78992),
    ("mosaic", 0x77dd8df52b45c8e7),
    ("non_uniform_directional_warp", 0xe9c6f2dfcf27daea),
    ("multi_directional_warp", 0x5f503f0477648111),
    ("normal_combine", 0x290d146e3847290a),
    ("triangle_grid", 0xad2c03d013bbf8fb),
    ("grunge_map", 0xed9aca467e58e756),
    ("rgba_split", 0x001d3c4350ddc346),
    ("alpha_split", 0x001d3c4350ddc346),
    ("weave_generator", 0xa2a8ad47e4d69aa5),
    ("starburst", 0x160f1777ea646bbe),
    ("gradient_reflected", 0x0ec3450646db6b25),
    ("shape_stroke", 0xe02ff9da6d70eb6c),
    ("shape_glow", 0x2f1b5184171fc6e3),
    ("chrominance_extract", 0x70b537174539fd3f),
    ("shape_drop_shadow", 0xe951f11a621e6de4),
];

#[test]
fn every_kind_renders_as_recorded() {
    let got = renders();
    let listing: Vec<String> = got.iter().map(|(k, h)| format!("    ({k:?}, 0x{h:016x}),")).collect();
    assert_eq!(got.as_slice(), EXPECTED, "render hashes changed; now:\n{}", listing.join("\n"));
}
