//! P7, brush and live stroke (plan section 2.6), the compositor side. Owned by the P7 agent; no
//! other package edits this file or `shaders/composite_brush.wgsl`.
//!
//! What P7 implements: the Eraser on layers and masks, mask strokes painting a mask image, the
//! pickup tools (Smudge, Clone, Paint with a material pattern; first step: let the GPU compositor
//! preview them from `display_stroke_pixels`), symmetry, brush alpha images and library tips, the
//! Projection tool in the UV view and without a stencil, Polygon Fill and Path, and one device
//! stroke feeding several texture sets of the same size. Ask the user first whether the Eraser
//! and UV-view Projection ports count as the off-limits physical functions (plan section 4.5).
//!
//! Known bug to fix first (verified in the code, plan "suspected silent mismatch"): with a mask
//! image loaded in the tool, a Paint stroke on a layer starts the GPU brush (`image_mask` in
//! crates/engine/src/painting.rs only covers mask targets), but `Session::device_stroke` returns
//! None for any `MaskGray::Image` and `display_stroke_pixels` returns None while a device exists,
//! so `SetCompositor::plan` (crates/ui-egui/src/gpu.rs) composites without the live stroke: it is
//! invisible until release. Resolve the gray per target, or return the device stroke for layer
//! targets, and test it.
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

pub(super) const WGSL: &str = include_str!("shaders/composite_brush.wgsl");
/// The same functions without P7's code, for programs that do not use it.
pub(super) const WGSL_STUB: &str = "fn live_brush_paint(layer: Operation, stored: vec4<f32>, m: f32, projected: ProjectedBrush, pixel: vec2<u32>, uv: vec2<f32>) -> vec4<f32> { return stored; }";

/// Whether a program needs the full `composite_brush.wgsl`.
pub(super) fn uses_shader(program: &CompositeProgram) -> bool {
    program.operations.iter().any(|op| op.ext[2] & BRUSH_OP_BITS != 0)
}

/// P7's vec4 of the band uniform (`params.packages[6]`).
pub(super) fn band_uniform(_program: &CompositeProgram) -> [u32; 4] {
    [0; 4]
}
