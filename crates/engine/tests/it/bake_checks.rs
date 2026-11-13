//! What bake mode lists before a bake: overlapping UVs and parts "By Mesh Name" leaves unpaired

use texturecraft_engine::Session;
use texturecraft_engine::bake::NamePair;
use texturecraft_engine::core::MeshMatchMode;

/// Two quads side by side; `shared_uvs` lays both on the same UV square instead of side by side in UV space
fn two_quads(shared_uvs: bool) -> Session {
    let second_u = if shared_uvs { 0.0 } else { 0.5 };
    let obj = format!(
        "o Box_low\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 0.5 0\nvt 0.5 1\nvt 0 1\nf 1/1 2/2 3/3 4/4\n\
         o Lone_low\nv 2 0 0\nv 3 0 0\nv 3 1 0\nv 2 1 0\nvt {second_u} 0\nvt {} 0\nvt {} 1\nvt {second_u} 1\nf 5/5 6/6 7/7 8/8\n",
        second_u + 0.5,
        second_u + 0.5
    );
    Session::from_mesh_bytes("quads.obj", obj.as_bytes(), 128, "quads").unwrap()
}

const HIGH_POLY: &[u8] = b"o Box_high\nv 0 0 0.01\nv 1 0 0.01\nv 0 1 0.01\nf 1 2 3\no Extra_high\nv 2 0 0.01\nv 3 0 0.01\nv 2 1 0.01\nf 4 5 6\n";

#[test]
fn overlapping_uvs_are_measured_before_the_bake() {
    assert!(two_quads(true).bake_uv_overlap(0).unwrap() > 0.4, "the second quad covers the first one's UVs");
    assert_eq!(two_quads(false).bake_uv_overlap(0).unwrap(), 0.0, "side by side in UV space nothing overlaps");
    const MISSING_TEXTURE_SET: usize = 9;
    assert!(two_quads(false).bake_uv_overlap(MISSING_TEXTURE_SET).is_err());
}

#[test]
fn overlap_in_a_uv_tile_left_out_of_the_bake_is_not_counted() {
    // One quad in tile 1001, two on the same UVs in tile 1002
    let obj = "o First\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 0.5 0\nvt 0.5 1\nvt 0 1\nf 1/1 2/2 3/3 4/4\n\
               o Second\nv 2 0 0\nv 3 0 0\nv 3 1 0\nv 2 1 0\nvt 1 0\nvt 1.5 0\nvt 1.5 1\nvt 1 1\nf 5/5 6/6 7/7 8/8\n\
               o Third\nv 4 0 0\nv 5 0 0\nv 5 1 0\nv 4 1 0\nvt 1 0\nvt 1.5 0\nvt 1.5 1\nvt 1 1\nf 9/9 10/10 11/11 12/12\n";
    let mut session = Session::from_mesh_bytes("tiles.obj", obj.as_bytes(), 128, "tiles").unwrap();
    assert_eq!(session.doc.project.texture_sets[0].uv_tiles, [1001, 1002]);
    assert!(session.bake_uv_overlap(0).unwrap() > 0.4, "the last two quads share tile 1002's UVs");

    session.doc.project.texture_sets[0].bake_excluded_tiles.insert(1002);
    assert_eq!(session.bake_uv_overlap(0).unwrap(), 0.0, "tile 1001 alone has no overlap");
}

#[test]
fn overlap_is_measured_at_the_same_detail_in_every_uv_tile() {
    // Tiles 1001 and 1010 make a grid ten tiles wide; 512 texels across all ten would give each tile about 51
    const TEXELS_PER_TILE: f32 = 512.0;
    const SLIVER_TEXELS: f32 = 4.0;
    // Inside tile 1010 (column 9), clear of the wide raster's texel centres
    const SLIVER_LEFT_U: f32 = 9.1;
    let (left, right) = (SLIVER_LEFT_U, SLIVER_LEFT_U + SLIVER_TEXELS / TEXELS_PER_TILE);
    let sliver = |name: &str, first: u32, x: f32| {
        let (second, third, fourth) = (first + 1, first + 2, first + 3);
        let corners = format!("v {x} 0 0\nv {} 0 0\nv {} 1 0\nv {x} 1 0\n", x + 1.0, x + 1.0);
        format!(
            "o {name}\n{corners}vt {left} 0\nvt {right} 0\nvt {right} 1\nvt {left} 1\nf {first}/{first} {second}/{second} {third}/{third} {fourth}/{fourth}\n"
        )
    };
    let corner = "o Corner\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 0.5 0\nvt 0.5 1\nvt 0 1\nf 1/1 2/2 3/3 4/4\n";
    let obj = [corner.to_owned(), sliver("First", 5, 2.0), sliver("Second", 9, 4.0)].concat();
    let session = Session::from_mesh_bytes("wide.obj", obj.as_bytes(), 128, "wide").unwrap();
    assert_eq!(session.doc.project.texture_sets[0].uv_tiles, [1001, 1010]);

    assert!(session.bake_uv_overlap(0).unwrap() > 0.0, "two slivers a few texels wide overlap in tile 1010");
}

#[test]
fn a_triangle_across_a_uv_tile_border_counts_in_both_tiles() {
    // Two quads on the same UVs, from u 0.9 in tile 1001 to u 1.1 in tile 1002
    let obj = "o First\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0.9 0\nvt 1.1 0\nvt 1.1 1\nvt 0.9 1\nf 1/1 2/2 3/3 4/4\n\
               o Second\nv 2 0 0\nv 3 0 0\nv 3 1 0\nv 2 1 0\nvt 0.9 0\nvt 1.1 0\nvt 1.1 1\nvt 0.9 1\nf 5/5 6/6 7/7 8/8\n";
    let mut session = Session::from_mesh_bytes("border.obj", obj.as_bytes(), 128, "border").unwrap();
    assert_eq!(session.doc.project.texture_sets[0].uv_tiles, [1001, 1002]);

    for (excluded, kept) in [(1002, 1001), (1001, 1002)] {
        let excluded_tiles = &mut session.doc.project.texture_sets[0].bake_excluded_tiles;
        excluded_tiles.clear();
        excluded_tiles.insert(excluded);
        assert!(session.bake_uv_overlap(0).unwrap() > 0.4, "tile {kept} holds part of the overlap");
    }
}

#[test]
fn unpaired_parts_are_listed_only_when_matching_by_name() {
    let mut session = two_quads(false);
    assert_eq!(session.bake_name_pairs(0).unwrap(), None, "no high poly");
    session.add_high_poly_bytes("high.obj", HIGH_POLY.to_vec()).unwrap();
    assert_eq!(session.bake_name_pairs(0).unwrap(), None, "\"Always\" pairs every part");

    session.doc.project.texture_sets[0].bake.high_poly.match_mode = MeshMatchMode::ByMeshName;
    let pairs = session.bake_name_pairs(0).unwrap().unwrap();
    let unpaired: Vec<&NamePair> = pairs.iter().filter(|pair| pair.unpaired()).collect();
    assert_eq!(unpaired, [&NamePair { low: Some("Lone_low".into()), high: None }, &NamePair { low: None, high: Some("Extra_high".into()) }], "{pairs:?}");
    assert!(pairs.contains(&NamePair { low: Some("Box_low".into()), high: Some("Box_high".into()) }));

    session.doc.project.texture_sets[0].bake.high_poly.use_low_as_high = true;
    assert_eq!(session.bake_name_pairs(0).unwrap(), None, "the low poly as its own high poly pairs every part");
}

#[test]
fn a_set_matching_always_bakes_from_every_high_poly_part() {
    let obj = "o Box_low\nusemtl Body\nv 0 0 0\nv 1 0 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 1/1 2/2 3/3\n\
               o Lid_low\nusemtl Lid\nv 2 0 0\nv 3 0 0\nv 2 1 0\nvt 0 0\nvt 1 0\nvt 0 1\nf 4/4 5/5 6/6\n";
    let mut session = Session::from_mesh_bytes("sets.obj", obj.as_bytes(), 128, "sets").unwrap();
    session.add_high_poly_bytes("high.obj", HIGH_POLY.to_vec()).unwrap();
    let both = [0, 1];
    session.doc.project.texture_sets[0].bake.high_poly.match_mode = MeshMatchMode::ByMeshName;
    assert_eq!(session.bake_unpaired_high_parts(&both).unwrap(), Vec::<String>::new(), "the other set matches \"Always\"");

    session.doc.project.texture_sets[1].bake.high_poly.match_mode = MeshMatchMode::ByMeshName;
    assert_eq!(session.bake_unpaired_high_parts(&both).unwrap(), ["Extra_high"]);
}
