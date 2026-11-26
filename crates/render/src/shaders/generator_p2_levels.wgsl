// P2 of the generator GPU port: one micro curvature's detail sizes (a set gpu_curvature.rs built
// from the bake with that micro curvature added) copied into the atlas the composite reads beside
// the bake's own (`generator_level_at` in generator_p2_levels_read.wgsl). The set's levels take
// rows from `params.rows.x`, its layout's words the header row `params.rows.y`, and each present
// level's origin moves down with the set. LAYOUT_VEC4S, the layout's vec4 count, is prepended.

struct Params {
    // The set's first row, its header row
    rows: vec4<u32>,
}

@group(0) @binding(0) var source: texture_2d<u32>;
@group(0) @binding(1) var<uniform> layout_words: array<vec4<u32>, LAYOUT_VEC4S>;
@group(0) @binding(2) var atlas: texture_storage_2d<r32uint, write>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(8, 8)
fn copy_levels(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id.xy >= textureDimensions(source)) {
        return;
    }
    textureStore(atlas, vec2<i32>(id.xy + vec2(0u, params.rows.x)), textureLoad(source, vec2<i32>(id.xy), 0));
}

@compute @workgroup_size(64)
fn write_header(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= LAYOUT_VEC4S * 4u {
        return;
    }
    let entry = layout_words[id.x / 4u];
    var word = entry[id.x % 4u];
    // A present level's origin row moves with the set; the last vec4 is the bake's size
    if id.x / 4u + 1u < LAYOUT_VEC4S && id.x % 4u == 1u && entry.z != 0u {
        word += params.rows.x;
    }
    textureStore(atlas, vec2<i32>(i32(id.x), i32(params.rows.y)), vec4(word, 0u, 0u, 0u));
}
