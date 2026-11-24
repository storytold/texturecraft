// P6's lit normal (gpu_composite_lighting.rs), for Baked Lighting Stylized and Environment

// HEIGHT_RELIEF and MIN_NORMAL_LENGTH_SQUARED of compose's eval.rs
const LIGHTING_HEIGHT_RELIEF: f32 = 0.03125;
const LIGHTING_MIN_NORMAL_LENGTH_SQUARED: f32 = 1e-6;

// The baked height at `pixel` from the bake rows
fn lighting_baked_height(pixel: vec2<u32>) -> f32 {
    return bitcast<f32>(baked_block_at(pixel, 1u).x);
}

// The baked height's change per UV unit along U and V at `pixel`, as the CPU's height_slope
fn lighting_height_slope(pixel: vec2<u32>) -> vec2<f32> {
    let size = params.source.xy;
    let low = max(pixel, vec2(1u)) - 1u;
    let high = min(pixel + 1u, size - 1u);
    let across = lighting_baked_height(vec2(high.x, pixel.y)) - lighting_baked_height(vec2(low.x, pixel.y));
    let down = lighting_baked_height(vec2(pixel.x, high.y)) - lighting_baked_height(vec2(pixel.x, low.y));
    return vec2(across / f32(max(high.x - low.x, 1u)) * f32(size.x), down / f32(max(high.y - low.y, 1u)) * f32(size.y));
}

// The baked world normal (the mesh's without one) tilted along the surface tangent by the baked height's slope times `relief`, as the CPU's detail_normal
fn lighting_detail_normal(texel: LayerTexel, relief: f32) -> vec3<f32> {
    var normal = texel.surface.normal;
    if bake_has(BAKED_WORLD_NORMAL) {
        let detail = bitcast<vec3<f32>>(baked_block_at(texel.pixel, 1u).yzw) * 2.0 - 1.0;
        let length_squared = dot(detail, detail);
        if length_squared > LIGHTING_MIN_NORMAL_LENGTH_SQUARED {
            normal = detail * (1.0 / sqrt(length_squared));
        }
    }
    if !texel.surface.valid || !bake_has(BAKED_HEIGHT) || any(texel.pixel >= params.source.xy) {
        return normal;
    }

    let slope = lighting_height_slope(texel.pixel);
    if !lighting_finite(slope.x) || !lighting_finite(slope.y) {
        return normal;
    }
    let tangent = texel.surface.tangent.xyz;
    let bitangent = cross(normal, tangent) * texel.surface.tangent.w;
    let tilted = normal - (tangent * slope.x + bitangent * slope.y) * LIGHTING_HEIGHT_RELIEF * relief;
    let reciprocal = 1.0 / sqrt(dot(tilted, tilted));
    if lighting_finite(reciprocal) && reciprocal > 0.0 {
        return tilted * reciprocal;
    }
    return normal;
}

// The normal a lighting effect lights: the detail normal at the relief in low.x, or the mesh's
fn lighting_normal(effect: Operation, texel: LayerTexel) -> vec3<f32> {
    if (effect.ids.z & LIGHTING_DETAIL_NORMAL) == 0u {
        return texel.surface.normal;
    }
    return lighting_detail_normal(texel, effect.low.x);
}
