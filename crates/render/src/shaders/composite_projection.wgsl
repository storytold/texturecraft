// P1, fill sampling and projection (gpu_composite_projection.rs). Owned by the P1 agent.
// Compiled into the composite shader when the program needs it, else replaced by the stub
// string in the Rust module (keep both signatures identical).

// A fill whose operation has projection or sampling flags (Operation.ext.y != 0): the value and
// its coverage in alpha. fill_value in composite.wgsl calls it before its own paths.
fn projection_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> {
    return vec4(value(layer, uv, surface), 1.0);
}
