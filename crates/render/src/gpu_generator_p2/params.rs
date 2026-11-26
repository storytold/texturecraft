use bytemuck::{Pod, Zeroable};

use super::{KERNEL_TAPS, OCCLUSION_DIRECTIONS, OCCLUSION_STEPS};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct Params {
    pub(super) block: [u32; 4],
    pub(super) slots: [u32; 4],
    pub(super) uniforms: [f32; 4],
    pub(super) origins: [u32; 4],
    pub(super) place: [u32; 4],
    pub(super) lines: [u32; 4],
    pub(super) gains: [f32; 4],
    pub(super) scales: [f32; 4],
    pub(super) intensity: [f32; 4],
    pub(super) kernel: [[f32; 4]; KERNEL_TAPS / 4],
    pub(super) directions: [[f32; 4]; OCCLUSION_DIRECTIONS],
    pub(super) taps: [[f32; 4]; OCCLUSION_DIRECTIONS * OCCLUSION_STEPS],
}
