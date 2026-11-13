//! The bake crate's analytic reference scenes on the GPU tracer: the same scenes and tolerances
//! as `crates/bake/tests/reference.rs` on the CPU.
#![cfg(not(target_arch = "wasm32"))]

#[path = "../../bake/tests/reference_scenes/mod.rs"]
mod reference_scenes;

use crate::gpu_bake::GpuBaker;

/// The GPU baker, or None (test skipped) when this machine has no hardware adapter. The adapter
/// is requested as `GpuBaker` requests it (high performance, software renderers refused). Only
/// its absence skips: with an adapter, any GPU bake error, a WGSL compile error in `bake.wgsl`
/// included, makes the bake fall back to the CPU, which `bake_with` reports as a failure.
fn gpu() -> Option<GpuBaker> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let options = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() };
    let info = match pollster::block_on(instance.request_adapter(&options)) {
        Ok(adapter) => adapter.get_info(),
        Err(error) => {
            eprintln!("skipping: no GPU adapter: {error}");
            return None;
        }
    };
    if info.device_type == wgpu::DeviceType::Cpu {
        eprintln!("skipping: {} is a software renderer", info.name);
        return None;
    }
    Some(GpuBaker::new())
}

/// Every reference scene on the GPU tracer (skipped without an adapter)
#[test]
fn gpu_bakes_the_reference_scenes() {
    let Some(gpu) = gpu() else { return };
    for (name, check) in reference_scenes::CHECKS {
        eprintln!("{name}");
        check(&gpu);
    }
}

/// The mixed-sign reference scene on the GPU tracer
#[test]
#[ignore = "fails until the per-corner tangent sign fix: compute_tangents keeps one tangent per vertex, so mirrored UVs meeting at a vertex mix bitangent signs in a triangle"]
fn gpu_mixed_tangent_signs_decode_like_the_viewport() {
    let Some(gpu) = gpu() else { return };
    reference_scenes::mixed_tangent_signs_decode_like_the_viewport(&gpu);
}
