//! P1, fill sampling and projection (plan section 2.1). Owned by the P1 agent; no other package
//! edits this file or `shaders/composite_projection.wgsl`.
//!
//! What P1 implements here, until [`supports`] accepts every projection and sampling the CPU has
//! and [`supports_image_sampling`] accepts every filtering:
//! - image filtering Bilinear HQ (the default: a mip chain per material image and a footprint
//!   from the UV derivatives) and Nearest;
//! - UV wrap other than Repeat, fill symmetry (copies), `UvTransform` and projector rotation and
//!   scale on the mesh projections, shape crop and backface, depth culling (the default of new
//!   planar fills), cylindrical angles below 360, the warp projection;
//! - procedural coordinates beyond ±32768 (only after the parity study, plan risk 2).
//!
//! How:
//! - [`place_image`] and [`place_procedural`] fill the operation's projection words. New
//!   projections take a selector from `packages::PROJECTION_SELECTORS` (16..32) in
//!   `procedural[3]`, flags in `Operation::ext[1]` (any bit set routes the fill through
//!   `projection_fill_value` in the WGSL file) and further numbers in the side table
//!   (`ProceduralInputs::pack_params`, offset in `ext[0]`).
//! - P1 also owns `gpu_image_source.rs` (`MaterialImages`, mip chains, filtering, wrap) and
//!   `shaders/material_image.wgsl`. `MaterialImages::register_image(name, ImageUse)` is the frozen
//!   API P3 and P7 call: add image kinds as `ImageUse` variants, never change the signature.
//! - Band-wide values go in [`band_uniform`] (`params.packages[0]` in WGSL).
//! - Keep [`uses_shader`] true for every program that needs the full WGSL file.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] (no library filter is P1's) and `still_on_cpu` in the tests
//! below. Every port gets a CPU oracle test (`crate::gpu_composite::oracle`) per context (colour,
//! other channel, mask), per setting value, with opacity below 1 and a non-Normal blend.

use texturecraft_core::{Backface, Filtering, Projection, Projector, Sampling, ShapeCrop};

use super::packages::PROJECTION_SELECTORS;
use super::{CompositeProgram, Operation, UV_TILE_PROJECTION};
use crate::gpu_image_source::DECAL_PROJECTION;

/// Library filter ids P1 still leaves on the CPU (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

pub(super) const WGSL: &str = include_str!("shaders/composite_projection.wgsl");
/// The same functions without P1's code, for programs that do not use it.
pub(super) const WGSL_STUB: &str = "fn projection_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> { return vec4(value(layer, uv, surface), 1.0); }";

/// Whether the GPU encodes `projection` with `sampling` as the CPU evaluates it.
pub(super) fn supports(projection: &Projection, sampling: Sampling) -> bool {
    // Keep the CPU result for projection features the compositor does not encode yet.
    if sampling.wrap.repeats() != (true, true) || (sampling.symmetry.on && projection.takes_symmetry()) {
        return false;
    }
    match projection {
        Projection::Uv { .. } | Projection::UvTile => true,
        Projection::Triplanar { uv, projector, .. } | Projection::Spherical { uv, projector, .. } => uv.is_default() && projector.is_default(),
        Projection::Planar { uv, projector, placed, .. } => {
            let expected = if placed.is_some() {
                Projector { crop: ShapeCrop::Cropped, backface: Backface { on: true, ..Backface::default() }, ..Projector::default() }
            } else {
                Projector::default()
            };
            uv.is_default() && *projector == expected
        }
        Projection::Cylindrical { uv, projector, angle, .. } => uv.is_default() && projector.is_default() && *angle == texturecraft_core::FULL_TURN_DEGREES,
        Projection::Warp { .. } => false,
    }
}

/// Whether the GPU samples image sources (fills and decals) with this filtering as the CPU does.
pub(super) fn supports_image_sampling(sampling: Sampling) -> bool {
    sampling.filtering == Filtering::BilinearSharp
}

/// The projection words of an image source operation. None for a projection it cannot encode.
pub(super) fn place_image(projection: &Projection, op: &mut Operation) -> Option<()> {
    match projection {
        Projection::Uv { scale, rotation, offset } => {
            let (sin, cos) = rotation.to_radians().sin_cos();
            op.projection = [cos, sin, scale[0], scale[1]];
            op.uv[..2].copy_from_slice(offset);
        }
        Projection::UvTile => op.procedural[3] = UV_TILE_PROJECTION,
        Projection::Triplanar { scale, sharpness, offset, .. } => {
            op.projection = [offset[0], offset[1], offset[2], *scale];
            op.procedural[3] = 1;
            op.balance[3] = sharpness.max(1.0);
        }
        Projection::Planar { placed: Some(_), .. } => {}
        Projection::Planar { scale, axis, offset, .. } => {
            op.projection = [offset[0], offset[1], offset[2], *scale];
            op.procedural[3] = 2 + u32::from((*axis).min(2));
        }
        Projection::Spherical { scale, offset, .. } => {
            op.projection = [offset[0], offset[1], offset[2], *scale];
            op.procedural[3] = 5;
        }
        Projection::Cylindrical { scale, axis, offset, .. } => {
            op.projection = [offset[0], offset[1], offset[2], *scale];
            op.procedural[3] = match axis {
                0 => 6,
                2 => 8,
                _ => 7,
            };
        }
        Projection::Warp { .. } => return None,
    }
    Some(())
}

/// The projection words of a procedural source operation, whose scale is already in `op.uv[2]`.
/// None for a projection it cannot encode, or coordinates beyond the guarded f32 range.
pub(super) fn place_procedural(projection: &Projection, op: &mut Operation) -> Option<()> {
    match projection {
        Projection::Uv { scale, rotation, offset } => {
            let (sin, cos) = rotation.to_radians().sin_cos();
            op.projection = [cos, sin, scale[0], scale[1]];
            op.uv[..2].copy_from_slice(offset);
            if scale.iter().zip(offset).any(|(s, o)| (s.abs() * 2.0 + o.abs()) * op.uv[2] > 32_768.0) {
                return None;
            }
        }
        Projection::UvTile => op.procedural[3] = UV_TILE_PROJECTION,
        Projection::Planar { placed: Some(_), .. } => {}
        Projection::Triplanar { scale, offset, .. } | Projection::Planar { scale, offset, .. } => {
            let frequency = *scale * op.uv[2];
            if offset.iter().any(|o| frequency.abs() + o.abs() > 32_768.0) {
                return None;
            }
            op.projection = [offset[0], offset[1], offset[2], frequency];
            if let Projection::Planar { axis, .. } = projection {
                op.procedural[3] = 2 + u32::from((*axis).min(2));
            } else if let Projection::Triplanar { sharpness, .. } = projection {
                op.procedural[3] = 1;
                op.balance[3] = sharpness.max(1.0);
            }
        }
        Projection::Spherical { scale, offset, .. } | Projection::Cylindrical { scale, offset, .. } => {
            let frequency = *scale * op.uv[2];
            if frequency.abs() * 4.0 > 32_768.0 {
                return None;
            }
            op.projection = [offset[0], offset[1], offset[2], frequency];
            op.procedural[3] = match projection {
                Projection::Cylindrical { axis: 0, .. } => 6,
                Projection::Cylindrical { axis: 2, .. } => 8,
                Projection::Cylindrical { .. } => 7,
                _ => 5,
            };
        }
        Projection::Warp { .. } => return None,
    }
    Some(())
}

/// A placed (decal) projection reads its frame from `op.uv[0]` (`register_decal`).
pub(super) fn place_decal(projection: &Projection, op: &mut Operation) {
    if projection.placed().is_some() {
        op.procedural[3] = DECAL_PROJECTION;
    }
}

/// Whether a program needs the full `composite_projection.wgsl`.
pub(super) fn uses_shader(program: &CompositeProgram) -> bool {
    program.operations.iter().any(|op| op.ext[1] != 0 || PROJECTION_SELECTORS.contains(&op.procedural[3]))
}

/// P1's vec4 of the band uniform (`params.packages[0]`).
pub(super) fn band_uniform(_program: &CompositeProgram) -> [u32; 4] {
    [0; 4]
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use texturecraft_core::{ContentsOffset, EffectKind};

    use crate::gpu_composite::oracle::{compile_effects, effect};

    /// P1's cases that still composite on the CPU, as mask effects (the content and the mask
    /// preview both compile them). Port one: delete its row (this test fails while a ported row
    /// stays) and add its oracle test.
    fn still_on_cpu() -> Vec<EffectKind> {
        Vec::new()
    }

    #[test]
    fn fill_sampling_and_projections_still_on_the_cpu_fall_back() {
        for kind in still_on_cpu() {
            let (material, mask) = compile_effects(None, Some(effect(kind.clone())), ContentsOffset::NONE);
            assert!(material.is_err() && mask.is_err(), "{kind:?}");
        }
    }
}
