struct ProjectionVertex {
    position: vec4<f32>,
    normal: vec4<f32>,
}

@group(0) @binding(0) var stroke_alpha: texture_2d<u32>;
@group(0) @binding(1) var projection_uv: texture_2d<u32>;
@group(0) @binding(2) var<storage, read> projection_vertices: array<ProjectionVertex>;
@group(0) @binding(3) var projection_image: texture_2d<f32>;
@group(0) @binding(4) var<uniform> projection_placement: StencilProjection;
@group(0) @binding(5) var<storage, read> projection_tiles: array<vec2<u32>>;
@group(0) @binding(6) var<storage, read_write> projected: array<vec4<f32>>;

fn projection_edge(a: vec2<f32>, b: vec2<f32>, p: vec2<f32>) -> f32 {
    let ab = b - a;
    let ap = p - a;
    return ab.x * ap.y - ab.y * ap.x;
}

// One 64×64 document tile per dispatch layer, sampled only where the stroke has ink.
@compute @workgroup_size(8, 8)
fn read_projection(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = projection_tiles[id.z] + id.xy;
    let index = (id.z * 64u + id.y) * 64u + id.x;
    let coords = vec2<i32>(pixel);
    if textureLoad(stroke_alpha, coords, 0).x == 0u {
        projected[index] = vec4(0.0);
        return;
    }
    let hit = textureLoad(projection_uv, coords, 0).xy;
    if hit.x == 0xffffffffu {
        projected[index] = vec4(0.0);
        return;
    }
    let a = projection_vertices[hit.x * 3u];
    let b = projection_vertices[hit.x * 3u + 1u];
    let c = projection_vertices[hit.x * 3u + 2u];
    // Match the live compositor's reconstruction from unquantized vertex UVs.
    let size = vec2<f32>(textureDimensions(projection_uv));
    let uv_a = vec2(a.position.w, a.normal.w) * size;
    let uv_b = vec2(b.position.w, b.normal.w) * size;
    let uv_c = vec2(c.position.w, c.normal.w) * size;
    let center = vec2<f32>(pixel) + 0.5;
    let area = projection_edge(uv_a, uv_b, uv_c);
    var weights = unpack2x16unorm(hit.y);
    if abs(area) >= 1e-12 {
        weights = vec2(projection_edge(uv_c, uv_a, center), projection_edge(uv_a, uv_b, center)) * (1.0 / area);
    }
    let position = a.position.xyz * (1.0 - weights.x - weights.y) + b.position.xyz * weights.x + c.position.xyz * weights.y;
    projected[index] = stencil_sample(projection_image, projection_placement, position);
}
