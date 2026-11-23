// P5, the gather and whole-set filters (gpu_neighbourhood_gather.rs): kinds 32..48 (gather) and
// 48..56 (whole set). Owned by the P5 agent. Always part of the neighbourhood shader; extra
// passes (structure tensor, reductions) get their own entry points here, created in
// gather::Pipelines.

// Whether a P5 kind runs the three box passes first (gather::uses_box must agree).
fn gather_family_uses_box(kind: u32) -> bool {
    return false;
}

// A P5 kind after the box passes.
fn gather_family_filtered(original: vec4<f32>, blurred: vec4<f32>) -> vec4<f32> {
    return blurred;
}

// A P5 kind at absolute texel position point: sample_field and normalized_or read the original
// field, input_at the sixth word.
fn gather_family_gathered(point: vec2<f32>, original: vec4<f32>) -> vec4<f32> {
    return original;
}
