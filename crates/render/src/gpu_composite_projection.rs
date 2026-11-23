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
//! - Hooks P0b added (stubs return today's result):
//!   - [`place_image`], [`place_procedural`] and [`place_cover`] get the fill's `sampling` and
//!     `&mut ProceduralInputs`; `place_cover` runs after every source kind (uniform and anchor
//!     too), since the CPU's coverage applies to all of them.
//!   - [`image_use`] picks the `ImageUse` that `source_operation` registers an image source with.
//!   - `MaterialImages::register_decal(frame, sources, sampling)` gets the fill's sampling (the
//!     decal path's Nearest and Bilinear HQ come in their own commit of the P1 series).
//!   - Mip chains: `MaterialImages::atlas_images` returns an `AtlasMips` per colour image
//!     (`ImageRes::mip_chain`, or `linear_mip_chain` for an sRGB colour read); the atlas uploads
//!     the levels and `append_rgba` sets flag bit 9 and appends the level origins.
//!   - Depth culling: `MaterialImages::set_surface(mesh)` runs in `set_surface_inputs` before
//!     the atlas is built; build the grids there with `ProjectorDepth::build`/`build_copies` and
//!     `grid()` (compose), return them from `raw_planes()` as exact-f32 `RawPlane`s, and read
//!     their origins with `TileImageAtlas::append_raw`.
//!   - P1 owns `gpu_tile_pattern.rs`'s atlas (`TileImageAtlas`, `AtlasMips`, `RawPlane`) from P0b on.
//! - Band-wide values go in [`band_uniform`] (`params.packages[0]` in WGSL).
//! - Keep [`uses_shader`] true for every program that needs the full WGSL file.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] (no library filter is P1's) and `still_on_cpu` in the tests
//! below. Every port gets a CPU oracle test (`crate::gpu_composite::oracle`) per context (colour,
//! other channel, mask), per setting value, with opacity below 1 and a non-Normal blend.

use texturecraft_core::{Backface, Filtering, Projection, Projector, Sampling, ShapeCrop};

use super::packages::PROJECTION_SELECTORS;
use super::{CompositeProgram, Operation, ProceduralInputs, UV_TILE_PROJECTION};
use crate::gpu_image_source::{DECAL_PROJECTION, ImageUse};

/// Library filter ids P1 still leaves on the CPU (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

pub(super) const WGSL: &str = include_str!("shaders/composite_projection.wgsl");
/// The same functions without P1's code, for programs that do not use it.
pub(super) const WGSL_STUB: &str =
    "fn projection_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> { return vec4(value(layer, uv, surface), 1.0); }";

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

/// How an image source with `sampling` registers its image (`MaterialImages::register_image`;
/// P1: the kind that makes the atlas hold its mip chain for Bilinear HQ, or a nearest read).
pub(super) fn image_use(_sampling: Sampling) -> ImageUse {
    ImageUse::Material
}

/// The projection words of an image source operation. None for a projection it cannot encode.
/// `sampling` and the side table (`inputs.pack_params`) are there for P1's settings.
pub(super) fn place_image(projection: &Projection, _sampling: Sampling, op: &mut Operation, _inputs: &mut ProceduralInputs) -> Option<()> {
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
pub(super) fn place_procedural(projection: &Projection, _sampling: Sampling, op: &mut Operation, _inputs: &mut ProceduralInputs) -> Option<()> {
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

/// The coverage words of every source operation (uniform, anchor, image and procedural alike),
/// after the source kind's own words: the CPU's coverage (wrap `tile_cover`, shape crop,
/// backface, depth culling, decal frames) applies to every kind. Today: a placed (decal)
/// projection reads its frame from `op.uv[0]` (`register_decal`).
pub(super) fn place_cover(projection: &Projection, _sampling: Sampling, op: &mut Operation, _inputs: &mut ProceduralInputs) {
    if projection.placed().is_some() {
        op.procedural[3] = DECAL_PROJECTION;
    }
}

/// The work per output texel of operation `op`, whose own work (a Tile Generator's placements
/// per plane and sample, 0 for other operations) is `own`: P1 adds its fills' cost here,
/// symmetry copies times sampling (planes, Bilinear HQ levels), so bands shrink and one
/// submission stays short. It counts against the update budget too
/// (`MAX_TILE_WORK_PER_UPDATE`). Today: `own`.
pub(super) fn work_per_texel(_op: &Operation, own: u64) -> u64 {
    own
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

    /// The placement hooks (P0b) leave every source kind's operation as before: no package
    /// word, no side-table words, the same projection selector, images registered as material.
    #[test]
    fn placement_hooks_leave_the_operations_as_before() {
        use texturecraft_core::{ChannelKind, NormalFormat, Procedural, ProceduralKind, Projection, Sampling, Source};

        use crate::gpu_composite::{ProceduralInputs, source_operation};
        use crate::gpu_image_source::ImageUse;

        let procedural = Source::Procedural { pattern: Procedural::new(ProceduralKind::Checker), low: [0.1; 3], high: [0.9; 3], ramp: Vec::new() };
        let image = Source::Image { name: "fill.png".into(), normal_format: NormalFormat::OpenGl };
        for (projection, selector) in [(Projection::default(), 0), (Projection::UvTile, 10), (Projection::triplanar(), 1)] {
            for source in [None, Some(Source::Uniform([0.4; 3])), Some(image.clone()), Some(procedural.clone())] {
                let mut inputs = ProceduralInputs::new(64);
                let op = source_operation(source.as_ref(), &projection, Sampling::SAVED_BEFORE, [0.5; 3], ChannelKind::Color, &mut inputs)
                    .unwrap_or_else(|| panic!("{projection:?} {source:?}"));
                assert_eq!(op.ext, [0; 4], "{projection:?} {source:?}");
                assert!(inputs.ramp_words.is_empty(), "{projection:?} {source:?}");
                let expected = if matches!(source, Some(Source::Image { .. }) | Some(Source::Procedural { .. })) { selector } else { 0 };
                assert_eq!(op.procedural[3], expected, "{projection:?} {source:?}");
            }
        }
        // The work hook adds nothing yet: a fill costs what the bands counted before
        let op = source_operation(Some(&image), &Projection::triplanar(), Sampling::SAVED_BEFORE, [0.5; 3], ChannelKind::Color, &mut ProceduralInputs::new(64))
            .unwrap();
        assert_eq!((super::work_per_texel(&op, 0), super::work_per_texel(&op, 7)), (0, 7));
        assert_eq!(super::image_use(Sampling::default()), ImageUse::Material);
        assert_eq!(super::image_use(Sampling::SAVED_BEFORE), ImageUse::Material);
    }

    /// Placed planars and warps get a frame record (P0d), other projections none, and a warp
    /// fill still composites on the CPU with the same reason until P1 evaluates it.
    #[test]
    fn warps_get_a_frame_record_like_placed_planars() {
        use std::collections::BTreeMap;

        use texturecraft_compose::PixelStore;
        use texturecraft_core::{Channel, Decal, Fill, Layer, LayerKind, Project, Projection, Source, TextureSet};

        use crate::gpu_composite::{CompositeProgram, register_decal};
        use crate::gpu_image_source::MaterialImages;

        let frame = Decal {
            center: [0.1, 0.0, 0.05],
            normal: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            size: [0.5, 0.7],
            rotation: 12.0,
            offset: [0.0; 2],
            depth: 0.07,
            symmetry: None,
            flip: [false; 2],
        };
        let fill = |projection: Projection| Fill {
            sources: BTreeMap::from([(Channel::BaseColor, Source::Image { name: "decal.png".into(), normal_format: Default::default() })]),
            projection,
            ..Default::default()
        };
        let mut materials = MaterialImages::default();
        assert_eq!(register_decal(&fill(Projection::default()), &mut materials), Ok(None));
        assert_eq!(register_decal(&fill(Projection::decal(frame)), &mut materials), Ok(Some(0)));
        assert_eq!(register_decal(&fill(Projection::warp(frame)), &mut materials), Ok(Some(0)), "the same frame and image share a record");

        let mut project = Project::new("Warp", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, 64));
        let layer = Layer::new(project.alloc_id(), "Warp", LayerKind::Fill(fill(Projection::warp(frame))), &[Channel::BaseColor]);
        project.insert_layer(0, layer, None, false).unwrap();
        let result = CompositeProgram::try_compile_region(&project, &PixelStore::default(), 0, None).map(|_| ());
        assert_eq!(result, Err("A fill source or projection uses CPU compositing."));
    }

    #[test]
    fn fill_sampling_and_projections_still_on_the_cpu_fall_back() {
        for kind in still_on_cpu() {
            let (material, mask) = compile_effects(None, Some(effect(kind.clone())), ContentsOffset::NONE);
            assert!(material.is_err() && mask.is_err(), "{kind:?}");
        }
    }
}
