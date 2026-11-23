// P6, lighting and material-reading effects (gpu_composite_lighting.rs). Owned by the P6 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical).

// A P6 opcode (152..160) on a scalar channel. Masks skip lighting.
fn lighting_scalar_filter(op: Operation, x: f32) -> f32 {
    return x;
}

// A P6 opcode on a content channel; channel is the destination slot (0 Base Color).
fn lighting_effect_color(op: Operation, c: vec3<f32>, channel: u32, surface: Surface, baked_ao: f32) -> vec3<f32> {
    return c;
}
