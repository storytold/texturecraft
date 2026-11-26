//! The uniform of one P3b field kernel (`Params` in shaders/generator_p3b.wgsl).

use bytemuck::{Pod, Zeroable};

/// Sizes, the kernel's words, the tile grid, the occlusion bake, the kernel's values and Dripping Rust's taps
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct FieldParams {
    pub(crate) size: [u32; 4],
    pub(crate) words: [u32; 4],
    pub(crate) grid: [f32; 4],
    pub(crate) bake: [u32; 4],
    pub(crate) values: [[f32; 4]; 5],
    pub(crate) taps: [[f32; 4]; 16],
}
