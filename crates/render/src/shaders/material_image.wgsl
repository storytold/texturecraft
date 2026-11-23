// Authored image and decal descriptors from gpu_image_source.rs. The R32Uint
// array also holds the Tile Generator's grayscale images.
const MISSING_IMAGE: u32 = 0xffffffffu;

fn image_plane_at(plane: u32, pixel: vec2<i32>) -> u32 {
    let origin = vec2<i32>(i32(srgb[plane]), i32(srgb[plane + 1u]));
    return textureLoad(tile_images, origin + pixel, i32(srgb[plane + 2u]), 0).x;
}

fn material_image_texel(image: u32, pixel: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    let at = vec2(wrap_coordinate(pixel.x, size.x), wrap_coordinate(pixel.y, size.y));
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
fn material_image_at(image: u32, uv: vec2<f32>) -> vec4<f32> {
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
    let a = material_image_texel(image, at, size);
    let b = material_image_texel(image, at + vec2(1, 0), size);
    let c = material_image_texel(image, at + vec2(0, 1), size);
    let d = material_image_texel(image, at + vec2(1, 1), size);
    let top = a + (b - a) * f.x;
    let bottom = c + (d - c) * f.x;
    return top + (bottom - top) * f.y;
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

// One stored level of an image for P1: its first plane's origin word, size and encoding
// (gpu_tile_pattern.rs append_rgba)
struct P1ImageLevel {
    planes: u32,
    size: vec2<i32>,
    encoding: u32,
}

fn p1_base_level(image: u32) -> P1ImageLevel {
    return P1ImageLevel(image + 3u, vec2<i32>(i32(srgb[image]), i32(srgb[image + 1u])), srgb[image + 2u] & 255u);
}

// Texel `pixel` of `level`: wrapped on the axes that repeat, the edge texel past the edge on the others
fn p1_level_texel(level: P1ImageLevel, pixel: vec2<i32>, clamps: vec2<bool>) -> vec4<f32> {
    let wrapped = vec2(wrap_coordinate(pixel.x, level.size.x), wrap_coordinate(pixel.y, level.size.y));
    let at = select(wrapped, clamp(pixel, vec2(0), level.size - 1), clamps);
    let first = image_plane_at(level.planes, at);
    switch level.encoding {
        case 1u: { return unpack4x8unorm(first); }
        case 2u: { return vec4(unpack2x16unorm(first), unpack2x16unorm(image_plane_at(level.planes + 3u, at))); }
        default: {
            return bitcast<vec4<f32>>(vec4(first, image_plane_at(level.planes + 3u, at), image_plane_at(level.planes + 6u, at), image_plane_at(level.planes + 9u, at)));
        }
    }
}

// floor(t) as texel indices: huge repeating coordinates reduced by the period, stopped ones kept
// just past the edge, so the integer conversion never overflows
fn p1_texel_index(t: vec2<f32>, size: vec2<i32>, clamps: vec2<bool>) -> vec2<i32> {
    let period = vec2<f32>(size);
    var base = floor(t);
    base = select(base, base % period, abs(base) >= vec2(2147483520.0) & !clamps);
    base = select(base, clamp(base, vec2(-1.0), period), clamps);
    return vec2<i32>(base);
}

// The texel under `uv` (compose ImageRes::nearest)
fn p1_nearest(level: P1ImageLevel, uv: vec2<f32>, clamps: vec2<bool>) -> vec4<f32> {
    return p1_level_texel(level, p1_texel_index(uv * vec2<f32>(level.size), level.size, clamps), clamps);
}

// Bilinear sample of `level` (compose store.rs `bilinear`)
fn p1_bilinear(level: P1ImageLevel, uv: vec2<f32>, clamps: vec2<bool>) -> vec4<f32> {
    let grid = uv * vec2<f32>(level.size) - 0.5;
    let at = p1_texel_index(grid, level.size, clamps);
    let f = grid - floor(grid);
    let a = p1_level_texel(level, at, clamps);
    let b = p1_level_texel(level, at + vec2(1, 0), clamps);
    let c = p1_level_texel(level, at + vec2(0, 1), clamps);
    let d = p1_level_texel(level, at + vec2(1, 1), clamps);
    let top = a + (b - a) * f.x;
    let bottom = c + (d - c) * f.x;
    return top + (bottom - top) * f.y;
}

// The halved levels an image holds besides itself (flag bit 9 of its descriptor, gpu_tile_pattern.rs)
fn p1_level_count(image: u32) -> u32 {
    if (srgb[image + 2u] & P1_IMAGE_LEVELS) == 0u {
        return 0u;
    }
    return srgb[image + 3u + 3u * p1_base_level(image).encoding];
}

// Halved level `index` of an image that holds it (1 is half the size), its plane origins after
// the count: width, height, encoding, then one origin per plane
fn p1_halved_level(image: u32, index: u32) -> P1ImageLevel {
    var word = image + 4u + 3u * p1_base_level(image).encoding;
    for (var level = 1u; level < index; level++) {
        word += 3u + 3u * srgb[word + 2u];
    }
    return P1ImageLevel(word + 3u, vec2<i32>(i32(srgb[word]), i32(srgb[word + 1u])), srgb[word + 2u]);
}

// Level `index` of an image read bilinearly, `full` (the image itself) at 0 and past the last level
fn p1_level_sample(image: u32, index: u32, uv: vec2<f32>, clamps: vec2<bool>, full: vec4<f32>) -> vec4<f32> {
    if index == 0u || index > p1_level_count(image) {
        return full;
    }
    return p1_bilinear(p1_halved_level(image, index), uv, clamps);
}

// An sRGB colour read as linear: the image's own level decodes after its bilinear blend (compose
// ImageRes::sample_srgb_color); alpha stays
fn p1_linear_read(sample: vec4<f32>, linear: bool) -> vec4<f32> {
    if !linear {
        return sample;
    }
    let encoded = clamp(sample.rgb, vec3(0.0), vec3(1.0));
    return vec4(select(pow((encoded + 0.055) / 1.055, vec3(2.4)), encoded / 12.92, encoded <= vec3(0.04045)), sample.a);
}

// P1 (composite_projection.wgsl): the image at a point the fill already projected, read as its
// sampling says (compose ImageRes::sample_filtered). The read's projection.x is its footprint in
// image texels per surface texel, projection.y 1 for an sRGB colour read in linear space.
fn p1_image_at(image: u32, uv: vec2<f32>, layer: Operation) -> vec4<f32> {
    if !all(abs(uv) <= vec2(3.402823466e38)) {
        return vec4(0.0);
    }
    let level = p1_base_level(image);
    let clamps = (vec2(layer.ext.y) & vec2(P1_CLAMP_U, P1_CLAMP_V)) != vec2(0u);
    if (layer.ext.y & P1_NEAREST) != 0u {
        return p1_nearest(level, uv, clamps);
    }
    let full = p1_linear_read(p1_bilinear(level, uv, clamps), layer.projection.y != 0.0);
    let footprint = layer.projection.x;
    if (layer.ext.y & P1_HQ) == 0u || !(footprint > 1.0 && footprint <= 3.402823466e38) {
        return full;
    }

    // Blend the two levels round the footprint's level of detail
    let detail = min(log2(footprint), f32(p1_level_count(image)));
    let finer = u32(floor(detail));
    let toward_coarser = detail - f32(finer);
    let finer_sample = p1_level_sample(image, finer, uv, clamps, full);
    if toward_coarser <= 0.0 {
        return finer_sample;
    }
    let coarser = p1_level_sample(image, finer + 1u, uv, clamps, full);
    return finer_sample + (coarser - finer_sample) * toward_coarser;
}

fn image_projection(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    let image = layer.ids.z;
    let mode = layer.procedural.w;
    if mode == P1_POINT_PROJECTION {
        return p1_image_at(image, uv, layer);
    }
    let p = surface.position * layer.projection.w + layer.projection.xyz;
    if mode == 1u {
        let w = triplanar_weights(surface.normal, layer.balance.w);
        var sampled = vec4(0.0);
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
            let position = surface.position + layer.projection.xyz;
            var dir = vec3(0.0, 1.0, 0.0);
            if dot(position, position) > 0.0 { dir = normalize(position); }
            point = vec2(azimuth(dir.z, dir.x), acos(clamp(dir.y, -1.0, 1.0)) / 3.141592654) * layer.projection.w;
        }
        case 6u, 7u, 8u: {
            let pos = surface.position + layer.projection.xyz;
            let axis = select(select(pos.xyz, pos.yxz, mode == 6u), pos.xzy, mode == 8u);
            point = vec2(azimuth(axis.z, axis.x), 0.5 - axis.y * 0.5) * layer.projection.w;
        }
        case DECAL_PROJECTION: {}
        case UV_TILE_PROJECTION: { point = fract(uv); }
        default: {
            let rotation = layer.projection.xy;
            let rotated = vec2(uv.x * rotation.x - uv.y * rotation.y, uv.x * rotation.y + uv.y * rotation.x);
            point = rotated * layer.projection.zw + layer.uv.xy;
        }
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
    let sampled = image_projection(layer, uv, surface);
    if layer.ids.w == P1_ALPHA_KIND {
        return vec3(sampled.a);
    }
    return decode_image_sample(layer, sampled.rgb);
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
