@group(0) @binding(15) var id_bake: texture_2d<u32>;

fn id_word_group(group: u32, width: u32) -> vec4<u32> {
    return textureLoad(id_bake, vec2<i32>(vec2(group % width, group / width)), 0);
}

// The stored float bits are contiguous RGB, packed four words into each GPU texel.
fn id_color_at(pixel: vec2<u32>) -> vec3<f32> {
    let size = params.id_bake.xy;
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy);
    let sample_pixel = min(vec2<u32>(uv * vec2<f32>(size)), size - 1u);
    let word = (sample_pixel.y * size.x + sample_pixel.x) * 3u;
    let group = word / 4u;
    let lane = word % 4u;
    let first = id_word_group(group, size.x);
    if lane == 0u { return bitcast<vec3<f32>>(first.xyz); }
    if lane == 1u { return bitcast<vec3<f32>>(first.yzw); }
    let next = id_word_group(group + 1u, size.x);
    if lane == 2u { return bitcast<vec3<f32>>(vec3(first.zw, next.x)); }
    return bitcast<vec3<f32>>(vec3(first.w, next.xy));
}
