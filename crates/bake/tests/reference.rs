//! The analytic reference scenes on the CPU tracer (`reference_scenes` has the derivations)

mod reference_scenes;

use texturecraft_bake::tracing::CpuTracer;

/// Run the reference check `name` on the CPU tracer
fn run(name: &str) {
    let (_, check) = reference_scenes::CHECKS.iter().find(|(check, _)| *check == name).unwrap();
    check(&CpuTracer);
}

/// The `occlusion_beside_a_wall` reference scene on the CPU tracer
#[test]
fn occlusion_beside_a_wall() {
    run("occlusion_beside_a_wall");
}

/// The `occlusion_inside_an_open_box` reference scene on the CPU tracer
#[test]
fn occlusion_inside_an_open_box() {
    run("occlusion_inside_an_open_box");
}

/// The `height_of_a_step` reference scene on the CPU tracer
#[test]
fn height_of_a_step() {
    run("height_of_a_step");
}

/// The `height_of_a_spherical_cap` reference scene on the CPU tracer
#[test]
fn height_of_a_spherical_cap() {
    run("height_of_a_spherical_cap");
}

/// The `tangent_normal_of_a_tilted_plane` reference scene on the CPU tracer
#[test]
fn tangent_normal_of_a_tilted_plane() {
    run("tangent_normal_of_a_tilted_plane");
}

/// The `world_normal_and_position` reference scene on the CPU tracer
#[test]
fn world_normal_and_position() {
    run("world_normal_and_position");
}

/// The `thickness_of_a_slab` reference scene on the CPU tracer
#[test]
fn thickness_of_a_slab() {
    run("thickness_of_a_slab");
}

/// The `opacity_of_a_hole` reference scene on the CPU tracer
#[test]
fn opacity_of_a_hole() {
    run("opacity_of_a_hole");
}

/// The `normal_map_round_trip` reference scene on the CPU tracer
#[test]
fn normal_map_round_trip() {
    run("normal_map_round_trip");
}

/// The `normal_map_round_trip_with_mirrored_uvs` reference scene on the CPU tracer
#[test]
fn normal_map_round_trip_with_mirrored_uvs() {
    run("normal_map_round_trip_with_mirrored_uvs");
}

/// The `padding_fills_each_gutter_from_the_nearest_island` reference scene on the CPU tracer
#[test]
fn padding_fills_each_gutter_from_the_nearest_island() {
    run("padding_fills_each_gutter_from_the_nearest_island");
}

/// The `tangent_normals_agree_across_a_uv_seam` reference scene on the CPU tracer
#[test]
fn tangent_normals_agree_across_a_uv_seam() {
    run("tangent_normals_agree_across_a_uv_seam");
}

/// Texels on an island's border average only the subsamples the island covers
#[test]
fn partly_covered_texels_average_only_their_covered_subsamples() {
    run("partly_covered_texels_average_only_their_covered_subsamples");
}

/// The mixed-sign reference scene on the CPU tracer
#[test]
#[ignore = "fails until the per-corner tangent sign fix: compute_tangents keeps one tangent per vertex, so mirrored UVs meeting at a vertex mix bitangent signs in a triangle"]
fn mixed_tangent_signs_decode_like_the_viewport() {
    reference_scenes::mixed_tangent_signs_decode_like_the_viewport(&CpuTracer);
}
