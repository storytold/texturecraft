// Quantize the output of compiled graph nodes to the 16-bit storage steps.
fn compiled_16bit(value: vec2<f32>) -> vec2<u32> {
    let fine = vec2<u32>(clamp(value, vec2(0.0), vec2(1.0)) * 1048576.0);
    // (65535 * fine + 524287) >> 20, split so no intermediate needs more than 32 bits.
    let low = vec2<i32>((fine & vec2<u32>(15u)) << vec2<u32>(16u)) + 524287 - vec2<i32>(fine);
    return vec2<u32>(vec2<i32>(fine >> vec2<u32>(4u)) + (low >> vec2<u32>(20u)));
}
