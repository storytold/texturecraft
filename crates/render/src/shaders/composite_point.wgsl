// P2, point filters and gradients (gpu_composite_point.rs). Owned by the P2 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical).

// A P2 opcode (128..144) on a scalar channel or a mask value.
fn point_scalar_filter(op: Operation, x: f32) -> f32 {
    return x;
}

// A P2 opcode on a content channel; channel is the destination slot (0 Base Color, 6 Height,
// 7 Normal, 8..10 colour lobes; channel_slot in Rust).
fn point_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> {
    return c;
}
