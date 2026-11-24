//! P7, brush and live stroke (plan section 2.6), the compositor side. Owned by the P7 agent; no
//! other package edits this file or `shaders/composite_brush.wgsl`.
//!
//! On the GPU brush: Paint, the Eraser on layers and masks (the ordinary eraser, flagged with
//! `BRUSH_ERASE`), the Projection tool in both views, symmetry copies in both views, and one
//! device stroke feeding every texture set of its size that draws the layer. Pickup strokes
//! (Smudge, Clone, Paint with a material pattern) rasterise on the CPU and composite live on the
//! GPU from `display_stroke_pixels`. Still on the CPU: mask strokes painting a mask image (the
//! live gray needs the texel in `live_brush_paint`), and strokes with a brush alpha image or a
//! library tip.
//!
//! P7 also owns `gpu_brush.rs`, `shaders/brush.wgsl`, crates/engine/src/painting.rs,
//! crates/engine/src/brush_device.rs, `LiveBrush` and `content_paint` in gpu_composite.rs, and
//! the stroke part of `SetCompositor::plan` (the lines before `view_project`).
//!
//! How: a paint operation with a live stroke mode sets `op.ext[2] & BRUSH_OP_BITS`;
//! `painted()` in composite.wgsl then calls `live_brush_paint` (this WGSL file) instead of the
//! premultiplied over. Keep [`uses_shader`] true for every such program.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] (no library filter is P7's). The brush cases live in
//! crates/engine and crates/ui-egui tests (see `gpu.rs` mask image stroke test); every port gets
//! a GPU-versus-CPU stroke test during the stroke and after release.

use super::CompositeProgram;
use super::packages::BRUSH_OP_BITS;

/// Library filter ids P7 still leaves on the CPU (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

/// The `ext[2]` bit (one of [`BRUSH_OP_BITS`]) of a live paint operation whose stroke erases;
/// `BRUSH_ERASE` in WGSL
const BRUSH_ERASE: u32 = 0x0100_0000;

/// The P7 bits of a live paint operation's `ext[2]`: zero leaves the premultiplied over of
/// `painted()` in composite.wgsl
pub(super) fn live_paint_bits(erase: bool) -> u32 {
    if erase { BRUSH_ERASE } else { 0 }
}

pub(super) const WGSL: &str = include_str!("shaders/composite_brush.wgsl");
/// The same functions without P7's code, for programs that do not use it.
pub(super) const WGSL_STUB: &str =
    "fn live_brush_paint(layer: Operation, stored: vec4<f32>, m: f32, projected: ProjectedBrush) -> vec4<f32> { return stored; }";

/// Whether a program needs the full `composite_brush.wgsl`.
pub(super) fn uses_shader(program: &CompositeProgram) -> bool {
    program.operations.iter().any(|op| op.ext[2] & BRUSH_OP_BITS != 0)
}

/// P7's vec4 of the band uniform (`params.packages[6]`).
pub(super) fn band_uniform(_program: &CompositeProgram) -> [u32; 4] {
    [0; 4]
}
