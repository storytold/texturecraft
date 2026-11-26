// P3a: the generators that read per-texel inputs alone (gpu_generator_p3a.rs): 3D Distance, 3D
// Linear Gradient, Grayscale Conversion and World Space Normals' axes. The kind is in
// procedural.x of the GENERATOR_KIND_OPCODE operation.
const GENERATOR_KIND_OPCODE: u32 = 47u;
const P3A_DISTANCE: u32 = 1u;
const P3A_LINEAR_GRADIENT: u32 = 2u;
const P3A_GRAYSCALE: u32 = 3u;
const P3A_WORLD_SPACE_NORMALS: u32 = 4u;

// Smallest squared start-to-end span the gradient divides by (three_d_distance.rs)
const P3A_MIN_GRADIENT_LENGTH_SQUARED: f32 = 1e-12;
const P3A_MISSING_IMAGE: u32 = 0xffffffffu;

// A histogram scan this narrow is a hard step (f32::EPSILON), a balanced range this narrow too (GeneratorGlobal::MIN_RANGE)
const P3A_MIN_SCAN_WIDTH: f32 = 1.1920929e-7;
const P3A_MIN_BALANCED_RANGE: f32 = 1e-6;

fn generator_p3a_ramp(effect: Operation, texel: LayerTexel) -> f32 {
    switch effect.procedural.x {
        case P3A_DISTANCE: {
            let from_point = length(p3a_uniform_position(effect, texel) - effect.low.xyz);
            let radius = effect.high.x;
            let gradient = select(0.0, clamp(1.0 - from_point / radius, 0.0, 1.0), radius > 0.0);
            return p3a_histogram_scan(gradient, effect.high.y, effect.high.z);
        }
        case P3A_LINEAR_GRADIENT: {
            let span = effect.high.xyz - effect.low.xyz;
            let length_squared = dot(span, span);
            var along = 0.0;
            if length_squared > P3A_MIN_GRADIENT_LENGTH_SQUARED {
                along = dot(p3a_uniform_position(effect, texel) - effect.low.xyz, span) / length_squared;
            }
            return p3a_balanced(clamp(along, 0.0, 1.0), effect.uv.z, effect.uv.w);
        }
        case P3A_GRAYSCALE: {
            return p3a_grayscale(effect, texel);
        }
        case P3A_WORLD_SPACE_NORMALS: {
            var channels = texel.surface.normal * 0.5 + 0.5;
            if effect.effects.w != 0u {
                channels = p3a_input(effect, texel);
            } else if bake_has(BAKED_WORLD_NORMAL) && udim_bake_valid(texel.pixel) {
                channels = bitcast<vec3<f32>>(baked_block_at(texel.pixel, 1u).yzw);
            }
            return p3a_section_adjusted(effect, p3a_axes_signal(effect, channels));
        }
        default: {
            return 0.0;
        }
    }
}

// The replaced input in effects.w, as the CPU's Texel::source reads it
fn p3a_input(effect: Operation, texel: LayerTexel) -> vec3<f32> {
    return fill_value(layers[effect.effects.w - 1u], texel.uv, texel.surface).xyz;
}

// The Position input, else the surface position across the mesh's bounds scaled evenly: the
// longest side spans 0..1 around 0.5 (uniform_position_input in eval.rs; 0.5 where uncovered)
fn p3a_uniform_position(effect: Operation, texel: LayerTexel) -> vec3<f32> {
    if effect.effects.w != 0u {
        return p3a_input(effect, texel);
    }
    return texel.surface.position * effect.uv.x + 0.5;
}

// The histogram scan: a ramp 1 - contrast wide slid through the values (histogram_scan.rs)
fn p3a_histogram_scan(value: f32, position: f32, contrast: f32) -> f32 {
    if position <= 0.0 {
        return 0.0;
    }
    if position >= 1.0 {
        return 1.0;
    }
    let width = 1.0 - clamp(contrast, 0.0, 1.0);
    let low = (1.0 - position) * (1.0 + width) - width;
    if width <= P3A_MIN_SCAN_WIDTH {
        return select(0.0, 1.0, value >= low);
    }
    return clamp((value - low) / width, 0.0, 1.0);
}

// Balance and contrast like GeneratorGlobal::apply
fn p3a_balanced(value: f32, balance: f32, contrast: f32) -> f32 {
    if balance == 0.5 && contrast == 0.0 {
        return value;
    }
    let clamped = clamp(balance, 0.0, 1.0);
    let low = max(1.0 - 2.0 * clamped, 0.0);
    let high = min(2.0 - 2.0 * clamped, 1.0);
    let middle = (low + high) * 0.5;
    let keep = 1.0 - clamp(contrast, 0.0, 1.0);
    let first = middle - (middle - low) * keep;
    let last = middle + (high - middle) * keep;
    if last - first < P3A_MIN_BALANCED_RANGE {
        return select(0.0, 1.0, value >= middle);
    }
    return clamp((value - first) / (last - first), 0.0, 1.0);
}

// The image tiled and turned about the UV square's centre, made gray, balanced and contrasted
// (Texel::grayscale_conversion)
fn p3a_grayscale(effect: Operation, texel: LayerTexel) -> f32 {
    let centred = udim_mesh_uv(texel.uv) - 0.5;
    let turned = vec2(centred.x * effect.high.x - centred.y * effect.high.y, centred.x * effect.high.y + centred.y * effect.high.x);
    let at = turned * effect.high.z + 0.5;
    var rgb = vec3(0.0);
    if effect.procedural.y != 0u {
        let image = srgb[effect.procedural.y - 1u];
        if image != P3A_MISSING_IMAGE {
            rgb = material_image_at(image, at).rgb;
        }
    } else if effect.effects.w != 0u {
        rgb = p3a_input(effect, texel);
    }
    return p3a_balanced(clamp(p3a_gray(u32(effect.high.w), rgb), 0.0, 1.0), effect.uv.z, effect.uv.w);
}

// GeneratorGrayscale::gray, by its index in GeneratorGrayscale::ALL
fn p3a_gray(mode: u32, rgb: vec3<f32>) -> f32 {
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

// The map's channels combined by the axes in their order, top to bottom first (sections.rs axes_signal)
fn p3a_axes_signal(effect: Operation, channels: vec3<f32>) -> f32 {
    var base = p3a_axis(effect, 1u, effect.low.y, effect.high.y, channels.y, 0.0);
    base = p3a_axis(effect, 0u, effect.low.x, effect.high.x, channels.x, base);
    return p3a_axis(effect, 2u, effect.low.z, effect.high.z, channels.z, base);
}

// One axis blended over `base` by its blend (an AxisBlend::ALL index) and weighted
fn p3a_axis(effect: Operation, axis: u32, weight: f32, blend: f32, channel: f32, base: f32) -> f32 {
    if weight <= 0.0 {
        return base;
    }
    let layer = select(channel, 1.0 - channel, ((effect.procedural.y >> axis) & 1u) != 0u);
    var blended = layer;
    switch u32(blend) {
        case 1u: { blended = base + layer; }
        case 2u: { blended = base - layer; }
        case 3u: { blended = base * layer; }
        case 4u: { blended = base + 2.0 * layer - 1.0; }
        case 5u: { blended = max(base, layer); }
        default: {}
    }
    return clamp(base + (blended - base) * weight, 0.0, 1.0);
}

// The section adjustments: the Rust side passes only neutral ones until P1's section path is on the GPU
// TODO(generator_p1): call P1's section path here
fn p3a_section_adjusted(effect: Operation, signal: f32) -> f32 {
    return clamp(signal, 0.0, 1.0);
}
