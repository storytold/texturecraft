//! P1, fill sampling and projection (plan section 2.1). Owned by the P1 agent; no other package
//! edits this file or `shaders/composite_projection.wgsl`.
//!
//! The compositor's built-in paths evaluate the fills they always did (`built_in`). Every other
//! fill takes P1's path: [`place_cover`] writes the whole projection into the side table
//! (`SLOT_*`, offset in `ext[0]`) and its flags into `ext[1]` (`FLAG_*`), and
//! `projection_fill_value` in the WGSL file ports the CPU evaluator's fill texel from them.
//!
//! P1's path evaluates:
//! - image filtering Bilinear HQ (the default: a mip chain per material image and the CPU's
//!   footprint) and Nearest, on fills and placed frames;
//! - UV wrap other than Repeat, fill symmetry (copies), `UvTransform` and projector rotation and
//!   scale on the mesh projections, shape crop and backface, depth culling (the default of new
//!   planar fills, through the CPU's own depth grids), cylindrical angles below 360, the warp
//!   projection.
//!
//! Still on the CPU: a cropped warp that takes its image's alpha (no frame record carries a
//! warp's images), and procedural coordinates beyond ±32768 (after the parity study, plan risk 2).
//!
//! How:
//! - [`place_image`] and [`place_procedural`] fill the operation's projection words for the
//!   built-in paths. New projections take a selector from `packages::PROJECTION_SELECTORS`
//!   (16..32) in `procedural[3]`.
//! - P1 also owns `gpu_image_source.rs` (`MaterialImages`, mip chains, filtering, wrap) and
//!   `shaders/material_image.wgsl`. `MaterialImages::register_image(name, ImageUse)` is the frozen
//!   API P3 and P7 call: add image kinds as `ImageUse` variants, never change the signature.
//! - Hooks P0b added:
//!   - [`place_image`], [`place_procedural`] and [`place_cover`] get the fill's `sampling` and
//!     `&mut ProceduralInputs`; `place_cover` runs after every source kind (uniform and anchor
//!     too), since the CPU's coverage applies to all of them.
//!   - [`image_use`] picks the `ImageUse` that `source_operation` registers an image source with.
//!   - `MaterialImages::register_decal(frame, sources, sampling)` gets the fill's sampling.
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

use texturecraft_compose::frame_placement::warp_reach;
use texturecraft_compose::pattern::{cylinder_axis_index, decal_axes, projector_stretch};
use texturecraft_core::{
    Backface, ChannelKind, Decal, DecalFlip, FULL_TURN_DEGREES, FillSymmetry, Filtering, Projection, Projector, Sampling, ShapeCrop, SymmetryKind, UvTransform,
};
use texturecraft_geom::glam::{EulerRot, Mat3};

use super::packages::PROJECTION_SELECTORS;
use super::{CompositeProgram, Operation, ProceduralInputs, UV_TILE_PROJECTION};
use crate::gpu_image_source::{DECAL_PROJECTION, IMAGE_SOURCE_OPCODE, ImageUse};

/// Library filter ids P1 still leaves on the CPU (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

pub(super) const WGSL: &str = concat!(include_str!("shaders/composite_projection_shared.wgsl"), include_str!("shaders/composite_projection.wgsl"));
/// The same functions without P1's code, for programs that do not use it. The shared constants
/// stay: material_image.wgsl and composite_patterns.wgsl read them.
pub(super) const WGSL_STUB: &str = concat!(
    include_str!("shaders/composite_projection_shared.wgsl"),
    "fn projection_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> { return vec4(value(layer, uv, surface), 1.0); }"
);

/// `procedural[3]` of a warp projection
const WARP_PROJECTION: u32 = PROJECTION_SELECTORS.start;

// `Operation::ext[1]` of a fill on P1's path (the P1_* constants of the WGSL files)
const FLAG_ROUTED: u32 = 1 << 0;
const FLAG_NEAREST: u32 = 1 << 1;
const FLAG_HQ: u32 = 1 << 2;
const FLAG_CLAMP_U: u32 = 1 << 3;
const FLAG_CLAMP_V: u32 = 1 << 4;
const FLAG_TILE_COVER: u32 = 1 << 5;
const FLAG_TURNS: u32 = 1 << 6;
const FLAG_CROP: u32 = 1 << 7;
const FLAG_BACKFACE: u32 = 1 << 8;
const FLAG_SYMMETRY: u32 = 1 << 9;
const FLAG_DEPTH: u32 = 1 << 10;
const FLAG_FRAMED: u32 = 1 << 11;
const FLAG_IMAGE_ALPHA: u32 = 1 << 12;
const FLAG_FRAME_VALID: u32 = 1 << 13;
const FLAG_UV_PLACED: u32 = 1 << 14;

// Side-table words of a fill on P1's path, from `ext[0]` (the P1_* slots of composite_projection.wgsl)
const SLOT_KIND: usize = 0;
const SLOT_AXIS: usize = 1;
const SLOT_SCALE: usize = 2;
const SLOT_SHARPNESS: usize = 4;
const SLOT_OFFSET: usize = 5;
const SLOT_ROTATION: usize = 8;
const SLOT_TO_LOCAL: usize = 10;
const SLOT_STRETCH: usize = 19;
const SLOT_UV_TILING: usize = 22;
const SLOT_UV_ROTATION: usize = 24;
const SLOT_UV_OFFSET: usize = 26;
const SLOT_CYLINDER_ANGLE: usize = 28;
const SLOT_BACKFACE_THRESHOLD: usize = 29;
const SLOT_BACKFACE_WIDTH: usize = 30;
const SLOT_FRAME_ORIGIN: usize = 31;
const SLOT_FRAME_RIGHT: usize = 34;
const SLOT_FRAME_UP: usize = 37;
const SLOT_FRAME_NORMAL: usize = 40;
const SLOT_FRAME_SIZE: usize = 43;
const SLOT_FRAME_DEPTH: usize = 45;
const SLOT_FRAME_FLIP: usize = 46;
const SLOT_FRAME_MIRROR_AXIS: usize = 47;
const SLOT_FRAME_MIRROR_OFFSET: usize = 48;
const SLOT_FRAME_MIRROR_FLIP: usize = 49;
const SLOT_WARP_CORNERS: usize = 50;
const SLOT_SYMMETRY_KIND: usize = 58;
const SLOT_SYMMETRY_AXIS: usize = 59;
const SLOT_SYMMETRY_COUNT: usize = 60;
const SLOT_SYMMETRY_POSITION: usize = 61;
const SLOT_SYMMETRY_STEP: usize = 64;
const SLOT_SYMMETRY_SPAN: usize = 65;
const SLOT_SYMMETRY_FLIP: usize = 66;
const SLOT_DEPTH_HARDNESS: usize = 67;
const SLOT_DEPTH_DIRECTORY: usize = 68;
const SLOT_WORDS: usize = 69;

// Projection kinds in `SLOT_KIND`
const KIND_UV: u32 = 0;
const KIND_UV_TILE: u32 = 1;
const KIND_TRIPLANAR: u32 = 2;
const KIND_PLANAR: u32 = 3;
const KIND_SPHERICAL: u32 = 4;
const KIND_CYLINDRICAL: u32 = 5;
const KIND_PLACED: u32 = 6;
const KIND_WARP: u32 = 7;

/// The turn (in cosine) over which backface culling fades at hardness 0, as compose's `projector_cover`
const BACKFACE_FADE: f32 = 0.6;
/// Smallest frame side or depth in world units, as compose's `pattern` keeps them
const MIN_FRAME_EXTENT: f32 = 1e-6;

/// Whether the GPU samples image sources (fills and decals) with this filtering as the CPU does:
/// every filtering, a decal's on P1's path.
pub(super) fn supports_image_sampling(_sampling: Sampling) -> bool {
    true
}

/// How an image source read as a `channel` with `sampling` registers its image
/// (`MaterialImages::register_image`): Bilinear HQ needs the image's halved levels, linear ones
/// for an sRGB colour read.
pub(super) fn image_use(sampling: Sampling, channel: ChannelKind) -> ImageUse {
    match (sampling.filtering, channel) {
        (Filtering::BilinearHq, ChannelKind::Color) => ImageUse::MipmappedColor,
        (Filtering::BilinearHq, _) => ImageUse::Mipmapped,
        (Filtering::BilinearSharp | Filtering::Nearest, _) => ImageUse::Material,
    }
}

/// The projection words of an image source operation for the built-in paths.
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
        // Framed projections read everything from P1's side table
        Projection::Planar { placed: Some(_), .. } | Projection::Warp { .. } => {}
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
    }
    Some(())
}

/// The projection words of a procedural source operation, whose scale is already in `op.uv[2]`.
/// None for coordinates beyond the guarded f32 range.
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
        Projection::Planar { placed: Some(_), .. } | Projection::Warp { .. } => {}
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
    }
    Some(())
}

/// The coverage of every source operation (uniform, anchor, image and procedural alike), after
/// the source kind's own words: a fill the built-in paths evaluate keeps them, any other takes
/// P1's path. None for a fill that stays on the CPU.
pub(super) fn place_cover(projection: &Projection, sampling: Sampling, op: &mut Operation, inputs: &mut ProceduralInputs) -> Option<()> {
    // A placed (decal) projection reads its frame from `op.uv[0]` (`register_decal`)
    if projection.placed().is_some() {
        op.procedural[3] = DECAL_PROJECTION;
    }
    if built_in(projection, sampling, op) {
        return Some(());
    }
    if cropped_warp_reads_image_alpha(projection) {
        return None;
    }

    let mut flags = FLAG_ROUTED;
    let words = side_table(projection, sampling, &mut flags)?;
    let offset = inputs.pack_words(&words)?;

    // The grids are built over the mesh, their directory written into the side table on upload
    if flags & FLAG_DEPTH != 0 {
        inputs.materials.register_depth(projection, sampling, offset.checked_add(SLOT_DEPTH_DIRECTORY as u32)?);
    }
    op.ext[0] = offset;
    op.ext[1] = flags;
    op.procedural[3] = selector(projection);
    Some(())
}

/// The projector of a decal the built-in decal path evaluates.
fn decal_projector() -> Projector {
    Projector { crop: ShapeCrop::Cropped, backface: Backface { on: true, ..Backface::default() }, ..Projector::default() }
}

/// Whether the compositor's built-in paths evaluate `op` through `projection` with `sampling` as
/// the CPU does, so it needs nothing from P1.
fn built_in(projection: &Projection, sampling: Sampling, op: &Operation) -> bool {
    // Filtering changes only what an image source reads
    let filtering = sampling.filtering == Filtering::BilinearSharp || op.ids[1] != IMAGE_SOURCE_OPCODE;
    if sampling.wrap.repeats() != (true, true) || (sampling.symmetry.on && projection.takes_symmetry()) {
        return false;
    }
    match projection {
        Projection::Uv { .. } | Projection::UvTile => filtering,
        Projection::Triplanar { uv, projector, .. } | Projection::Spherical { uv, projector, .. } => filtering && uv.is_default() && projector.is_default(),
        // The decal path reads its frame's image alpha bilinearly, whatever the source
        Projection::Planar { uv, projector, placed: Some(_), .. } => {
            sampling.filtering == Filtering::BilinearSharp && uv.is_default() && *projector == decal_projector()
        }
        Projection::Planar { uv, projector, .. } => filtering && uv.is_default() && projector.is_default(),
        Projection::Cylindrical { uv, projector, angle, .. } => filtering && uv.is_default() && projector.is_default() && *angle == FULL_TURN_DEGREES,
        Projection::Warp { .. } => false,
    }
}

/// A warp cropped to its shape takes its image's alpha, which only the fill knows: the GPU has
/// no frame record for a warp (`register_decal` registers placed planars only), so it stays on
/// the CPU.
fn cropped_warp_reads_image_alpha(projection: &Projection) -> bool {
    matches!(projection, Projection::Warp { projector, .. } if projector.crop == ShapeCrop::Cropped && !projector.ignore_image_alpha)
}

/// The `procedural[3]` selector of a fill on P1's path, so the program binds the surface its
/// projection reads.
fn selector(projection: &Projection) -> u32 {
    match projection {
        Projection::Uv { .. } => 0,
        Projection::UvTile => UV_TILE_PROJECTION,
        Projection::Triplanar { .. } => 1,
        Projection::Planar { placed: Some(_), .. } => DECAL_PROJECTION,
        Projection::Planar { axis, .. } => 2 + u32::from((*axis).min(2)),
        Projection::Spherical { .. } => 5,
        Projection::Cylindrical { axis: 0, .. } => 6,
        Projection::Cylindrical { axis: 2, .. } => 8,
        Projection::Cylindrical { .. } => 7,
        Projection::Warp { .. } => WARP_PROJECTION,
    }
}

fn projector_of(projection: &Projection) -> Option<&Projector> {
    match projection {
        Projection::Uv { .. } | Projection::UvTile => None,
        Projection::Triplanar { projector, .. }
        | Projection::Planar { projector, .. }
        | Projection::Spherical { projector, .. }
        | Projection::Cylindrical { projector, .. }
        | Projection::Warp { projector, .. } => Some(projector),
    }
}

fn uv_transform_of(projection: &Projection) -> Option<&UvTransform> {
    match projection {
        Projection::Uv { .. } | Projection::UvTile => None,
        Projection::Triplanar { uv, .. }
        | Projection::Planar { uv, .. }
        | Projection::Spherical { uv, .. }
        | Projection::Cylindrical { uv, .. }
        | Projection::Warp { uv, .. } => Some(uv),
    }
}

/// Write finite `values` from word `slot` on; None for a value that is not finite.
fn put(words: &mut [u32; SLOT_WORDS], slot: usize, values: &[f32]) -> Option<()> {
    if !values.iter().all(|value| value.is_finite()) {
        return None;
    }
    for (word, value) in words[slot..slot + values.len()].iter_mut().zip(values) {
        *word = value.to_bits();
    }
    Some(())
}

/// The side-table words of a fill on P1's path, its flags added to `flags`. None for a setting
/// that is not finite.
fn side_table(projection: &Projection, sampling: Sampling, flags: &mut u32) -> Option<[u32; SLOT_WORDS]> {
    let mut words = [0; SLOT_WORDS];
    *flags |= match sampling.filtering {
        Filtering::Nearest => FLAG_NEAREST,
        Filtering::BilinearHq => FLAG_HQ,
        Filtering::BilinearSharp => 0,
    };
    let (across, down) = sampling.wrap.repeats();
    if !across {
        *flags |= FLAG_CLAMP_U;
    }
    if !down {
        *flags |= FLAG_CLAMP_V;
    }

    // Tri-planar never ends, so only the other projections leave their tile
    if !(across && down) && !matches!(projection, Projection::Triplanar { .. }) {
        *flags |= FLAG_TILE_COVER;
    }

    match projection {
        Projection::Uv { scale, rotation, offset } => {
            words[SLOT_KIND] = KIND_UV;
            let (sin, cos) = rotation.to_radians().sin_cos();
            put(&mut words, SLOT_SCALE, scale)?;
            put(&mut words, SLOT_ROTATION, &[sin, cos])?;
            put(&mut words, SLOT_OFFSET, &[offset[0], offset[1], 0.0])?;
        }
        Projection::UvTile => words[SLOT_KIND] = KIND_UV_TILE,
        Projection::Triplanar { scale, sharpness, offset, .. } => {
            words[SLOT_KIND] = KIND_TRIPLANAR;
            put(&mut words, SLOT_SCALE, &[*scale; 2])?;
            put(&mut words, SLOT_SHARPNESS, &[sharpness.max(1.0)])?;
            put(&mut words, SLOT_OFFSET, offset)?;
        }
        Projection::Planar { placed: Some(frame), .. } => {
            words[SLOT_KIND] = KIND_PLACED;
            put_frame(&mut words, frame, frame.depth, flags)?;
            if let Some(symmetry) = frame.symmetry {
                words[SLOT_FRAME_MIRROR_AXIS] = u32::from(symmetry.axis.min(2)) + 1;
                words[SLOT_FRAME_MIRROR_FLIP] = match symmetry.flip {
                    DecalFlip::None => 0,
                    DecalFlip::U => 1,
                    DecalFlip::V => 2,
                };
                put(&mut words, SLOT_FRAME_MIRROR_OFFSET, &[symmetry.offset])?;
            }
        }
        Projection::Planar { scale, axis, offset, .. } => {
            words[SLOT_KIND] = KIND_PLANAR;
            words[SLOT_AXIS] = u32::from((*axis).min(2));
            put(&mut words, SLOT_SCALE, &[*scale; 2])?;
            put(&mut words, SLOT_OFFSET, offset)?;
        }
        Projection::Spherical { scale, offset, .. } => {
            words[SLOT_KIND] = KIND_SPHERICAL;
            put(&mut words, SLOT_SCALE, &[*scale; 2])?;
            put(&mut words, SLOT_OFFSET, offset)?;
        }
        Projection::Cylindrical { scale, axis, offset, angle, .. } => {
            words[SLOT_KIND] = KIND_CYLINDRICAL;
            words[SLOT_AXIS] = cylinder_axis_index(*axis) as u32;
            put(&mut words, SLOT_SCALE, &[*scale; 2])?;
            put(&mut words, SLOT_OFFSET, offset)?;
            put(&mut words, SLOT_CYLINDER_ANGLE, &[*angle])?;
        }
        Projection::Warp { frame, corners, depth, .. } => {
            words[SLOT_KIND] = KIND_WARP;
            put_frame(&mut words, frame, warp_reach(frame.size, *depth), flags)?;
            put(&mut words, SLOT_WARP_CORNERS, corners.as_flattened())?;
        }
    }

    if let Some(uv) = uv_transform_of(projection).filter(|uv| !uv.is_default()) {
        *flags |= FLAG_UV_PLACED;
        let (sin, cos) = uv.rotation.to_radians().sin_cos();
        put(&mut words, SLOT_UV_TILING, &uv.tiling)?;
        put(&mut words, SLOT_UV_ROTATION, &[sin, cos])?;
        put(&mut words, SLOT_UV_OFFSET, &uv.offset)?;
    }

    if let Some(projector) = projector_of(projection) {
        put_projector(&mut words, projection, projector, flags)?;
    }

    if sampling.symmetry.on && projection.takes_symmetry() {
        *flags |= FLAG_SYMMETRY;
        put_symmetry(&mut words, &sampling.symmetry)?;
    }

    // The depth grid's directory word is filled in when the program uploads (`MaterialImages::append_words`)
    if let Some(culling) = projection.depth_culling().filter(|culling| culling.hardness.is_finite()) {
        *flags |= FLAG_DEPTH;
        put(&mut words, SLOT_DEPTH_HARDNESS, &[culling.hardness])?;
    }
    Some(words)
}

/// A framed projection's frame in world units (compose `decal_axes`, `frame_local`), reaching
/// `depth` in front of and behind it. A degenerate frame covers nothing.
fn put_frame(words: &mut [u32; SLOT_WORDS], frame: &Decal, depth: f32, flags: &mut u32) -> Option<()> {
    *flags |= FLAG_FRAMED;
    let Some((origin, right, up, normal)) = decal_axes(frame) else { return Some(()) };

    *flags |= FLAG_FRAME_VALID;
    put(words, SLOT_FRAME_ORIGIN, &origin.to_array())?;
    put(words, SLOT_FRAME_RIGHT, &right.to_array())?;
    put(words, SLOT_FRAME_UP, &up.to_array())?;
    put(words, SLOT_FRAME_NORMAL, &normal.to_array())?;
    put(words, SLOT_FRAME_SIZE, &frame.size.map(|side| side.max(MIN_FRAME_EXTENT)))?;
    put(words, SLOT_FRAME_DEPTH, &[depth.max(MIN_FRAME_EXTENT)])?;
    words[SLOT_FRAME_FLIP] = u32::from(frame.flip[0]) | (u32::from(frame.flip[1]) << 1);
    Some(())
}

/// The projector's turn and stretch (compose `projector_local`), shape crop and backface culling
/// (compose `projector_cover`, `placed_cover`).
fn put_projector(words: &mut [u32; SLOT_WORDS], projection: &Projection, projector: &Projector, flags: &mut u32) -> Option<()> {
    if projector.turns() {
        *flags |= FLAG_TURNS;
        let [x, y, z] = projector.rotation.map(f32::to_radians);
        let to_local = Mat3::from_euler(EulerRot::ZYX, z, y, x).transpose();
        put(words, SLOT_TO_LOCAL, &to_local.to_cols_array())?;
        put(words, SLOT_STRETCH, &projector_stretch(projector).to_array())?;
    }
    if projector.crop == ShapeCrop::Cropped {
        *flags |= FLAG_CROP;

        // Cropped to its shape, a placed frame takes the shape of its image too
        if projection.placed().is_some() && !projector.ignore_image_alpha {
            *flags |= FLAG_IMAGE_ALPHA;
        }
    }

    // A hand-edited NaN angle or hardness culls nothing
    let backface = projector.backface;
    if backface.on && backface.angle.is_finite() && backface.hardness.is_finite() {
        *flags |= FLAG_BACKFACE;
        let threshold = (90.0 - backface.angle).to_radians().sin();
        let width = (1.0 - backface.hardness.clamp(0.0, 1.0)) * BACKFACE_FADE;
        put(words, SLOT_BACKFACE_THRESHOLD, &[threshold])?;
        put(words, SLOT_BACKFACE_WIDTH, &[width])?;
    }
    Some(())
}

/// A fill's symmetry (compose `FillCopies`): its kind, axis, copies, pivot in mesh radii, radial
/// step and span, and the copies' flip.
fn put_symmetry(words: &mut [u32; SLOT_WORDS], symmetry: &FillSymmetry) -> Option<()> {
    let reach = FillSymmetry::REACH;
    let position = symmetry.position.map(|coordinate| if coordinate.is_finite() { coordinate.clamp(-reach, reach) } else { 0.0 });

    // A span that is not a number spreads the copies over a full turn
    let span = if symmetry.span.is_nan() { FULL_TURN_DEGREES } else { symmetry.span };
    words[SLOT_SYMMETRY_KIND] = u32::from(symmetry.kind == SymmetryKind::Radial);
    words[SLOT_SYMMETRY_AXIS] = symmetry.axis_index() as u32;
    words[SLOT_SYMMETRY_COUNT] = symmetry.copies();
    words[SLOT_SYMMETRY_FLIP] = u32::from(symmetry.flip[0]) | (u32::from(symmetry.flip[1]) << 1);
    put(words, SLOT_SYMMETRY_POSITION, &position)?;
    put(words, SLOT_SYMMETRY_STEP, &[symmetry.step()])?;
    put(words, SLOT_SYMMETRY_SPAN, &[span])
}

/// Whether a program needs the full `composite_projection.wgsl`.
pub(super) fn uses_shader(program: &CompositeProgram) -> bool {
    program.operations.iter().any(|op| op.ext[1] != 0 || PROJECTION_SELECTORS.contains(&op.procedural[3]))
}

/// P1's vec4 of the band uniform (`params.packages[0]`): the mesh's centre and radius, which turn
/// the shader's positions in mesh radii back into the world positions frames read.
pub(super) fn band_uniform(program: &CompositeProgram) -> [u32; 4] {
    program.material_images.mesh_frame().map_or([0; 4], |(center, radius)| [center.x, center.y, center.z, radius].map(f32::to_bits))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageRes, ImageStore, PixelStore};
    use texturecraft_core::{
        Backface, BlendMode, Channel, ChannelBlend, ContentsOffset, Decal, DepthCulling, Effect, EffectKind, Fill, FillSymmetry, Filter, Filtering, Layer,
        LayerId, LayerKind, Mask, MaskBackground, Procedural, ProceduralKind, Project, Projection, Projector, Sampling, ShapeCrop, Source, SymmetryKind,
        TextureSet, UvTransform, UvWrap,
    };
    use texturecraft_geom::{Mesh, primitives};
    use texturecraft_texel::TILE;

    use crate::gpu_composite::CompositeProgram;
    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, compile_effects, effect, headless};
    use crate::headless::Headless;

    /// Texture set side of the oracle cases
    const SIZE: u32 = TILE * 2;
    /// Texels a hard decision (nearest texel, an edge, culling) may flip: 0.1 % of the set
    const FLIPS: usize = (SIZE * SIZE / 1000) as usize;

    /// A unit sphere: positions and normals all round, its UVs over the whole set
    fn sphere() -> Arc<Mesh> {
        Arc::new(primitives::sphere(1.0, 48, 24))
    }

    /// A flat square whose UVs cover the whole set
    fn plane() -> Arc<Mesh> {
        Arc::new(primitives::plane(1.0, 1))
    }

    /// A busy sRGB colour image with alpha, stored as bytes and larger than the set, and a float
    /// image beyond 0..1
    fn images() -> ImageStore {
        const COLOR_SIDE: u32 = 256;
        const FLOAT_SIDE: u32 = 96;
        let color = (0..COLOR_SIDE * COLOR_SIDE)
            .map(|index| {
                let (x, y) = (index % COLOR_SIDE, index / COLOR_SIDE);
                let checker = ((x / 3 + y / 5) % 2) as f32;
                [x as f32 / 255.0, y as f32 / 255.0, checker, (64 + (x + 2 * y) % 192) as f32 / 255.0]
            })
            .collect();
        let float = (0..FLOAT_SIDE * FLOAT_SIDE)
            .map(|index| {
                let (x, y) = (index % FLOAT_SIDE, index / FLOAT_SIDE);
                let value = ((x * 13 + y * 7) % 101) as f32 / 97.0 - 0.1;
                [value, 1.0 - value, value * 0.5, 1.0]
            })
            .collect();
        ImageStore::from([
            ("color".into(), Arc::new(ImageRes::new(COLOR_SIDE, COLOR_SIDE, color, true))),
            ("float".into(), Arc::new(ImageRes::new(FLOAT_SIDE, FLOAT_SIDE, float, false))),
        ])
    }

    /// A base fill under a fill projected by `projection` with `sampling` in every source kind
    /// (sRGB image colour, pattern, uniform, float image height), its colour blended Multiply at
    /// 0.7, inverted after. Its mask lays the projected image over a uniform fill, then a disabled
    /// projected pattern and an inversion. Returns the project and the projected layer.
    fn fill_project(projection: &Projection, sampling: Sampling) -> (Project, LayerId) {
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Metallic, Channel::Height];
        let mut project = Project::new("Fill sampling", "builtin:sphere");
        project.texture_sets.push(TextureSet::new("Set", 0, SIZE));
        let base_sources = BTreeMap::from([
            (Channel::BaseColor, Source::Uniform([0.62, 0.48, 0.3])),
            (Channel::Roughness, Source::Uniform([0.55; 3])),
            (Channel::Height, Source::Uniform([0.1; 3])),
        ]);
        let base_id = project.alloc_id();
        let base = Layer::new(base_id, "Base", LayerKind::Fill(Fill { sources: base_sources, ..Fill::default() }), &channels);
        assert!(project.insert_layer(0, base, None, false).is_ok());

        let pattern = Procedural { scale: 3.7, seed: 19, ..Procedural::new(ProceduralKind::Clouds) };
        let pattern = Source::Procedural { pattern, low: [0.1; 3], high: [0.9; 3], ramp: Vec::new() };
        let sources = BTreeMap::from([
            (Channel::BaseColor, Source::image("color")),
            (Channel::Roughness, pattern.clone()),
            (Channel::Metallic, Source::Uniform([0.8; 3])),
            (Channel::Height, Source::image("float")),
        ]);
        let owner = project.alloc_id();
        let fill = Fill { sources, projection: projection.clone(), sampling, ..Fill::default() };
        let mut layer = Layer::new(owner, "Projected", LayerKind::Fill(fill), &channels);
        layer.channels.insert(Channel::BaseColor, ChannelBlend { mode: BlendMode::Multiply, opacity: 0.7 });
        let mut invert = Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Invert));
        invert.opacity = 0.6;
        layer.content_effects.push(invert);

        let mut mask = Mask::new(MaskBackground::Black);
        let under = EffectKind::Fill { source: Source::Uniform([0.35; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE };
        mask.effects.push(Effect::new(project.alloc_id(), under));
        let mut projected = Effect::new(project.alloc_id(), EffectKind::Fill { source: Source::image("color"), projection: projection.clone(), sampling });
        projected.opacity = 0.8;
        projected.blend = BlendMode::Lighten;
        mask.effects.push(projected);
        let mut disabled = Effect::new(project.alloc_id(), EffectKind::Fill { source: pattern, projection: projection.clone(), sampling });
        disabled.blend = BlendMode::Disable;
        mask.effects.push(disabled);
        let mut mask_invert = Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Invert));
        mask_invert.opacity = 0.5;
        mask.effects.push(mask_invert);
        layer.mask = Some(mask);
        assert!(project.insert_layer(0, layer, None, false).is_ok());
        (project, owner)
    }

    /// `fill_project` for `projection` and `sampling` compiles, takes P1's path, and (with an
    /// adapter) matches the CPU in every map and the mask preview.
    fn assert_fill_matches(gpu: &mut Option<Headless>, mesh: &Arc<Mesh>, projection: &Projection, sampling: Sampling, tolerance: Tolerance, label: &str) {
        let (project, owner) = fill_project(projection, sampling);
        let pixels = PixelStore::default();
        let program = CompositeProgram::try_compile_region(&project, &pixels, 0, None);
        let mask = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None);
        assert_eq!(program.as_ref().err(), None, "{label}");
        assert_eq!(mask.as_ref().err(), None, "{label} mask");
        assert!(program.is_ok_and(|program| program.operations.iter().any(|op| op.ext[1] != 0)), "{label}: no fill takes P1's path");
        let Some(gpu) = gpu else { return };

        let (baked, images) = (BakedMaps::default(), images());
        let mut case = OracleCase::new(&project, &pixels, &baked, &images, mesh);
        case.mask_owner = Some(owner);
        assert_matches_cpu(gpu, &case, tolerance, label);
    }

    /// A frame in front of a unit sphere, facing +Z, turned and moved a little
    fn frame() -> Decal {
        Decal {
            center: [0.0, 0.0, 0.9],
            normal: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
            size: [0.9, 0.7],
            rotation: 17.0,
            offset: [0.05, -0.04],
            depth: 0.6,
            symmetry: None,
            flip: [false, true],
        }
    }

    fn planar(projector: Projector) -> Projection {
        Projection::Planar { scale: 0.8, axis: 2, offset: [0.1, -0.05, 0.0], uv: UvTransform::default(), projector, placed: None }
    }

    /// P1's cases that still composite on the CPU, as mask effects (the content and the mask
    /// preview both compile them). Port one: delete its row (this test fails while a ported row
    /// stays) and add its oracle test. A cropped warp that takes its image's alpha waits for a
    /// frame record that carries a warp's images (`register_decal` in gpu_composite.rs).
    fn still_on_cpu() -> Vec<EffectKind> {
        let cropped = Projector { crop: ShapeCrop::Cropped, ..Projector::default() };
        let warp = Projection::Warp {
            frame: frame(),
            corners: Projection::FRAME_CORNERS,
            depth: Projection::WARP_DEPTH,
            uv: UvTransform::default(),
            projector: cropped,
        };
        vec![EffectKind::Fill { source: Source::Uniform([0.5; 3]), projection: warp, sampling: Sampling::SAVED_BEFORE }]
    }

    /// The placement hooks (P0b) leave every source kind's operation as before: no package
    /// word, no side-table words, the same projection selector, images registered as material.
    #[test]
    fn placement_hooks_leave_the_operations_as_before() {
        use texturecraft_core::{ChannelKind, NormalFormat, Procedural, ProceduralKind};

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
        assert_eq!(super::image_use(Sampling::SAVED_BEFORE, ChannelKind::Color), ImageUse::Material);
        assert_eq!(super::image_use(Sampling::default(), ChannelKind::Color), ImageUse::MipmappedColor);
        assert_eq!(super::image_use(Sampling::default(), ChannelKind::Scalar), ImageUse::Mipmapped);
    }

    /// Image sources read the nearest texel through every projection the built-in paths have
    #[test]
    fn nearest_image_fills_match_the_cpu() {
        let mut gpu = headless();
        let nearest = Sampling { filtering: Filtering::Nearest, ..Sampling::SAVED_BEFORE };
        let triplanar =
            Projection::Triplanar { scale: 0.7, sharpness: 3.0, offset: [0.1, -0.2, 0.05], uv: UvTransform::default(), projector: Projector::default() };
        let spherical = Projection::Spherical { scale: 1.2, uv: UvTransform::default(), offset: [0.0; 3], projector: Projector::default() };
        let cylindrical =
            Projection::Cylindrical { scale: 0.9, axis: 0, uv: UvTransform::default(), offset: [0.0; 3], projector: Projector::default(), angle: 360.0 };
        let cases = [
            ("UV", plane(), Projection::Uv { scale: [1.3, 0.85], rotation: 21.0, offset: [0.17, -0.31] }),
            ("UV tile", plane(), Projection::UvTile),
            ("tri-planar", sphere(), triplanar),
            ("planar", sphere(), planar(Projector::default())),
            ("spherical", sphere(), spherical),
            ("cylindrical", sphere(), cylindrical),
        ];
        for (label, mesh, projection) in cases {
            assert_fill_matches(&mut gpu, &mesh, &projection, nearest, Tolerance::BILINEAR.with_flips(FLIPS), &format!("nearest {label}"));
        }
    }

    /// Every wrap stops the fill at its tile's edges on the axes that do not repeat, and stops
    /// image reads at the edge texel there; tri-planar never leaves its tile but clamps its reads
    #[test]
    fn uv_wrap_matches_the_cpu() {
        let mut gpu = headless();
        let uv = Projection::Uv { scale: [1.31, 0.83], rotation: 9.0, offset: [0.17, -0.12] };
        let triplanar =
            Projection::Triplanar { scale: 0.9, sharpness: 2.0, offset: [0.4, -0.2, 0.3], uv: UvTransform::default(), projector: Projector::default() };
        let spherical = Projection::Spherical { scale: 1.7, uv: UvTransform::default(), offset: [0.0; 3], projector: Projector::default() };
        let cylindrical =
            Projection::Cylindrical { scale: 1.4, axis: 1, uv: UvTransform::default(), offset: [0.0; 3], projector: Projector::default(), angle: 360.0 };
        for wrap in [UvWrap::RepeatHorizontally, UvWrap::RepeatVertically, UvWrap::None] {
            let sharp = Sampling { wrap, ..Sampling::SAVED_BEFORE };
            let cases = [
                ("UV", plane(), uv.clone(), sharp),
                ("UV nearest", plane(), uv.clone(), Sampling { filtering: Filtering::Nearest, ..sharp }),
                ("UV tile", plane(), Projection::UvTile, sharp),
                ("tri-planar", sphere(), triplanar.clone(), sharp),
                ("planar", sphere(), planar(Projector::default()), sharp),
                ("spherical", sphere(), spherical.clone(), sharp),
                ("cylindrical", sphere(), cylindrical.clone(), sharp),
            ];
            for (label, mesh, projection, sampling) in cases {
                assert_fill_matches(&mut gpu, &mesh, &projection, sampling, Tolerance::BILINEAR.with_flips(FLIPS), &format!("{wrap:?} {label}"));
            }
        }
    }

    /// A cylindrical projection over less than a full turn spreads one repeat over its arc, round
    /// every axis, for images and patterns
    #[test]
    fn cylindrical_angles_match_the_cpu() {
        let mut gpu = headless();
        for (axis, angle) in [(0, 120.0), (1, 45.0), (2, 270.0), (1, 10.0)] {
            let projection =
                Projection::Cylindrical { scale: 1.1, axis, uv: UvTransform::default(), offset: [0.05, 0.1, -0.1], projector: Projector::default(), angle };
            let label = format!("cylindrical axis {axis} over {angle} degrees");
            assert_fill_matches(&mut gpu, &sphere(), &projection, Sampling::SAVED_BEFORE, Tolerance::BILINEAR, &label);
        }
    }

    /// The 3D projections that turn the CPU's way, every one of them with a UV transformation
    /// and a projector turned and stretched
    fn turned_projections(uv: UvTransform, projector: Projector) -> [(&'static str, Projection); 6] {
        [
            ("tri-planar", Projection::Triplanar { scale: 0.8, sharpness: 5.0, offset: [0.2, -0.1, 0.3], uv, projector }),
            ("planar X", Projection::Planar { scale: 0.9, axis: 0, offset: [0.1, 0.2, -0.1], uv, projector, placed: None }),
            ("planar Y", Projection::Planar { scale: 1.2, axis: 1, offset: [-0.2, 0.1, 0.3], uv, projector, placed: None }),
            ("planar Z", Projection::Planar { scale: 0.7, axis: 2, offset: [0.3, -0.3, 0.0], uv, projector, placed: None }),
            ("spherical", Projection::Spherical { scale: 1.3, uv, offset: [0.1, 0.0, -0.2], projector }),
            ("cylindrical", Projection::Cylindrical { scale: 1.1, axis: 2, uv, offset: [0.0, 0.2, 0.1], projector, angle: 200.0 }),
        ]
    }

    /// A UV transformation tiles, turns and moves the source on every 3D projection
    #[test]
    fn uv_transformations_match_the_cpu() {
        let mut gpu = headless();
        let uv = UvTransform { tiling: [1.7, 0.6], rotation: 23.0, offset: [0.11, -0.3] };
        for (label, projection) in turned_projections(uv, Projector::default()) {
            assert_fill_matches(&mut gpu, &sphere(), &projection, Sampling::SAVED_BEFORE, Tolerance::BILINEAR, &format!("UV transformation {label}"));
        }
    }

    /// A projector's rotation and per-axis scale (one of them flipped) turn and stretch every 3D
    /// projection, alone and with a UV transformation
    #[test]
    fn projector_rotation_and_scale_match_the_cpu() {
        let mut gpu = headless();
        let uv = UvTransform { tiling: [0.8, 1.4], rotation: -35.0, offset: [0.2, 0.05] };
        let rotated = Projector { rotation: [20.0, 35.0, -10.0], ..Projector::default() };
        let stretched = Projector { scale: [1.2, -0.8, 0.6], ..Projector::default() };
        let both = Projector { rotation: [-15.0, 60.0, 5.0], scale: [0.7, 1.3, 1.1], ..Projector::default() };
        for (name, projector, uv) in
            [("rotated", rotated, UvTransform::default()), ("stretched", stretched, UvTransform::default()), ("turned and placed", both, uv)]
        {
            for (label, projection) in turned_projections(uv, projector) {
                assert_fill_matches(&mut gpu, &sphere(), &projection, Sampling::SAVED_BEFORE, Tolerance::BILINEAR, &format!("{name} {label}"));
            }
        }
    }

    /// Cropped to its shape, each 3D projection covers only the texels inside its box, frame,
    /// sphere or cylinder (sized here so the shape's edge crosses the sphere)
    #[test]
    fn shape_crops_match_the_cpu() {
        let mut gpu = headless();
        let cropped = Projector { crop: ShapeCrop::Cropped, ..Projector::default() };
        let shrunk = Projector { scale: [0.45, 0.5, 0.4], ..cropped };
        let uv = UvTransform::default();
        let cases = [
            ("tri-planar", Projection::Triplanar { scale: 0.8, sharpness: 4.0, offset: [0.0; 3], uv, projector: shrunk }),
            ("planar X", Projection::Planar { scale: 2.0, axis: 0, offset: [0.1, 0.2, -0.1], uv, projector: cropped, placed: None }),
            ("planar Y", Projection::Planar { scale: 1.9, axis: 1, offset: [-0.2, 0.0, 0.3], uv, projector: cropped, placed: None }),
            ("planar Z", Projection::Planar { scale: 2.2, axis: 2, offset: [0.3, -0.3, 0.0], uv, projector: cropped, placed: None }),
            ("spherical", Projection::Spherical { scale: 1.3, uv, offset: [0.6, 0.0, -0.2], projector: cropped }),
            ("cylindrical", Projection::Cylindrical { scale: 1.1, axis: 1, uv, offset: [0.0, 0.2, 0.1], projector: shrunk, angle: 360.0 }),
        ];
        for (label, projection) in cases {
            assert_fill_matches(&mut gpu, &sphere(), &projection, Sampling::SAVED_BEFORE, Tolerance::BILINEAR.with_flips(FLIPS), &format!("cropped {label}"));
        }
    }

    /// Backface culling fades off planar and cylindrical projections on faces turned away, hard
    /// and soft, also inside a cropped and turned projector
    #[test]
    fn backface_culling_matches_the_cpu() {
        let mut gpu = headless();
        let uv = UvTransform::default();
        for backface in [
            Backface { on: true, angle: 60.0, hardness: 0.0 },
            Backface { on: true, angle: 120.0, hardness: 1.0 },
            Backface { on: true, ..Backface::default() },
        ] {
            let culled = Projector { backface, ..Projector::default() };
            let turned = Projector { rotation: [10.0, -25.0, 15.0], crop: ShapeCrop::Cropped, backface, ..Projector::default() };
            let cases = [
                ("planar X", Projection::Planar { scale: 0.9, axis: 0, offset: [0.0; 3], uv, projector: culled, placed: None }),
                ("planar Y", Projection::Planar { scale: 0.9, axis: 1, offset: [0.0; 3], uv, projector: culled, placed: None }),
                ("planar Z turned", Projection::Planar { scale: 1.6, axis: 2, offset: [0.1, 0.0, 0.0], uv, projector: turned, placed: None }),
                ("cylindrical X", Projection::Cylindrical { scale: 1.0, axis: 0, uv, offset: [0.0; 3], projector: culled, angle: 360.0 }),
                ("cylindrical Z turned", Projection::Cylindrical { scale: 1.0, axis: 2, uv, offset: [0.1, 0.0, 0.0], projector: turned, angle: 360.0 }),
            ];
            for (label, projection) in cases {
                let label = format!("backface {} degrees hardness {} {label}", backface.angle, backface.hardness);
                assert_fill_matches(&mut gpu, &sphere(), &projection, Sampling::SAVED_BEFORE, Tolerance::BILINEAR.with_flips(FLIPS), &label);
            }
        }
    }

    /// A planar projection placed on the surface with other projector settings than a decal's:
    /// reaching past its frame, culling at another angle, ignoring its image's alpha, turned,
    /// with a UV transformation, and with a mirrored copy
    #[test]
    fn placed_frames_match_the_cpu() {
        let mut gpu = headless();
        let mirrored = Decal { symmetry: Some(texturecraft_core::DecalSymmetry { axis: 2, flip: texturecraft_core::DecalFlip::U, offset: 0.1 }), ..frame() };
        let soft = Backface { on: true, angle: 70.0, hardness: 0.2 };
        let cases = [
            ("reaching past its frame", frame(), Projector::default(), UvTransform::default()),
            ("culled softly", frame(), Projector { backface: soft, crop: ShapeCrop::Cropped, ..Projector::default() }, UvTransform::default()),
            (
                "ignoring its image alpha",
                frame(),
                Projector { crop: ShapeCrop::Cropped, ignore_image_alpha: true, ..Projector::default() },
                UvTransform::default(),
            ),
            (
                "turned and placed",
                frame(),
                Projector { rotation: [0.0, 0.0, 25.0], scale: [1.3, 0.8, 1.0], crop: ShapeCrop::Cropped, backface: soft, ..Projector::default() },
                UvTransform { tiling: [1.5, 1.2], rotation: 15.0, offset: [0.1, 0.0] },
            ),
            ("mirrored", mirrored, Projector { crop: ShapeCrop::Cropped, backface: soft, ..Projector::default() }, UvTransform::default()),
        ];
        for (label, frame, projector, uv) in cases {
            let projection = Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv, projector, placed: Some(frame) };
            assert_fill_matches(
                &mut gpu,
                &sphere(),
                &projection,
                Sampling::SAVED_BEFORE,
                Tolerance::BILINEAR.with_flips(FLIPS),
                &format!("placed frame {label}"),
            );
        }
    }

    /// Mirrored and radial copies of 3D projections, moved off the centre and flipped, laid
    /// over each other with the texel's own copy on top, cropped so copies overlap partly
    #[test]
    fn fill_symmetry_matches_the_cpu() {
        let mut gpu = headless();
        let on = FillSymmetry { on: true, ..FillSymmetry::OFF };
        let symmetries = [
            ("mirror X", on),
            ("mirror Y off centre, flipped", FillSymmetry { axis: 1, position: [0.0, 0.2, 0.0], flip: [true, false], ..on }),
            ("mirror Z", FillSymmetry { axis: 2, flip: [false, true], ..on }),
            ("radial Y, 5", FillSymmetry { kind: SymmetryKind::Radial, axis: 1, count: 5, ..on }),
            ("radial Z, 4 over 180", FillSymmetry { kind: SymmetryKind::Radial, axis: 2, count: 4, span: 180.0, flip: [true, true], ..on }),
            ("radial X, 3 off centre", FillSymmetry { kind: SymmetryKind::Radial, axis: 0, count: 3, position: [0.1, -0.2, 0.3], ..on }),
        ];
        let uv = UvTransform::default();
        let cropped = Projector { crop: ShapeCrop::Cropped, ..Projector::default() };
        let projections = [
            ("planar", Projection::Planar { scale: 1.6, axis: 0, offset: [0.2, 0.1, 0.0], uv, projector: cropped, placed: None }),
            ("tri-planar", Projection::Triplanar { scale: 0.8, sharpness: 4.0, offset: [0.1, 0.0, 0.2], uv, projector: Projector::default() }),
            ("spherical", Projection::Spherical { scale: 1.2, uv, offset: [0.4, 0.0, 0.1], projector: cropped }),
            ("decal", Projection::decal(frame())),
        ];
        for (name, symmetry) in symmetries {
            for (label, projection) in &projections {
                let sampling = Sampling { symmetry, ..Sampling::SAVED_BEFORE };
                assert_fill_matches(&mut gpu, &sphere(), projection, sampling, Tolerance::BILINEAR.with_flips(FLIPS), &format!("{name} {label}"));
            }
        }
    }

    /// A warp bends the source to the quad its pulled corners make, reaching past it or cropped
    /// to it and its Projection Depth, turned, and with a UV transformation
    #[test]
    fn warp_projections_match_the_cpu() {
        let mut gpu = headless();
        let pulled = [[-0.6, 0.45], [0.55, 0.6], [0.4, -0.5], [-0.45, -0.4]];
        let parallel = [[-0.5, 0.5], [0.6, 0.5], [0.6, -0.5], [-0.5, -0.5]];
        let cropped = Projector { crop: ShapeCrop::Cropped, ignore_image_alpha: true, ..Projector::default() };
        let turned = Projector { rotation: [0.0, 0.0, -20.0], scale: [1.2, 0.9, 1.0], ..cropped };
        let cases = [
            ("pulled", pulled, 0.3, Projector::default(), UvTransform::default()),
            ("parallel", parallel, 0.3, Projector::default(), UvTransform::default()),
            ("cropped", pulled, 0.4, cropped, UvTransform::default()),
            ("turned and placed", pulled, 0.5, turned, UvTransform { tiling: [1.4, 0.7], rotation: 30.0, offset: [0.2, -0.1] }),
        ];
        for (label, corners, depth, projector, uv) in cases {
            let projection = Projection::Warp { frame: frame(), corners, depth, uv, projector };
            assert_fill_matches(&mut gpu, &sphere(), &projection, Sampling::SAVED_BEFORE, Tolerance::BILINEAR.with_flips(FLIPS), &format!("warp {label}"));
        }
    }

    /// Bilinear HQ, the default of new image fills, reads the image level its footprint shrinks
    /// it to (linear levels for sRGB colour), through every projection, with wrap and copies
    #[test]
    fn bilinear_hq_matches_the_cpu() {
        let mut gpu = headless();
        let hq = Sampling { filtering: Filtering::BilinearHq, ..Sampling::SAVED_BEFORE };
        let uv = UvTransform::default();
        let mirror = FillSymmetry { on: true, ..FillSymmetry::OFF };
        let cases = [
            ("UV shrunk", plane(), Projection::Uv { scale: [3.3, 2.1], rotation: 17.0, offset: [0.1, -0.2] }, hq),
            ("UV between levels", plane(), Projection::Uv { scale: [1.45, 1.45], rotation: 0.0, offset: [0.0; 2] }, hq),
            ("UV enlarged", plane(), Projection::Uv { scale: [0.3, 0.4], rotation: -40.0, offset: [0.3, 0.2] }, hq),
            ("UV tile", plane(), Projection::UvTile, hq),
            ("UV wrap none", plane(), Projection::Uv { scale: [2.6, 1.2], rotation: 5.0, offset: [0.2, 0.1] }, Sampling { wrap: UvWrap::None, ..hq }),
            ("tri-planar", sphere(), Projection::Triplanar { scale: 2.5, sharpness: 3.0, offset: [0.1, 0.0, -0.1], uv, projector: Projector::default() }, hq),
            ("planar", sphere(), Projection::Planar { scale: 2.8, axis: 1, offset: [0.0; 3], uv, projector: Projector::default(), placed: None }, hq),
            ("spherical", sphere(), Projection::Spherical { scale: 3.0, uv, offset: [0.0; 3], projector: Projector::default() }, hq),
            ("cylindrical", sphere(), Projection::Cylindrical { scale: 2.0, axis: 1, uv, offset: [0.0; 3], projector: Projector::default(), angle: 90.0 }, hq),
            (
                "mirrored planar",
                sphere(),
                Projection::Planar { scale: 2.4, axis: 0, offset: [0.1, 0.0, 0.0], uv, projector: Projector::default(), placed: None },
                Sampling { symmetry: mirror, ..hq },
            ),
            ("library defaults", plane(), Projection::default(), Sampling::default()),
        ];
        for (label, mesh, projection, sampling) in cases {
            assert_fill_matches(&mut gpu, &mesh, &projection, sampling, Tolerance::BILINEAR, &format!("bilinear HQ {label}"));
        }
    }

    /// Decals and other placed frames read their images and the alpha that crops them with
    /// Nearest and Bilinear HQ, also with wrap and copies
    #[test]
    fn decal_filtering_matches_the_cpu() {
        let mut gpu = headless();
        let shrunk = Decal { size: [0.35, 0.3], ..frame() };
        let mirror = FillSymmetry { on: true, ..FillSymmetry::OFF };
        for filtering in [Filtering::Nearest, Filtering::BilinearHq] {
            let sampling = Sampling { filtering, ..Sampling::SAVED_BEFORE };
            let decal = Projector { crop: ShapeCrop::Cropped, backface: Backface { on: true, ..Backface::default() }, ..Projector::default() };
            let turned = Projector { rotation: [0.0, 0.0, 40.0], ..decal };
            let cases = [
                ("decal", Projection::decal(frame()), sampling),
                ("shrunk decal", Projection::decal(shrunk), sampling),
                (
                    "turned frame",
                    Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv: UvTransform::default(), projector: turned, placed: Some(shrunk) },
                    sampling,
                ),
                ("decal, wrap none", Projection::decal(frame()), Sampling { wrap: UvWrap::None, ..sampling }),
                ("mirrored decal", Projection::decal(frame()), Sampling { symmetry: mirror, ..sampling }),
            ];
            for (label, projection, sampling) in cases {
                // The frame's edge crops hard
                assert_fill_matches(&mut gpu, &sphere(), &projection, sampling, Tolerance::BILINEAR.with_flips(FLIPS), &format!("{filtering:?} {label}"));
            }
        }
    }

    /// Depth culling, the default of new planar and warp fills, hides surfaces behind the one
    /// nearest the projector through the CPU's own grid, read from the atlas's float planes:
    /// planar axes, hard and soft, turned, placed frames, warps and symmetry copies' own grids
    #[test]
    fn depth_culling_matches_the_cpu() {
        let mut gpu = headless();
        let uv = UvTransform::default();
        let culled = Projector::depth_culled();
        let hard = Projector { depth: DepthCulling { on: true, hardness: 1.0 }, ..Projector::default() };
        let soft = Projector { depth: DepthCulling { on: true, hardness: 0.0 }, ..Projector::default() };
        let turned = Projector { rotation: [30.0, -20.0, 0.0], scale: [1.2, 1.0, 0.8], ..culled };
        let decal = Projector { crop: ShapeCrop::Cropped, backface: Backface { on: true, ..Backface::default() }, ..culled };
        let mirror = FillSymmetry { on: true, axis: 1, ..FillSymmetry::OFF };
        let radial = FillSymmetry { on: true, kind: SymmetryKind::Radial, axis: 1, count: 3, ..FillSymmetry::OFF };
        let sharp = Sampling::SAVED_BEFORE;
        let cases = [
            ("planar X", Projection::Planar { scale: 0.9, axis: 0, offset: [0.0; 3], uv, projector: culled, placed: None }, sharp),
            ("planar Y hard", Projection::Planar { scale: 0.9, axis: 1, offset: [0.0; 3], uv, projector: hard, placed: None }, sharp),
            ("planar Z soft", Projection::Planar { scale: 0.9, axis: 2, offset: [0.0; 3], uv, projector: soft, placed: None }, sharp),
            ("planar turned", Projection::Planar { scale: 1.1, axis: 2, offset: [0.1, 0.0, 0.0], uv, projector: turned, placed: None }, sharp),
            ("placed frame", Projection::Planar { scale: 1.0, axis: 2, offset: [0.0; 3], uv, projector: decal, placed: Some(frame()) }, sharp),
            ("warp", Projection::Warp { frame: frame(), corners: Projection::FRAME_CORNERS, depth: 0.5, uv, projector: culled }, sharp),
            (
                "mirrored planar",
                Projection::Planar { scale: 0.9, axis: 0, offset: [0.2, 0.0, 0.0], uv, projector: culled, placed: None },
                Sampling { symmetry: mirror, ..sharp },
            ),
            (
                "radial planar",
                Projection::Planar { scale: 0.9, axis: 2, offset: [0.0; 3], uv, projector: culled, placed: None },
                Sampling { symmetry: radial, ..sharp },
            ),
            ("planar bilinear HQ", Projection::Planar { scale: 2.5, axis: 2, offset: [0.0; 3], uv, projector: culled, placed: None }, Sampling::default()),
        ];
        for (label, projection, sampling) in cases {
            assert_fill_matches(&mut gpu, &sphere(), &projection, sampling, Tolerance::BILINEAR.with_flips(FLIPS), &format!("depth culled {label}"));
        }
    }

    #[test]
    fn fill_sampling_and_projections_still_on_the_cpu_fall_back() {
        for kind in still_on_cpu() {
            let (material, mask) = compile_effects(None, Some(effect(kind.clone())), ContentsOffset::NONE);
            assert!(material.is_err() && mask.is_err(), "{kind:?}");
        }
    }
}
