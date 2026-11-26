// Shared by live material compositing and projected stroke commit.
struct StencilProjection {
    view_proj: mat4x4<f32>,
    center_half: vec4<f32>,
    rotation_flags: vec4<f32>,
}

fn stencil_wrap(coordinate: i32, period: u32) -> i32 {
    let bits = bitcast<u32>(coordinate);
    let negative = coordinate < 0;
    let remainder = select(bits, 0u - bits, negative) % period;
    return i32(select(remainder, period - remainder, negative && remainder != 0u));
}

fn stencil_texel(image: texture_2d<f32>, pixel: vec2<i32>, size: vec2<u32>) -> vec4<f32> {
    return textureLoad(image, vec2(stencil_wrap(pixel.x, size.x), stencil_wrap(pixel.y, size.y)), 0);
}

fn stencil_sample(image: texture_2d<f32>, placement: StencilProjection, position: vec3<f32>) -> vec4<f32> {
    let clip = placement.view_proj * vec4(position, 1.0);
    if clip.w <= 1e-6 {
        return vec4(0.0);
    }
    let d = clip.xy / clip.w - placement.center_half.xy;
    let sn = placement.rotation_flags.x;
    let cs = placement.rotation_flags.y;
    let local = vec2(d.x * cs - d.y * sn, d.x * sn + d.y * cs) / placement.center_half.zw;
    if placement.rotation_flags.z == 0.0 && any(abs(local) > vec2(1.0)) {
        return vec4(0.0);
    }
    let uv = local * vec2(0.5, -0.5) + 0.5;
    let size = textureDimensions(image);
    let grid = uv * vec2<f32>(size) - 0.5;
    if !all(abs(grid) <= vec2(3.402823466e38)) {
        return vec4(0.0);
    }
    let floor_grid = floor(grid);
    // The usual image positions fit in i32. Reduce huge tiled coordinates first
    // so that the signed conversion and neighbouring texel remain in range.
    var base_grid = floor_grid;
    if any(abs(base_grid) >= vec2(2147483520.0)) {
        base_grid = base_grid % vec2<f32>(size);
    }
    let at = vec2<i32>(base_grid);
    let f = grid - floor_grid;
    let a = stencil_texel(image, at, size);
    let b = stencil_texel(image, at + vec2(1, 0), size);
    let c = stencil_texel(image, at + vec2(0, 1), size);
    let e = stencil_texel(image, at + vec2(1, 1), size);
    let top = a + (b - a) * f.x;
    let bottom = c + (e - c) * f.x;
    let rgba = top + (bottom - top) * f.y;
    let alpha = clamp(rgba.w, 0.0, 1.0);
    if alpha == 0.0 {
        return vec4(0.0);
    }
    var color = rgba.rgb;
    if placement.rotation_flags.w != 0.0 {
        let srgb = clamp(color, vec3(0.0), vec3(1.0));
        color = select(pow((srgb + 0.055) / 1.055, vec3(2.4)), srgb / 12.92, srgb <= vec3(0.04045));
    }
    return vec4(color, alpha);
}
