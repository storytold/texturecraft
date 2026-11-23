//! P4, the box family and distance neighbourhood filters (plan section 2.3). Owned by the P4
//! agent; no other package edits this file or `shaders/neighbourhood_box.wgsl`.
//!
//! What P4 implements: Blur and Bevel `wrap`, Highpass `space` (sRGB from linear or ACEScg) and
//! `alpha`, Pixelate `alpha: false`, the Gradient's `input.blur` pre-blur (a box stage, then P2's
//! gradient), Glow (`HaloGlow`), Drop Shadow, Mask Outline, and the distance filters Bevel
//! (`DistanceBevel`), Bevel Smooth and Directional Distance (separable distance transform with
//! exact integer envelope arithmetic: the CPU computes the crossings in f64). The Distance Map
//! input of Bevel Smooth and Directional Distance reads the field's sixth word (P3's inputs).
//!
//! How:
//! - [`supports`] and [`new`] take every filter the built-in stages reject (the built-in ones keep
//!   their exact settings, so `Blur { wrap: true, .. }` reaches them). Kinds: `BOX_KINDS`
//!   16..24 and `DISTANCE_KINDS` 24..32 (gpu_neighbourhood.rs), as `Kind(n)` constants in this file. `new` returns None
//!   only when the size reaches nothing (the effect then changes nothing, as on the CPU).
//!   `StageInputs::filter` builds the stage; `words` and `extra` carry further parameters.
//! - [`uses_box`], [`reach`] and [`work_per_texel`] answer for P4 kinds only; `uses_box` must
//!   agree with `box_family_uses_box` in the WGSL file. Large halos (Drop Shadow offset) belong
//!   in [`reach`].
//! - [`encode`] runs passes of P4's own (return true), else the box passes and the apply pass run;
//!   the WGSL functions `box_family_filtered`/`box_family_gathered` then compute the texel.
//!   New entry points go in [`Pipelines`].
//!
//! - The Gradient's `input.blur` (with P2): P4 provides the stage (a kind whose output keeps the
//!   original colour for the opacity mix and carries the blurred gray where P2's gradient reads
//!   it); P2's `lower` queues it with `ProceduralInputs::precede_with_stage` before the gradient.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`, `Tolerance::NEIGHBOURHOOD`) on paint and in a
//! mask, chained, and with a partial region (the halo); distance filters also compare the raw
//! distance field exactly.

use texturecraft_core::Filter;

use super::{NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion, StageInputs};

/// Library filter ids P4 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &["bevel", "bevelSmooth", "directionalDistance", "glow", "dropShadow", "maskOutline"];

pub(super) const WGSL: &str = include_str!("shaders/neighbourhood_box.wgsl");

/// P4's own compute pipelines, created from the neighbourhood module.
pub(super) struct Pipelines;

impl Pipelines {
    pub(super) fn new(_pipeline: &dyn Fn(&'static str) -> wgpu::ComputePipeline) -> Self {
        Self
    }
}

/// Whether a P4 stage evaluates `filter` as the CPU does.
pub(super) fn supports(_filter: &Filter) -> bool {
    false
}

/// The P4 stage for `filter`, None when its size reaches nothing.
pub(super) fn new(_filter: &Filter, _stage: StageInputs) -> Option<NeighbourhoodFilter> {
    None
}

pub(super) fn uses_box(_filter: NeighbourhoodFilter) -> bool {
    false
}

pub(super) fn reach(filter: NeighbourhoodFilter) -> u32 {
    filter.radius.saturating_add(1)
}

pub(super) fn work_per_texel(_filter: NeighbourhoodFilter) -> u64 {
    8
}

/// Encodes a P4 stage with passes of its own; false lets the box and apply passes run.
pub(super) fn encode(
    (_gpu, _pipelines): (&NeighbourhoodGpu, &Pipelines),
    _device: &wgpu::Device,
    _encoder: &mut wgpu::CommandEncoder,
    _scratch: &wgpu::Buffer,
    _region: NeighbourhoodRegion,
    _filter: NeighbourhoodFilter,
) -> bool {
    false
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use texturecraft_core::{ContentsOffset, EffectKind, Filter, GradientInput, HighpassSpace};

    use crate::gpu_composite::oracle::{compile_effects, effect};

    /// P4's filters and settings that still composite on the CPU, in content and in masks. Port
    /// one: delete its row (this test fails while a ported row stays) and add its oracle test.
    fn still_on_cpu() -> Vec<Filter> {
        vec![
            Filter::DropShadow { angle: 45.0, distance: 0.02, size: 8.0, spread: 0.0, opacity: 1.0, color: [0.0; 3] },
            Filter::Blur { radius: 8.0, wrap: true },
            Filter::Mosaic { size: 8.0, alpha: false },
            Filter::HighPass { radius: 8.0, space: HighpassSpace::SrgbFromLinear, alpha: false },
            Filter::GradientMap {
                colors: vec![[0.0; 3], [1.0; 3]],
                positions: Vec::new(),
                quantity: None,
                input: GradientInput { blur: 2.0, ..Default::default() },
                linear: true,
                debug: false,
            },
            Filter::Bevel { width: 16.0, wrap: true },
            Filter::HighPass { radius: 8.0, space: HighpassSpace::Working, alpha: true },
        ]
    }

    #[test]
    fn box_and_distance_filters_still_on_the_cpu_fall_back() {
        let none = ContentsOffset::NONE;
        for filter in still_on_cpu() {
            let (material, _) = compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, none);
            assert!(material.is_err(), "content {filter:?}");
            let (material, mask) = compile_effects(None, Some(effect(EffectKind::Filter(filter.clone()))), none);
            assert!(material.is_err() && mask.is_err(), "mask {filter:?}");
        }
    }
}
