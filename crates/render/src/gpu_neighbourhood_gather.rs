//! P5, the gather and whole-set neighbourhood filters (plan section 2.3). Owned by the P5 agent;
//! no other package edits this file or `shaders/neighbourhood_gather.wgsl`.
//!
//! What P5 implements: FXAA, Anisotropic Kuwahara (structure tensor pass, then a sector gather),
//! Height To Normal (the layer's own Height in the field's sixth word while filtering Normal),
//! Warp (`SourceWarp`) and Blur Slope (`SourceSlopeBlur`) with every mode, blending and source,
//! Mirror and Transform (whole-UV-tile reach), and Color Match (the whole set's mean: a reduction
//! pass, then a point map). Custom inputs read the sixth word that P3's inputs write.
//!
//! P5 also owns the whole-tile and whole-set field path in `GpuCompositor::compose`
//! (gpu_composite.rs: band planning and the prepare loop; no other package touches `compose`)
//! and may write `params.packages[4]` there. Whole-set fields do not fit the band scratch (plan
//! risk 1): plan a pre-pass that writes the stage result into a texture.
//!
//! How:
//! - [`supports`] and [`new`] take every filter the built-in stages reject. Kinds: `GATHER_KINDS`
//!   32..48 and `WHOLE_SET_KINDS` 48..56 (gpu_neighbourhood.rs), as `Kind(n)` constants in this
//!   file. `new` returns None only when the size reaches nothing. `StageInputs::filter` builds the
//!   stage; `words` and `extra` carry further parameters.
//! - [`uses_box`], [`reach`] and [`work_per_texel`] answer for P5 kinds only; `uses_box` must
//!   agree with `gather_family_uses_box` in the WGSL file.
//! - [`encode`] runs passes of P5's own (return true), else the apply pass calls
//!   `gather_family_gathered`. New entry points go in [`Pipelines`].
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] and `still_on_cpu` in the tests below. Every port gets a CPU
//! oracle test (`crate::gpu_composite::oracle`, `Tolerance::TRANSCENDENTAL`, normals by dot
//! product) on paint and in a mask, chained, and with a partial region.

use texturecraft_core::Filter;

use super::{NeighbourhoodFilter, NeighbourhoodGpu, NeighbourhoodRegion, StageInputs};

/// Library filter ids P5 still leaves on the CPU with their defaults (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &["warp", "blurSlope", "fxaa", "anisotropicKuwahara", "mirror", "transform", "colorMatch", "heightToNormal"];

pub(super) const WGSL: &str = include_str!("shaders/neighbourhood_gather.wgsl");

/// P5's own compute pipelines, created from the neighbourhood module.
pub(super) struct Pipelines;

impl Pipelines {
    pub(super) fn new(_pipeline: &dyn Fn(&'static str) -> wgpu::ComputePipeline) -> Self {
        Self
    }
}

/// Whether a P5 stage evaluates `filter` as the CPU does.
pub(super) fn supports(_filter: &Filter) -> bool {
    false
}

/// The P5 stage for `filter`, None when its size reaches nothing.
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

/// Whether a stage runs in the whole-set pre-pass instead of in every band (see
/// `NeighbourhoodFilter::whole_set`): large reaches and whole-set reads.
pub(super) fn whole_set(_filter: NeighbourhoodFilter, _resolution: u32) -> bool {
    false
}

/// Encodes a P5 stage with passes of its own; false lets the apply pass run.
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
    use texturecraft_core::{ContentsOffset, EffectKind, Filter};

    use crate::gpu_composite::oracle::{compile_effects, effect};

    /// P5's filters that still composite on the CPU, in content and in masks. Port one: delete
    /// its row (this test fails while a ported row stays) and add its oracle test.
    fn still_on_cpu() -> Vec<Filter> {
        vec![Filter::Fxaa { alpha: false }]
    }

    #[test]
    fn gather_filters_still_on_the_cpu_fall_back() {
        let none = ContentsOffset::NONE;
        for filter in still_on_cpu() {
            let (material, _) = compile_effects(Some(effect(EffectKind::Filter(filter.clone()))), None, none);
            assert!(material.is_err(), "content {filter:?}");
            let (material, mask) = compile_effects(None, Some(effect(EffectKind::Filter(filter.clone()))), none);
            assert!(material.is_err() && mask.is_err(), "mask {filter:?}");
        }
    }
}
