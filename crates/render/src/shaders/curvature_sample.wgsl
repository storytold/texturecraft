// Coarsened curvature deviations sampled within the classic UV bake.
struct CurvatureLayout {
    levels: array<vec4<u32>, 6>, // Atlas origin, step (zero if absent), unused.
    bake_size: vec4<u32>,
}

@group(0) @binding(13) var curvature_levels: texture_2d<u32>;
@group(0) @binding(14) var<uniform> curvature_layout: CurvatureLayout;

fn curvature_sample(image: texture_2d<u32>, mapping: CurvatureLayout, baked_pixel: vec2<u32>, level: u32) -> f32 {
    let desc = mapping.levels[level];
    if desc.z == 0u { return 0.0; }
    let size = (mapping.bake_size.xy + desc.z - 1u) / desc.z;
    let position = clamp((vec2<f32>(baked_pixel) + 0.5) / f32(desc.z) - 0.5, vec2(0.0), vec2<f32>(size - 1u));
    let lo = vec2<u32>(floor(position));
    let hi = min(lo + 1u, size - 1u);
    let blend = position - vec2<f32>(lo);
    let a = f32(bitcast<i32>(textureLoad(image, vec2<i32>(desc.xy + lo), 0).x));
    let b = f32(bitcast<i32>(textureLoad(image, vec2<i32>(desc.xy + vec2(hi.x, lo.y)), 0).x));
    let c = f32(bitcast<i32>(textureLoad(image, vec2<i32>(desc.xy + vec2(lo.x, hi.y)), 0).x));
    let d = f32(bitcast<i32>(textureLoad(image, vec2<i32>(desc.xy + hi), 0).x));
    let top = a + (b - a) * blend.x;
    let bottom = c + (d - c) * blend.x;
    return (top + (bottom - top) * blend.y) / 65534.0;
}

fn curvature_level_at(pixel: vec2<u32>, level: u32) -> f32 {
    return curvature_sample(curvature_levels, curvature_layout, pixel, level);
}
