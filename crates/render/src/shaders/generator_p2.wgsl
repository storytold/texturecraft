// P2 of the generator GPU port (gpu_generator_p2.rs): what a generator's Micro Details add to its
// curvature and occlusion, read from the maps generator_p2_micro.wgsl builds. A generator's P2
// record sits in the side table at `uv.w` (0: none): its micro curvature layer plus one and its
// micro occlusion layer plus one (0: none), then its texture and grunge records (0: none).

@group(0) @binding(16) var micro_maps: texture_2d_array<f32>;

fn p2_word(effect: Operation, word: u32) -> u32 {
    return srgb[u32(effect.uv.w) + word];
}

// A micro map's value under working texel `pixel`, at the texel of its bake layout the texel's
// bake UV falls in (LayoutMap::at); the map's own row count sits in the row after the tallest map
fn micro_map_at(layer: u32, pixel: vec2<u32>) -> f32 {
    let extent = textureDimensions(micro_maps);
    let rows = u32(textureLoad(micro_maps, vec2<i32>(0, i32(extent.y - 1u)), i32(layer), 0).x);
    let size = vec2(extent.x, max(rows, 1u));
    let uv = udim_bake_uv((vec2<f32>(pixel) + 0.5) / vec2<f32>(params.source.xy)).xy;
    let at = min(vec2<u32>(uv * vec2<f32>(size)), size - 1u);
    return textureLoad(micro_maps, vec2<i32>(at), i32(layer), 0).x;
}

// The baked occlusion (x) and curvature (y) at `pixel` with what the generator's micro details
// add: the curvature term added, the occlusion multiplied (curvature_input, occlusion_input)
fn generator_micro(effect: Operation, pixel: vec2<u32>, baked: vec3<f32>) -> vec3<f32> {
    if effect.uv.w == 0.0 || !udim_bake_valid(pixel) {
        return baked;
    }
    var adjusted = baked;
    let curvature = p2_word(effect, 0u);
    if curvature != 0u {
        adjusted.y += micro_map_at(curvature - 1u, pixel);
    }
    let occlusion = p2_word(effect, 1u);
    if occlusion != 0u {
        adjusted.x *= micro_map_at(occlusion - 1u, pixel);
    }
    return adjusted;
}

// An image slot's record (p2 slot_words in Rust): the side-table word holding its image's record
// (0: a uniform colour), the uniform colour, the scale and offset it is read at, triplanar (1) or
// in UV (0), the triplanar contrast, and whether the X projection mirrors about the centre
const P2_SLOT_IMAGE: u32 = 0u;
const P2_SLOT_COLOR: u32 = 1u;
const P2_SLOT_SCALE: u32 = 4u;
const P2_SLOT_OFFSET: u32 = 5u;
const P2_SLOT_TRIPLANAR: u32 = 6u;
const P2_SLOT_CONTRAST: u32 = 7u;
const P2_SLOT_MIRRORED: u32 = 8u;
const P2_SLOT_WORDS: u32 = 9u;
// The texture record after its slot: grayscale mode, invert, contrast, brightness, blend, opacity
const P2_TEXTURE_GRAY: u32 = 9u;
const P2_TEXTURE_INVERT: u32 = 10u;
const P2_TEXTURE_CONTRAST: u32 = 11u;
const P2_TEXTURE_BRIGHTNESS: u32 = 12u;
const P2_TEXTURE_BLEND: u32 = 13u;
const P2_TEXTURE_OPACITY: u32 = 14u;
// The grunge record: its mode (0 the wear's own pattern, 1 a custom image), then the pattern's
// source operation and its breakup's (plus one; 0: none), mean and spread, or the custom slot
// and its gain
const P2_GRUNGE_MODE: u32 = 0u;
const P2_GRUNGE_PATTERN: u32 = 1u;
const P2_GRUNGE_BREAKUP: u32 = 2u;
const P2_GRUNGE_MEAN: u32 = 3u;
const P2_GRUNGE_SPREAD: u32 = 4u;
const P2_GRUNGE_SLOT: u32 = 1u;
const P2_GRUNGE_GAIN: u32 = 10u;
// How much a breakup pattern varies the grunge's strength (compose's BREAKUP_DEPTH)
const P2_BREAKUP_DEPTH: f32 = 0.6;
// The gray at which a custom grunge neither adds nor removes wear
const P2_CUSTOM_GRUNGE_MID_GRAY: f32 = 0.5;
// Exponent of the triplanar weights per unit of 1 / (1 − contrast), and the smallest normal
// component that still divides them (compose's texture_layer.rs)
const P2_TRIPLANAR_SHARPNESS_POWER: f32 = 1.58;
const P2_MIN_NORMAL_COMPONENT: f32 = 1e-6;
const P2_MIN_TRIPLANAR_CONTRAST: f32 = 0.001;
const P2_FLOAT_EPSILON: f32 = 1.1920929e-7;

fn p2_float(record: u32, word: u32) -> f32 {
    return bitcast<f32>(srgb[record + word]);
}

// How much each projection (facing X, Y and Z) counts for a surface facing `normal`
// (texture_layer::triplanar_weights)
fn p2_triplanar_weights(contrast: f32, normal: vec3<f32>) -> vec3<f32> {
    let clamped = clamp(contrast, P2_MIN_TRIPLANAR_CONTRAST, 1.0);
    let sharpness = pow(max(1.0 - clamped, P2_FLOAT_EPSILON), -P2_TRIPLANAR_SHARPNESS_POWER);
    let facing = abs(normal);
    let largest = max(max(facing.x, max(facing.y, facing.z)), P2_MIN_NORMAL_COMPONENT);
    let weights = pow(facing / largest, vec3(sharpness));
    let total = weights.x + weights.y + weights.z;
    return select(vec3(1.0 / 3.0), weights / total, total > 0.0);
}

// The colour of the image slot at `record` (Texel::slot_color): its uniform colour, or its image
// read where the slot places it, in UV or from the mesh's three axes; `found` false where the
// image is not loaded. `position` is the texel across the mesh's bounds, `normal` the
// generator's world normal.
fn p2_slot_color(record: u32, uv: vec2<f32>, position: vec3<f32>, normal: vec3<f32>, found: ptr<function, bool>) -> vec3<f32> {
    *found = true;
    let image_word = srgb[record + P2_SLOT_IMAGE];
    if image_word == 0u {
        return bitcast<vec3<f32>>(vec3(srgb[record + P2_SLOT_COLOR], srgb[record + P2_SLOT_COLOR + 1u], srgb[record + P2_SLOT_COLOR + 2u]));
    }
    let image = srgb[image_word];
    if image == MISSING_IMAGE {
        *found = false;
        return vec3(0.0);
    }
    let scale = p2_float(record, P2_SLOT_SCALE);
    let offset = p2_float(record, P2_SLOT_OFFSET);
    if srgb[record + P2_SLOT_TRIPLANAR] == 0u {
        return material_image_at(image, uv * scale + offset).rgb;
    }
    let weights = p2_triplanar_weights(p2_float(record, P2_SLOT_CONTRAST), normal);
    let mirrored_z = select(-position.z, 1.0 - position.z, srgb[record + P2_SLOT_MIRRORED] != 0u);
    var planes = array(vec2(mirrored_z, position.y), vec2(position.x, position.z), vec2(position.x, position.y));
    var color = vec3(0.0);
    for (var axis = 0u; axis < 3u; axis++) {
        color += material_image_at(image, planes[axis] * scale + offset).rgb * weights[axis];
    }
    return color;
}

// A colour's gray by a generator grayscale mode (GeneratorGrayscale::gray)
fn p2_gray(mode: u32, rgb: vec3<f32>) -> f32 {
    switch mode {
        case 0u: { return (max(max(rgb.r, rgb.g), rgb.b) + min(min(rgb.r, rgb.g), rgb.b)) * 0.5; }
        case 1u: { return rgb.r * 0.299 + rgb.g * 0.587 + rgb.b * 0.114; }
        case 2u: { return (rgb.r + rgb.g + rgb.b) / 3.0; }
        case 3u: { return max(max(rgb.r, rgb.g), rgb.b); }
        case 4u: { return min(min(rgb.r, rgb.g), rgb.b); }
        case 5u: { return rgb.r; }
        case 6u: { return rgb.g; }
        default: { return rgb.b; }
    }
}

// The texture's gray from its colour: converted, inverted, contrasted, then brightened
// (texture_layer::texture_gray)
fn p2_texture_gray(record: u32, rgb: vec3<f32>) -> f32 {
    var gray = clamp(p2_gray(srgb[record + P2_TEXTURE_GRAY], rgb), 0.0, 1.0);
    if srgb[record + P2_TEXTURE_INVERT] != 0u {
        gray = 1.0 - gray;
    }
    let contrast = p2_float(record, P2_TEXTURE_CONTRAST);
    if contrast > 0.0 {
        gray = clamp((gray - 0.5) / max(1.0 - contrast, P2_FLOAT_EPSILON) + 0.5, 0.0, 1.0);
    } else {
        gray = (gray - 0.5) * (1.0 + contrast) + 0.5;
    }
    let brightness = p2_float(record, P2_TEXTURE_BRIGHTNESS);
    return select(gray * (1.0 + brightness), gray + brightness * (1.0 - gray), brightness > 0.0);
}

// The texture's `gray` blended over `signal` by its mode and opacity (texture_layer::blended_over)
fn p2_blended_over(record: u32, signal: f32, gray: f32) -> f32 {
    var blend = gray;
    switch srgb[record + P2_TEXTURE_BLEND] {
        case 1u: { blend = signal + gray; }
        case 2u: { blend = signal - gray; }
        case 3u: { blend = signal * gray; }
        case 4u: { blend = signal + 2.0 * gray - 1.0; }
        case 5u: { blend = max(signal, gray); }
        case 6u: { blend = min(signal, gray); }
        case 7u: { blend = select(1.0 - 2.0 * (1.0 - signal) * (1.0 - gray), 2.0 * signal * gray, signal < 0.5); }
        case 8u: { blend = 1.0 - (1.0 - signal) * (1.0 - gray); }
        case 9u: {
            if gray < 0.5 {
                blend = 2.0 * signal * gray + signal * signal * (1.0 - 2.0 * gray);
            } else {
                blend = 2.0 * signal * (1.0 - gray) + sqrt(max(signal, 0.0)) * (2.0 * gray - 1.0);
            }
        }
        default: {}
    }
    return clamp(signal + (blend - signal) * p2_float(record, P2_TEXTURE_OPACITY), 0.0, 1.0);
}

// The generator's texture blended over its adjusted `signal`, before the global blur (Texel::
// generator); the signal itself without a texture. `position` is the texel across the mesh's
// bounds and `normal` the generator's world normal, which a triplanar texture reads.
fn generator_texture(effect: Operation, texel: LayerTexel, signal: f32, position: vec3<f32>, normal: vec3<f32>) -> f32 {
    let record = select(0u, p2_word(effect, 2u), effect.uv.w != 0.0);
    if record == 0u {
        return signal;
    }
    var found = true;
    let rgb = p2_slot_color(record, texel.uv, position, normal, &found);
    // An image still loading reads black
    let gray = select(0.0, p2_texture_gray(record, rgb), found);
    return p2_blended_over(record, signal, gray);
}

// A wear generator's grunge at the texel, normalised to mean 0 and spread 1 (wear::grunge), or
// its custom grunge image's gray as that grunge (wear::custom_grunge); 0 without a grunge record
fn generator_grunge(effect: Operation, texel: LayerTexel, position: vec3<f32>, normal: vec3<f32>) -> f32 {
    let record = select(0u, p2_word(effect, 3u), effect.uv.w != 0.0);
    if record == 0u {
        return 0.0;
    }
    if srgb[record + P2_GRUNGE_MODE] != 0u {
        var found = true;
        let rgb = p2_slot_color(record + P2_GRUNGE_SLOT, texel.uv, position, normal, &found);
        // An image not loaded yet reads mid gray, which leaves the wear as without grunge
        let gray = select(P2_CUSTOM_GRUNGE_MID_GRAY, luminance(rgb), found);
        return p2_float(record, P2_GRUNGE_GAIN) * (gray - P2_CUSTOM_GRUNGE_MID_GRAY);
    }
    var value = fill_value(layers[srgb[record + P2_GRUNGE_PATTERN] - 1u], texel.uv, texel.surface).x;
    let breakup = srgb[record + P2_GRUNGE_BREAKUP];
    if breakup != 0u {
        value *= 1.0 - P2_BREAKUP_DEPTH + P2_BREAKUP_DEPTH * fill_value(layers[breakup - 1u], texel.uv, texel.surface).x;
    }
    return (value - p2_float(record, P2_GRUNGE_MEAN)) / p2_float(record, P2_GRUNGE_SPREAD);
}
