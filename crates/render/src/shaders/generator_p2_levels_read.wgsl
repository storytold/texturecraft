// P2 of the generator GPU port: the curvature detail sizes a generator reads. Its P2 record's
// fifth word names the micro curvature set (its request plus one; 0: the bake's own, the curvature
// levels binding), which the atlas at binding 18 holds: row 0 names each request's header row
// and, in its last texel, the layout's level count; a header row holds the set's layout words.

@group(0) @binding(18) var micro_levels: texture_2d<u32>;

fn p2_levels_word(x: u32, y: u32) -> u32 {
    return textureLoad(micro_levels, vec2<i32>(i32(x), i32(y)), 0).x;
}

// Blurred detail size `level` under bake texel `pixel`, from the generator's curvature set
fn generator_level_at(effect: Operation, pixel: vec2<u32>, level: u32) -> f32 {
    let curvature_set = select(0u, p2_word(effect, 4u), effect.uv.w != 0.0);
    if curvature_set == 0u {
        return curvature_level_at(pixel, level);
    }
    let width = textureDimensions(micro_levels).x;
    let header = p2_levels_word(curvature_set - 1u, 0u);
    let count = p2_levels_word(width - 1u, 0u);
    var mapping = curvature_layout;
    mapping.levels[level] = vec4(
        p2_levels_word(4u * level, header),
        p2_levels_word(4u * level + 1u, header),
        p2_levels_word(4u * level + 2u, header),
        p2_levels_word(4u * level + 3u, header),
    );
    mapping.bake_size = vec4(
        p2_levels_word(4u * count, header),
        p2_levels_word(4u * count + 1u, header),
        p2_levels_word(4u * count + 2u, header),
        p2_levels_word(4u * count + 3u, header),
    );
    return curvature_sample(micro_levels, mapping, pixel, level);
}
