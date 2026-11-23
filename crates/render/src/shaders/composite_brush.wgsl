// P7, brush and live stroke on the compositor side (gpu_composite_brush.rs). Owned by the P7
// agent. Compiled into the composite shader when the program needs it, else replaced by the
// stub string in the Rust module (keep both signatures identical).

// The stored paint after the live stroke for a paint operation with P7 flags
// (Operation.ext.z & BRUSH_OP_BITS): m is the stroke's coverage times its opacity at the texel,
// stored the premultiplied stored paint. painted() in composite.wgsl calls it.
fn live_brush_paint(layer: Operation, stored: vec4<f32>, m: f32, projected: ProjectedBrush) -> vec4<f32> {
    return stored;
}
