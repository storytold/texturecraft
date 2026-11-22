// Authored image and decal descriptors from gpu_image_source.rs. The R32Uint
// array also holds the Tile Generator's grayscale images.
const MISSING_IMAGE: u32 = 0xffffffffu;

fn image_plane_at(plane: u32, pixel: vec2<i32>) -> u32 {
    let origin = vec2<i32>(i32(srgb[plane]), i32(srgb[plane + 1u]));
    return textureLoad(tile_images, origin + pixel, i32(srgb[plane + 2u]), 0).x;
}

fn material_image_texel(image: u32, pixel: vec2<i32>, size: vec2<i32>, repeat: vec2<bool>) -> vec4<f32> {
    let wrapped = vec2(wrap_coordinate(pixel.x, size.x), wrap_coordinate(pixel.y, size.y));
    let at = select(clamp(pixel, vec2(0), size - 1), wrapped, repeat);
    let planes = image + 3u;
    let first = image_plane_at(planes, at);
    switch srgb[image + 2u] & 255u {
        case 1u: { return unpack4x8unorm(first); }
        case 2u: { return vec4(unpack2x16unorm(first), unpack2x16unorm(image_plane_at(planes + 3u, at))); }
        default: {
            return bitcast<vec4<f32>>(vec4(first, image_plane_at(planes + 3u, at), image_plane_at(planes + 6u, at), image_plane_at(planes + 9u, at)));
        }
    }
}

// Match ImageRes::sample: wrap and bilinearly mix the file's encoded samples.
fn material_image_at_with_repeat(image: u32, uv: vec2<f32>, repeat: vec2<bool>) -> vec4<f32> {
    let size = vec2<i32>(i32(srgb[image]), i32(srgb[image + 1u]));
    let grid = uv * vec2<f32>(size) - 0.5;
    if !all(abs(grid) <= vec2(3.402823466e38)) {
        return vec4(0.0);
    }
    let floor_grid = floor(grid);
    var base_grid = floor_grid;
    if any(abs(base_grid) >= vec2(2147483520.0)) {
        base_grid = base_grid % vec2<f32>(size);
    }
    let at = vec2<i32>(base_grid);
    let f = grid - floor_grid;
    let a = material_image_texel(image, at, size, repeat);
    let b = material_image_texel(image, at + vec2(1, 0), size, repeat);
    let c = material_image_texel(image, at + vec2(0, 1), size, repeat);
    let d = material_image_texel(image, at + vec2(1, 1), size, repeat);
    let top = a + (b - a) * f.x;
    let bottom = c + (d - c) * f.x;
    return top + (bottom - top) * f.y;
}

fn material_image_at(image: u32, uv: vec2<f32>) -> vec4<f32> {
    return material_image_at_with_repeat(image, uv, vec2(true));
}

fn decode_image_sample(layer: Operation, rgb: vec3<f32>) -> vec3<f32> {
    switch layer.ids.w {
        case 1u: { return vec3(dot(rgb, vec3(0.3, 0.59, 0.11))); }
        case 2u: { return vec3(dot(rgb, vec3(0.3, 0.59, 0.11)) * 2.0 - 1.0); }
        case 4u: { return vec3(rgb.r, 1.0 - rgb.g, rgb.b); }
        case 0u: {
            if (srgb[layer.ids.z + 2u] & 256u) != 0u {
                let encoded = clamp(rgb, vec3(0.0), vec3(1.0));
                return select(pow((encoded + 0.055) / 1.055, vec3(2.4)), encoded / 12.92, encoded <= vec3(0.04045));
            }
        }
        default: {}
    }
    return rgb;
}

fn image_projection(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    let image = layer.ids.z;
    let mode = layer.procedural.w;
    let p = surface.position * layer.projection.w + layer.projection.xyz;
    if mode == 1u {
        let weights = pow(abs(surface.normal), vec3(layer.balance.w));
        let w = weights / max(weights.x + weights.y + weights.z, 1e-6);
        var sampled = vec4(0.0);
        if measured_blend {
            let position = surface.position * layer.projection.w;
            let offset = layer.projection.xyz;
            let side = select(vec3(1.0), vec3(-1.0), surface.normal < vec3(0.0));
            if w.x > 0.0 { sampled += w.x * material_image_at(image, vec2(offset.z - position.z * side.x, -p.y)); }
            if w.y > 0.0 { sampled += w.y * material_image_at(image, vec2(p.x, offset.z + position.z * side.y)); }
            if w.z > 0.0 { sampled += w.z * material_image_at(image, vec2(offset.x + position.x * side.z, -p.y)); }
            return sampled;
        }
        if w.x > 0.0 { sampled += w.x * material_image_at(image, vec2(p.z, -p.y)); }
        if w.y > 0.0 { sampled += w.y * material_image_at(image, p.xz); }
        if w.z > 0.0 { sampled += w.z * material_image_at(image, vec2(p.x, -p.y)); }
        return sampled;
    }
    var point = uv;
    switch mode {
        case 2u: { point = vec2(p.z, -p.y) * 0.5 + 0.5; }
        case 3u: { point = p.xz * 0.5 + 0.5; }
        case 4u: { point = vec2(p.x, -p.y) * 0.5 + 0.5; }
        case 5u: {
            var dir = vec3(0.0, 1.0, 0.0);
            if dot(surface.position, surface.position) > 0.0 { dir = normalize(surface.position); }
            point = vec2(azimuth(dir.z, dir.x), acos(clamp(dir.y, -1.0, 1.0)) / 3.141592654) * layer.projection.w;
        }
        case 6u, 7u, 8u: {
            let pos = surface.position;
            let axis = select(select(pos.xyz, pos.yxz, mode == 6u), pos.xzy, mode == 8u);
            point = vec2(azimuth(axis.z, axis.x), 0.5 - axis.y * 0.5) * layer.projection.w;
        }
        case DECAL_PROJECTION: {}
        default: { point = transformed_uv(layer, uv); }
    }
    return material_image_at(image, point);
}

fn decal_vector(word: u32) -> vec3<f32> {
    return bitcast<vec3<f32>>(vec3(srgb[word], srgb[word + 1u], srgb[word + 2u]));
}

fn decal_at(frame: u32, position: vec3<f32>, normal: vec3<f32>) -> vec4<f32> {
    let displacement = position - decal_vector(frame);
    let axes = decal_vector(frame + 12u);
    let facing = dot(normal, axes);
    let x = dot(displacement, decal_vector(frame + 4u));
    let y = dot(displacement, decal_vector(frame + 8u));
    let z = abs(dot(displacement, axes));
    let depth = bitcast<f32>(srgb[frame + 15u]);
    if facing <= 0.0 || !all(abs(vec2(x, y)) <= vec2(0.5)) || z > depth {
        return vec4(0.0);
    }
    let flags = srgb[frame + 3u];
    let uv = vec2(x + 0.5, 0.5 - y);
    let flipped = select(uv, 1.0 - uv, (vec2(flags) & vec2(2u, 4u)) != vec2(0u));
    let weight = smoothstep(0.0, 0.3, facing) * (1.0 - smoothstep(0.75 * depth, depth, z));
    return vec4(flipped, weight, 1.0);
}

fn project_decal(frame: u32, surface: Surface) -> vec4<f32> {
    let flags = srgb[frame + 3u];
    if !surface.valid || (flags & 1u) == 0u {
        return vec4(0.0);
    }
    let direct = decal_at(frame, surface.position, surface.normal);
    let axis = (flags >> 3u) & 3u;
    if axis == 0u {
        return direct;
    }
    let mirror = vec3(1u, 2u, 3u) == vec3(axis);
    let plane = bitcast<f32>(srgb[frame + 7u]);
    let position = select(surface.position, 2.0 * plane - surface.position, mirror);
    let normal = select(surface.normal, -surface.normal, mirror);
    let reflected = decal_at(frame, position, normal);
    if reflected.w != 0.0 && (direct.w == 0.0 || reflected.z > direct.z) {
        let flipped = select(reflected.xy, 1.0 - reflected.xy, (vec2(flags) & vec2(32u, 64u)) != vec2(0u));
        return vec4(flipped, reflected.zw);
    }
    return direct;
}

fn authored_image_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec3<f32> {
    if layer.ids.z == MISSING_IMAGE {
        return vec3(0.5);
    }
    return decode_image_sample(layer, image_projection(layer, uv, surface).rgb);
}

fn authored_image_content(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    if layer.ids.z == MISSING_IMAGE {
        return vec4(vec3(0.5), 1.0);
    }
    let sampled = image_projection(layer, uv, surface);
    let alpha = select(0.0, clamp(sampled.a, 0.0, 1.0), abs(sampled.a) <= 3.402823466e38);
    return vec4(decode_image_sample(layer, sampled.rgb), alpha);
}

fn authored_image_placed(layer: Operation, uv: vec2<f32>, repeat: vec2<bool>) -> vec4<f32> {
    if layer.ids.z == MISSING_IMAGE {
        return vec4(vec3(0.5), 1.0);
    }
    let sampled = material_image_at_with_repeat(layer.ids.z, uv, repeat);
    let alpha = select(0.0, clamp(sampled.a, 0.0, 1.0), abs(sampled.a) <= 3.402823466e38);
    return vec4(decode_image_sample(layer, sampled.rgb), alpha);
}

fn decal_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    let frame = u32(layer.uv.x);
    let decal = project_decal(frame, surface);
    if decal.z <= 0.0 {
        return vec4(0.0);
    }
    let alpha_image = srgb[frame + 11u];
    let sample_uv = select(decal.xy, uv, layer.ids.y == 15u);
    if alpha_image != MISSING_IMAGE {
        let sampled = material_image_at(alpha_image, decal.xy);
        let coverage = decal.z * clamp(sampled.a, 0.0, 1.0);
        if layer.ids.y == IMAGE_SOURCE_OPCODE && layer.ids.z == alpha_image {
            return vec4(decode_image_sample(layer, sampled.rgb), coverage);
        }
        return vec4(value(layer, sample_uv, surface), coverage);
    }
    return vec4(value(layer, sample_uv, surface), decal.z);
}
