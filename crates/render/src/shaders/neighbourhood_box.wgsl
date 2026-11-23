// P4, the box family and distance filters (gpu_neighbourhood_box.rs): kinds 16..24 (box) and
// 24..32 (distance). Owned by the P4 agent. Always part of the neighbourhood shader; extra
// passes (row and column distance transforms) get their own entry points here, created in
// box_family::Pipelines. Box sums use the 2^24 fixed point of box_blur, as the CPU's i64 sums.

// Whether a P4 kind runs the three box passes first (box_family::uses_box must agree).
fn box_family_uses_box(kind: u32) -> bool {
    return false;
}

// A P4 kind after the box passes: the original and the box-blurred field, unpremultiplied
// by coverage like filtered_box.
fn box_family_filtered(original: vec4<f32>, blurred: vec4<f32>) -> vec4<f32> {
    return blurred;
}

// A P4 kind without the box passes, at absolute texel position point.
fn box_family_gathered(point: vec2<f32>, original: vec4<f32>) -> vec4<f32> {
    return original;
}
