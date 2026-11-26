struct QuickMaskParams {
    size: u32,
    inverted: u32,
    erase: u32,
    opacity: f32,
}
@group(0) @binding(0) var material: texture_2d<f32>;
@group(0) @binding(1) var mask: texture_2d<u32>;
@group(0) @binding(2) var brush: texture_2d<u32>;
@group(0) @binding(3) var<uniform> params: QuickMaskParams;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(vec2(i << 1u, i) & vec2(2u));
    return vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<u32>(position.xy);
    let source = textureLoad(material, vec2<i32>(p * textureDimensions(material) / params.size), 0).rgb;
    // BaseColor was sampled from an sRGB texture. The original overlay tints the
    // display-encoded color, before the aux view's ordinary decoding.
    let srgb = select(1.055 * pow(source, vec3(1.0 / 2.4)) - 0.055, source * 12.92, source <= vec3(0.0031308));
    var coverage = f32(textureLoad(mask, vec2<i32>(p * textureDimensions(mask) / params.size), 0).r) / 65535.0;
    if params.opacity > 0.0 {
        let a = f32(textureLoad(brush, vec2<i32>(p * textureDimensions(brush) / params.size), 0).r) * (params.opacity / 65535.0);
        coverage = coverage * (1.0 - a) + select(a, 0.0, params.erase != 0u);
    }
    let blocked = select(1.0 - coverage, coverage, params.inverted != 0u);
    return vec4(mix(srgb, vec3(0.9, 0.1, 0.15), 0.55 * blocked), 1.0);
}
