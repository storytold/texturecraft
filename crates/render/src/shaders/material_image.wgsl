// Authored image and decal descriptors from gpu_image_source.rs. The R32Uint
// array also holds the Tile Generator's grayscale images.
const MISSING_IMAGE: u32 = 0xffffffffu;

fn image_plane_at(plane: u32, pixel: vec2<i32>) -> u32 {
    let origin = vec2<i32>(i32(srgb[plane]), i32(srgb[plane + 1u]));
    return textureLoad(tile_images, origin + pixel, i32(srgb[plane + 2u]), 0).x;
}

fn material_image_texel(image: u32, pixel: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    return material_image_texel_with_repeat(image, pixel, size, vec2(true));
}

// The texel wrapped on the axes that repeat and clamped to the edge on the others
fn material_image_texel_with_repeat(image: u32, pixel: vec2<i32>, size: vec2<i32>, repeat: vec2<bool>) -> vec4<f32> {
    let wrapped = vec2(wrap_coordinate(pixel.x, size.x), wrap_coordinate(pixel.y, size.y));
    let at = select(clamp(pixel, vec2(0), size - 1), wrapped, repeat);
    let planes = image + 3u;
    let first = image_plane_at(planes, at);
    switch srgb[image + 2u] & 255u {
        case 1u: { return unpack4x8unorm(first); }
        case 2u: { return vec4(unpack2x16unorm(first), unpack2x16unorm(image_plane_at(planes + 3u, at))); }
        case 5u: { return vec4(vec3(f32(first) / 65535.0), 1.0); }
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
    let a = material_image_texel_with_repeat(image, at, size, repeat);
    let b = material_image_texel_with_repeat(image, at + vec2(1, 0), size, repeat);
    let c = material_image_texel_with_repeat(image, at + vec2(0, 1), size, repeat);
    let d = material_image_texel_with_repeat(image, at + vec2(1, 1), size, repeat);
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
        case 9u: { return vec3(rgb.r); }
        case 10u: { return vec3(rgb.r * 2.0 - 1.0); }
        case 11u: {
            let xy = vec2(rgb.r, 1.0 - rgb.g) * 2.0 - 1.0;
            return vec3(rgb.r, 1.0 - rgb.g, 0.5 + 0.5 * sqrt(max(1.0 - dot(xy, xy), 0.0)));
        }
        case 0u, 8u: {
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
    if mode == P1_POINT_PROJECTION {
        return p1_image_at(image, uv, layer);
    }

    let p = surface.position * layer.projection.w + layer.projection.xyz;
    if mode == 1u {
        // The three planes through one material_image_at call (the driver inlines each call site)
        let w = triplanar_weights(surface.normal, layer.balance.w);
        var points = array<vec2<f32>, 3>(vec2(p.z, -p.y), p.xz, vec2(p.x, -p.y));
        if measured_blend {
            // Measured rules mirror each plane by the side the normal faces
            let position = surface.position * layer.projection.w;
            let offset = layer.projection.xyz;
            let side = select(vec3(1.0), vec3(-1.0), surface.normal < vec3(0.0));
            points = array<vec2<f32>, 3>(
                vec2(offset.z - position.z * side.x, -p.y),
                vec2(p.x, offset.z + position.z * side.y),
                vec2(offset.x + position.x * side.z, -p.y),
            );
        }
        var sampled = vec4(0.0);
        for (var plane = 0u; plane < 3u; plane++) {
            if w[plane] > 0.0 { sampled += w[plane] * material_image_at(image, points[plane]); }
        }
        return sampled;
    }
    var point = uv;
    switch mode {
        case 2u: { point = vec2(p.z, -p.y) * 0.5 + 0.5; }
        case 3u: { point = p.xz * 0.5 + 0.5; }
        case 4u: { point = vec2(p.x, -p.y) * 0.5 + 0.5; }
        case 5u: {
            let position = surface.position + layer.projection.xyz;
            var dir = vec3(0.0, 1.0, 0.0);
            if dot(position, position) > 0.0 { dir = normalize(position); }
            point = vec2(azimuth(dir.z, dir.x), polar_angle(dir.y) / 3.141592654) * layer.projection.w;
        }
        case 6u, 7u, 8u: {
            let pos = surface.position + layer.projection.xyz;
            let axis = select(select(pos.xyz, pos.yxz, mode == 6u), pos.xzy, mode == 8u);
            point = vec2(azimuth(axis.z, axis.x), 0.5 - axis.y * 0.5) * layer.projection.w;
        }
        case DECAL_PROJECTION: {}
        case UV_TILE_PROJECTION: { point = fract(uv); }
        default: { point = transformed_uv(layer, uv); }
    }
    return material_image_at(image, point);
}

fn placed_image_coord(frame: u32, p: vec2<f32>) -> vec2<f32> {
    let image_x = bitcast<vec2<f32>>(vec2(srgb[frame + 12u], srgb[frame + 13u]));
    let image_y = bitcast<vec2<f32>>(vec2(srgb[frame + 14u], srgb[frame + 15u]));
    let centered = p - bitcast<vec2<f32>>(vec2(srgb[frame + 16u], srgb[frame + 17u]));
    return vec2(dot(centered, image_x), dot(centered, image_y)) + 0.5;
}

fn triplanar_sample(image: u32, frame: u32, p: vec3<f32>, n: vec3<f32>, weights: vec3<f32>) -> vec4<f32> {
    let side = select(vec3(1.0), vec3(-1.0), n < vec3(0.0));
    // The three planes through one material_image_at call (the driver inlines each call site)
    let points = array<vec2<f32>, 3>(vec2(-p.z * side.x, -p.y), vec2(p.x, p.z * side.y), vec2(p.x * side.z, -p.y));
    var result = vec4(0.0);
    for (var plane = 0u; plane < 3u; plane++) {
        if weights[plane] > 0.0 { result += weights[plane] * material_image_at(image, placed_image_coord(frame, points[plane])); }
    }
    return result;
}

fn placed_volume_position(frame: u32, position: vec3<f32>) -> vec3<f32> {
    return vec3(dot(decal_vector(frame), position), dot(decal_vector(frame + 4u), position), dot(decal_vector(frame + 8u), position))
        + bitcast<vec3<f32>>(vec3(srgb[frame + 3u], srgb[frame + 7u], srgb[frame + 11u]));
}

fn authored_image_warp(layer: Operation, surface: Surface, with_alpha: bool) -> vec4<f32> {
    if !surface.valid { return vec4(0.0); }
    let frame = u32(layer.uv.x);
    let p = placed_volume_position(frame, surface.position);
    let data = srgb[frame + 22u];
    if bitcast<f32>(srgb[frame + 19u]) != 0.0 && any(abs(p) > vec3(0.5)) {
        return vec4(0.0);
    }
    var image_coord = clamp(vec2(p.x, -p.y), vec2(-0.5), vec2(0.5));
    var depth = 0.0;
    if data == 0u {
        if any(abs(p.xy) > vec2(0.5000005)) { return vec4(0.0); }
    } else {
        let hit = warp_inverse(data, 2.0 * p.xy);
        if hit.w == 0.0 { return vec4(0.0); }
        image_coord = hit.xy - 0.5;
        depth = hit.z;
    }
    let z = abs(2.0 * p.z - depth);
    let reach = bitcast<f32>(srgb[frame + 18u]);
    if z > reach { return vec4(0.0); }
    var coverage = 1.0;
    if bitcast<f32>(srgb[frame + 20u]) != 0.0 {
        let edge = bitcast<f32>(srgb[frame + 21u]) * reach;
        if edge < reach { coverage = 1.0 - smoothstep(edge, reach, z); }
        else { coverage = select(0.0, 1.0, z < edge); }
    }
    if coverage <= 0.0 { return vec4(0.0); }
    let point = placed_image_coord(frame, image_coord);
    let repeat = (vec2(layer.procedural.z) & vec2(1u, 2u)) != vec2(0u);
    if !placed_uv_covered(point, repeat) { return vec4(0.0); }
    if layer.ids.y != IMAGE_SOURCE_OPCODE { return vec4(layer.low.rgb, coverage); }
    let sample = authored_image_placed(layer, point, repeat);
    return vec4(sample.rgb, coverage * select(1.0, sample.a, with_alpha));
}

fn authored_image_triplanar(layer: Operation, surface: Surface, with_alpha: bool) -> vec4<f32> {
    if !surface.valid { return vec4(0.0); }
    let frame = u32(layer.uv.x);
    let x = decal_vector(frame);
    let y = decal_vector(frame + 4u);
    let z = decal_vector(frame + 8u);
    let p = placed_volume_position(frame, surface.position);
    if bitcast<f32>(srgb[frame + 19u]) != 0.0 && any(abs(p) > vec3(0.5)) { return vec4(0.0); }
    if layer.ids.y != IMAGE_SOURCE_OPCODE { return vec4(layer.low.rgb, 1.0); }
    let image = layer.ids.z;
    if image == MISSING_IMAGE { return vec4(vec3(0.5), 1.0); }
    let n = vec3(dot(normalize(x), surface.normal), dot(normalize(y), surface.normal), dot(normalize(z), surface.normal));
    let magnitude = abs(n);
    let largest = max(max(magnitude.x, magnitude.y), magnitude.z);
    let edge = bitcast<f32>(srgb[frame + 18u]);
    var weights = vec3(0.0, 0.0, 1.0);
    if largest > 1e-12 {
        let share = magnitude / largest;
        if edge >= 1.0 { weights = select(vec3(0.0), vec3(1.0), share >= vec3(1.0)); }
        else { weights = smoothstep(vec3(edge), vec3(1.0), share); }
        weights /= max(weights.x + weights.y + weights.z, 1e-12);
    }
    // The image, then a graph's matte, through one triplanar_sample call
    let matte = with_alpha && is_graph_image(layer.ids.w) && layer.procedural.y > 0u;
    var sample = vec4(0.0);
    var alpha = 1.0;
    for (var read = 0u; read < select(1u, 2u, matte); read++) {
        let sampled = triplanar_sample(select(image, layer.procedural.y - 1u, read == 1u), frame, p, n, weights);
        if read == 0u {
            sample = sampled;
            if with_alpha && !is_graph_image(layer.ids.w) { alpha = sampled.a; }
        } else {
            alpha = sampled.r;
        }
    }
    return vec4(decode_image_sample(layer, sample.rgb), clamp(alpha, 0.0, 1.0));
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
    let sampled = image_projection(layer, uv, surface);
    if layer.ids.w == P1_ALPHA_KIND {
        return vec3(sampled.a);
    }

    return decode_image_sample(layer, sampled.rgb);
}

fn authored_image_content(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    if layer.ids.z == MISSING_IMAGE {
        return vec4(vec3(0.5), 1.0);
    }
    // The image, then a graph's matte, through one image_projection call
    let matte = is_graph_image(layer.ids.w) && layer.procedural.y > 0u;
    var sampled = vec4(0.0);
    var alpha = 1.0;
    for (var read = 0u; read < select(1u, 2u, matte); read++) {
        var source = layer;
        if read == 1u { source.ids.z = layer.procedural.y - 1u; }
        let projected = image_projection(source, uv, surface);
        if read == 0u {
            sampled = projected;
            alpha = select(projected.a, select(1.0, projected.a, layer.procedural.x == 2u), is_graph_image(layer.ids.w));
        } else {
            alpha = projected.r;
        }
    }
    alpha = select(0.0, clamp(alpha, 0.0, 1.0), abs(alpha) <= 3.402823466e38);
    return vec4(decode_image_sample(layer, sampled.rgb), alpha);
}

fn authored_image_placed(layer: Operation, uv: vec2<f32>, repeat: vec2<bool>) -> vec4<f32> {
    if layer.ids.z == MISSING_IMAGE {
        return vec4(vec3(0.5), 1.0);
    }
    // The image, then a graph's matte, through one material_image_at_with_repeat call
    let matte = is_graph_image(layer.ids.w) && layer.procedural.y > 0u;
    var sampled = vec4(0.0);
    var alpha = 1.0;
    for (var read = 0u; read < select(1u, 2u, matte); read++) {
        let image = select(layer.ids.z, layer.procedural.y - 1u, read == 1u);
        let read_sample = material_image_at_with_repeat(image, uv, repeat);
        if read == 0u {
            sampled = read_sample;
            alpha = select(read_sample.a, 1.0, is_graph_image(layer.ids.w));
        } else {
            alpha = read_sample.r;
        }
    }
    alpha = select(0.0, clamp(alpha, 0.0, 1.0), abs(alpha) <= 3.402823466e38);
    return vec4(decode_image_sample(layer, sampled.rgb), alpha);
}

fn decal_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    let frame = u32(layer.uv.x);
    let decal = project_decal(frame, surface);
    if decal.z <= 0.0 {
        return vec4(0.0);
    }
    let alpha_image = srgb[frame + 11u];
    var gain = decal.z;
    if layer.ids.y == IMAGE_SOURCE_OPCODE && is_graph_image(layer.ids.w) && layer.procedural.y > 0u {
        gain *= clamp(material_image_at(layer.procedural.y - 1u, decal.xy).r, 0.0, 1.0);
    }
    let sample_uv = select(decal.xy, uv, layer.ids.y == 15u);
    if alpha_image != MISSING_IMAGE {
        let sampled = material_image_at(alpha_image, decal.xy);
        let coverage = gain * clamp(sampled.a, 0.0, 1.0);
        if layer.ids.y == IMAGE_SOURCE_OPCODE && layer.ids.z == alpha_image {
            return vec4(decode_image_sample(layer, sampled.rgb), coverage);
        }
        return vec4(value(layer, sample_uv, surface), coverage);
    }
    return vec4(value(layer, sample_uv, surface), gain);
}
