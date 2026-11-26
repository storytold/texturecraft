// P7, brush and live stroke on the compositor side (gpu_composite_brush.rs). Owned by the P7
// agent. Compiled into the composite shader when the program needs it, else replaced by the
// stub string in the Rust module (keep both signatures identical).

// The stored paint after the live stroke for a paint operation with P7 flags
// (Operation.ext.z & BRUSH_OP_BITS): m is the stroke's coverage times its opacity at the texel,
// stored the premultiplied stored paint, pixel and uv the texel's (uv at the texel centre, as
// MaskGray::at samples). painted() in composite.wgsl calls it.
fn live_brush_paint(layer: Operation, stored: vec4<f32>, m: f32, projected: ProjectedBrush, pixel: vec2<u32>, uv: vec2<f32>) -> vec4<f32> {
    if (layer.ext.z & BRUSH_ERASE) != 0u {
        return erased_paint(layer, stored, m);
    }
    if (layer.ext.z & BRUSH_MASK_IMAGE) != 0u {
        return gray_over(stored, m, mask_image_gray(param_word(layer, 0u), uv));
    }
    return stored;
}

// gpu_composite_brush.rs BRUSH_ERASE and BRUSH_MASK_IMAGE
const BRUSH_ERASE: u32 = 0x01000000u;
const BRUSH_MASK_IMAGE: u32 = 0x02000000u;

// The weights of core's lum, which MaskGray::at reads an image's gray with
const MASK_GRAY_WEIGHTS: vec3<f32> = vec3(0.3, 0.59, 0.11);

// MaskGray::at: the image laid once over the texture, read at the texel centre
fn mask_image_gray(image: u32, uv: vec2<f32>) -> f32 {
    return clamp(dot(image_texel_bilinear_clamped(image, uv).rgb, MASK_GRAY_WEIGHTS), 0.0, 1.0);
}

// The paint crate's mask commit of one gray: premultiplied over in its 16-bit encoding
fn gray_over(stored: vec4<f32>, m: f32, gray: f32) -> vec4<f32> {
    let remaining = 1.0 - m;
    let premultiplied = gray * m + stored.rgb * remaining;
    let quantized = floor(clamp(premultiplied, vec3(0.0), vec3(1.0)) * 65535.0 + 0.5) / 65535.0;
    let coverage = f32(to_u16(m + stored.a * remaining)) / 65535.0;
    return vec4(quantized, coverage);
}

// The paint crate's erase commit: values and coverage scale by 1 - m in their 16-bit encoding
fn erased_paint(layer: Operation, stored: vec4<f32>, m: f32) -> vec4<f32> {
    let remaining = 1.0 - m;
    let signed = layer.ids.x == 6u;
    let scaled = stored.rgb * remaining;
    let encode = select(scaled, scaled * 0.5 + 0.5, signed);
    let quantized = floor(clamp(encode, vec3(0.0), vec3(1.0)) * 65535.0 + 0.5) / 65535.0;
    let rgb = select(quantized, quantized * 2.0 - 1.0, signed);
    let coverage = f32(to_u16(stored.a * remaining)) / 65535.0;
    return vec4(rgb, coverage);
}
