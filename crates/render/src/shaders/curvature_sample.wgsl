// Coarsened curvature deviations sampled within their UV tile block of the bake (the whole bake
// of a classic set), as compose's CurvatureSizeLevel::sample interpolates.
struct CurvatureLayout {
    levels: array<vec4<u32>, 6>, // Atlas origin, step (zero if absent), unused.
    bake_size: vec4<u32>,        // Bake width and height, a UV tile block's width and height.
}

@group(0) @binding(13) var curvature_levels: texture_2d<u32>;
@group(0) @binding(14) var<uniform> curvature_layout: CurvatureLayout;

fn curvature_sample(image: texture_2d<u32>, mapping: CurvatureLayout, baked_pixel: vec2<u32>, level: u32) -> f32 {
    let desc = mapping.levels[level];
    if desc.z == 0u { return 0.0; }
    let step = desc.z;
    let size = (mapping.bake_size.xy + step - 1u) / step;
    let block = (max(mapping.bake_size.zw, vec2(1u)) + step - 1u) / step;
    let first = select(min(baked_pixel / (block * step) * block, size - 1u), vec2(0u), block >= size);
    let last = min(first + block - 1u, size - 1u);
    let position = clamp((vec2<f32>(baked_pixel) + 0.5) / f32(step) - 0.5, vec2<f32>(first), vec2<f32>(last));
    let lo = vec2<u32>(floor(position));
    let hi = min(lo + 1u, last);
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
