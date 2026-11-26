//! P2 of the generator GPU port: Micro Details. A generator's micro height and normal add
//! curvature and occlusion to its mesh maps; compose's `micro_detail/mod.rs` builds those maps
//! once per map, image and settings, in the bake's layout. Here the GPU builds them
//! (`shaders/generator_p2_micro.wgsl`) before the composite reads them
//! (`shaders/generator_p2.wgsl`), and the curvature detail sizes blur the bake with the micro
//! curvature added, as `GeneratorMaps::of` blurs `MicroCurvature::curvature`.
//!
//! A generator's P2 record (`Operation::uv[3]`, the side-table offset; 0: none) holds its micro
//! curvature layer plus one, its micro occlusion layer plus one, then its texture and grunge
//! records (0: none). The texture and the grunge are read on the new model's section and wear
//! controls alone (P1's paths call `generator_texture` and `generator_grunge` in WGSL), so on the
//! earlier model their records stay 0.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use bytemuck::{Pod, Zeroable};
use texturecraft_compose::{ImageRes, ImageStore};
use texturecraft_core::{
    ChannelKind, Generator, GeneratorGrayscale, GeneratorKind, MicroCurvatureType, MicroDetails, NormalFormat, Procedural, ProceduralKind, Projection,
    Sampling, Source, TextureBlend, WearParameters,
};
use wgpu::util::DeviceExt;

use super::{CompositeProgram, ImageUse, ProceduralInputs, source_operation};
use crate::gpu_trace::{entry, storage};

/// Words of a generator's P2 record
const RECORD_WORDS: usize = 4;

/// Layers of the micro map texture: two per generator (its curvature term, its occlusion factor)
const MAX_MICRO_LAYERS: u32 = 256;

/// How steeply the micro height tilts the normal it is turned into, per unit of Height Details
/// Intensity (compose's `HEIGHT_NORMAL_STRENGTH`)
const HEIGHT_NORMAL_STRENGTH: f32 = 5.9;

/// Blur sizes of the Smooth type, in texels at 2K (compose's `SMOOTH_SIGMAS`)
const SMOOTH_SIGMAS: [f32; 7] = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];

/// Below this sigma a blur is an explicit kernel; wider ones are three box passes
const BOX_BLUR_MIN_SIGMA: f32 = 2.0;

/// How many sigmas the sampled kernel reaches each side
const KERNEL_REACH_SIGMAS: f32 = 3.0;

/// The widest kernel the parameters hold: a sigma below [`BOX_BLUR_MIN_SIGMA`] reaches 6 texels
const KERNEL_TAPS: usize = 16;

/// Directions the occlusion looks along, and its most steps along each
const OCCLUSION_DIRECTIONS: usize = 16;
const OCCLUSION_STEPS: usize = 16;

/// The occlusion radius in texels at 1K for AO Radius 1, its exponent, and the shortest radius
/// that occludes (compose's `OCCLUSION_RADIUS_*`)
const OCCLUSION_RADIUS_TEXELS_1K: f32 = 47.675;
const OCCLUSION_RADIUS_POWER: f32 = 1.663;
const MIN_OCCLUSION_RADIUS_TEXELS_1K: f32 = 2.0;

/// How steep the micro height reads for occlusion per texel at 1K at AO Depth 1, and its exponent
const OCCLUSION_DEPTH_SCALE_1K: f32 = 1473.8;
const OCCLUSION_DEPTH_POWER: f32 = 1.0654;

/// A UV tile's texels at the reference resolution (compose's `REFERENCE_TILE_TEXELS`)
const REFERENCE_TILE_TEXELS: f32 = 2048.0;

const WORKGROUP: u32 = 8;
const LINE_WORKGROUP: u32 = 64;

/// The slot bits of `Params::scales[3]`: the height, the normal
const READS_HEIGHT: u32 = 1;
const READS_NORMAL: u32 = 2;

/// What a micro details slot reads, as compose's `Slot::of` reads it
#[derive(Clone, Debug, PartialEq)]
enum MicroSlot {
    Uniform([f32; 3]),
    Image { name: String, flip_green: bool },
}

impl MicroSlot {
    /// The slot holding `source`: an empty slot, a procedural and an anchor read black. None for
    /// an anchor read as the height (`height_anchor`), whose height the GPU does not gather.
    fn of(source: Option<&Source>, height_anchor: bool) -> Option<MicroSlot> {
        match source {
            Some(Source::Uniform(value)) => Some(MicroSlot::Uniform(*value)),
            Some(Source::Image { name, normal_format }) => Some(MicroSlot::Image { name: name.clone(), flip_green: *normal_format == NormalFormat::DirectX }),
            Some(Source::Anchor { .. }) if height_anchor => None,
            _ => Some(MicroSlot::Uniform([0.0; 3])),
        }
    }
}

/// One generator's micro details, as the maps the GPU builds for it
#[derive(Clone, Debug, PartialEq)]
struct MicroRequest {
    micro: MicroDetails,
    height: Option<MicroSlot>,
    normal: Option<MicroSlot>,
    /// Build the curvature term (the generator reads its curvature input)
    curvature: bool,
    /// Build the occlusion factor (it reads its occlusion input and has a micro height)
    occlusion: bool,
}

/// The micro details a program's generators read, and the curvature its detail sizes blur
#[derive(Debug, Default)]
pub(super) struct MicroPlan {
    requests: Vec<MicroRequest>,
    /// The curvature every detail size reader reads: None unclaimed, Some(None) the bake's,
    /// Some(Some(i)) the bake with request `i`'s micro curvature
    levels: Option<Option<usize>>,
    /// Each request's height and normal image, once the program's images are set
    images: Vec<[Option<Arc<ImageRes>>; 2]>,
    /// Side-table words (indices into the table after its lookup words) that hold a generator
    /// source's local index plus one, made absolute by [`bind_sources`]
    source_words: Vec<usize>,
}

impl MicroPlan {
    /// Whether any generator reads micro maps
    pub(super) fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Detail sizes read from `source` (None: the bake's own curvature, else a request's micro
    /// curvature): None when another reader already took another curvature, whose sizes the one
    /// curvature levels binding cannot hold at once (that program composes on the CPU)
    pub(super) fn claim_levels(&mut self, source: Option<usize>) -> Option<()> {
        match self.levels {
            None => {
                self.levels = Some(source);
                Some(())
            }
            Some(claimed) => (claimed == source).then_some(()),
        }
    }

    /// Resolve the requests' images (an image not loaded reads black, as the CPU's slot does)
    pub(super) fn set_images(&mut self, images: &ImageStore) {
        let image = |slot: &Option<MicroSlot>| match slot {
            Some(MicroSlot::Image { name, .. }) => images.get(name).cloned(),
            _ => None,
        };
        self.images = self.requests.iter().map(|request| [image(&request.height), image(&request.normal)]).collect();
    }
}

/// The micro maps an earlier-model generator reads: its curvature input (Curvature, Edge Wear)
/// and its occlusion input (Ambient Occlusion, Dirt). P1's wear controls read both.
fn reads_micro(kind: GeneratorKind) -> (bool, bool) {
    let curvature = MicroDetails::has_curvature(kind) && matches!(kind, GeneratorKind::Curvature | GeneratorKind::EdgeWear);
    let occlusion = MicroDetails::has_occlusion(kind) && matches!(kind, GeneratorKind::AmbientOcclusion | GeneratorKind::Dirt);
    (curvature, occlusion)
}

/// The generator's P2 record for `Operation::uv[3]` (0: none) and its micro details' request
/// (None without them); None when the GPU leaves something it reads to the CPU (a micro height
/// read from an anchor point, a texture or grunge slot holding a procedural or an anchor)
pub(super) fn generator_record(generator: &Generator, inputs: &mut ProceduralInputs) -> Option<(f32, Option<usize>)> {
    let (micro_words, micro) = micro_record(generator, inputs)?;
    let texture = texture_record(generator, inputs)?;
    let grunge = grunge_record(generator, inputs)?;
    let words = [micro_words[0], micro_words[1], texture, grunge];
    if words == [0; RECORD_WORDS] {
        return Some((0.0, None));
    }

    let offset = inputs.pack_words(&words)?;

    // The record's offset travels as an exact float
    (offset < 1 << 24).then_some((offset as f32, micro))
}

/// The micro curvature and occlusion layers (plus one; 0: none) of the generator's micro
/// details, and their request when it builds a curvature term
fn micro_record(generator: &Generator, inputs: &mut ProceduralInputs) -> Option<([u32; 2], Option<usize>)> {
    let Some(micro) = generator.micro_in_use() else { return Some(([0; 2], None)) };

    let micro = micro.clone().clamped(generator.kind);
    let height = if micro.height { Some(MicroSlot::of(micro.height_source.as_ref(), true)?) } else { None };
    let normal = if micro.normal { MicroSlot::of(micro.normal_source.as_ref(), false) } else { None };
    let (curvature, occlusion) = reads_micro(generator.kind);
    let request = MicroRequest { micro, curvature, occlusion: occlusion && height.is_some(), height, normal };
    if !request.curvature && !request.occlusion {
        return Some(([0; 2], None));
    }

    let plan = &mut inputs.micro;
    let index = match plan.requests.iter().position(|existing| *existing == request) {
        Some(index) => index,
        None => {
            if (plan.requests.len() as u32 + 1) * 2 > MAX_MICRO_LAYERS {
                return None;
            }
            plan.requests.push(request.clone());
            plan.requests.len() - 1
        }
    };
    let layer = |built: bool, offset: u32| if built { index as u32 * 2 + offset + 1 } else { 0 };
    Some(([layer(request.curvature, 0), layer(request.occlusion, 1)], request.curvature.then_some(index)))
}

/// Where an image slot is read: its scale and offset across the UV square (or the mesh's
/// bounds), its triplanar contrast (None: in UV), and whether the X projection mirrors about the
/// centre (compose's `SlotPlacement`)
struct SlotPlacement {
    scale: f32,
    offset: f32,
    triplanar_contrast: Option<f32>,
    mirrored_about_centre: bool,
}

/// The words of an image slot holding `source` (`p2_slot_color` in WGSL): an empty slot reads
/// black; None for a procedural or an anchor, which the GPU does not read there
fn slot_words(source: Option<&Source>, placement: &SlotPlacement, inputs: &mut ProceduralInputs) -> Option<[u32; 9]> {
    let (image, color) = match source {
        None => (0, [0.0; 3]),
        Some(Source::Uniform(value)) => (0, *value),
        Some(Source::Image { name, .. }) => (inputs.pack_image(name, ImageUse::Material)?, [0.0; 3]),
        Some(Source::Procedural { .. } | Source::Anchor { .. }) => return None,
    };
    let values = [color[0], color[1], color[2], placement.scale, placement.offset, placement.triplanar_contrast.unwrap_or(0.0)];
    if values.iter().any(|value| !value.is_finite()) {
        return None;
    }

    let [red, green, blue, scale, offset, contrast] = values.map(f32::to_bits);
    Some([image, red, green, blue, scale, offset, u32::from(placement.triplanar_contrast.is_some()), contrast, u32::from(placement.mirrored_about_centre)])
}

/// The record of the texture the generator blends over its signal (0: none): its slot, then its
/// grayscale mode, invert, contrast, brightness, blend and opacity (`generator_texture` in WGSL)
fn texture_record(generator: &Generator, inputs: &mut ProceduralInputs) -> Option<u32> {
    let Some(texture) = generator.texture_in_use() else { return Some(0) };

    // Scaled about the centre of the UV square (or of the mesh's bounds)
    let placement = SlotPlacement {
        scale: texture.scale,
        offset: 0.5 * (1.0 - texture.scale),
        triplanar_contrast: texture.triplanar.then_some(texture.triplanar_contrast),
        mirrored_about_centre: true,
    };
    let slot = slot_words(texture.source.as_ref(), &placement, inputs)?;
    let adjustments = [texture.contrast.clamp(-1.0, 1.0), texture.brightness.clamp(-1.0, 1.0), texture.opacity.clamp(0.0, 1.0)];
    if adjustments.iter().any(|value| !value.is_finite()) {
        return None;
    }

    let gray = GeneratorGrayscale::ALL.iter().position(|mode| *mode == texture.grayscale)? as u32;
    let blend = TextureBlend::ALL.iter().position(|blend| *blend == texture.blend)? as u32;
    let [contrast, brightness, opacity] = adjustments.map(f32::to_bits);
    let mut words = slot.to_vec();
    words.extend([gray, u32::from(texture.invert), contrast, brightness, blend, opacity]);
    inputs.pack_words(&words)
}

/// A wear kind's grunge pattern, its repeats across the UV square per unit of grunge scale, its
/// rotation, its breakup (pattern and repeats), its mean and spread, and a custom grunge's gain
/// (compose's `grunge_pattern.rs`)
struct GrungePattern {
    kind: ProceduralKind,
    repeats_per_scale: f32,
    rotation: f32,
    breakup: Option<(ProceduralKind, f32)>,
    mean: f32,
    spread: f32,
    custom_gain: f32,
}

const EDGE_WEAR_GRUNGE: GrungePattern =
    GrungePattern { kind: ProceduralKind::Scratches, repeats_per_scale: 4.0, rotation: 0.0, breakup: None, mean: 0.037, spread: 0.149, custom_gain: 6.22 };
const FIBER_GLASS_GRUNGE: GrungePattern = GrungePattern {
    kind: ProceduralKind::Weave,
    repeats_per_scale: 16.0,
    rotation: 45.0,
    breakup: Some((ProceduralKind::Grunge, 4.0)),
    mean: 0.539,
    spread: 0.198,
    custom_gain: 1.9,
};
const DIRT_GRUNGE: GrungePattern =
    GrungePattern { kind: ProceduralKind::Grunge, repeats_per_scale: 4.0, rotation: 0.0, breakup: None, mean: 0.682, spread: 0.242, custom_gain: -5.56 };

/// The triplanar seams' sharpness at triplanar contrast 0 and 1 (compose's wear.rs)
const TRIPLANAR_SHARPNESS: std::ops::RangeInclusive<f32> = 1.0..=16.0;

/// Where the reference app reads a custom grunge at the UV square's origin: half way across
const CUSTOM_GRUNGE_UV_OFFSET: f32 = 0.5;

/// The record of a wear generator's grunge (0: none): its own pattern's source operations, mean
/// and spread, or its custom grunge slot and gain (`generator_grunge` in WGSL)
fn grunge_record(generator: &Generator, inputs: &mut ProceduralInputs) -> Option<u32> {
    // Fiber Glass Edge Wear has no earlier model: it reads its default wear controls
    let wear = generator.wear.or_else(|| WearParameters::defaults(generator.kind).filter(|_| generator.kind == GeneratorKind::FiberGlassEdgeWear));
    let Some(wear) = wear else { return Some(0) };

    let pattern = match generator.kind {
        GeneratorKind::Dirt => &DIRT_GRUNGE,
        GeneratorKind::FiberGlassEdgeWear => &FIBER_GLASS_GRUNGE,
        _ => &EDGE_WEAR_GRUNGE,
    };
    if wear.custom_grunge {
        let placement = if wear.triplanar {
            SlotPlacement { scale: wear.grunge_scale, offset: 0.0, triplanar_contrast: Some(wear.triplanar_contrast), mirrored_about_centre: false }
        } else {
            SlotPlacement { scale: wear.grunge_scale, offset: CUSTOM_GRUNGE_UV_OFFSET, triplanar_contrast: None, mirrored_about_centre: false }
        };
        let mut words = vec![1];
        words.extend(slot_words(generator.grunge.as_ref(), &placement, inputs)?);
        words.push(pattern.custom_gain.to_bits());
        return inputs.pack_words(&words);
    }

    let sharpness = TRIPLANAR_SHARPNESS.start() + (TRIPLANAR_SHARPNESS.end() - TRIPLANAR_SHARPNESS.start()) * wear.triplanar_contrast;
    let projection = if wear.triplanar {
        Projection::Triplanar { scale: 1.0, sharpness, offset: [0.0; 3], uv: Default::default(), projector: Default::default() }
    } else {
        Projection::Uv { scale: [1.0, 1.0], rotation: pattern.rotation, offset: [0.0, 0.0] }
    };
    let source = |kind: ProceduralKind, repeats_per_scale: f32, inputs: &mut ProceduralInputs| -> Option<u32> {
        let pattern = Procedural { scale: repeats_per_scale * wear.grunge_scale, seed: generator.seed, ..Procedural::new(kind) };
        let source = Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() };
        let operation = source_operation(Some(&source), &projection, Sampling::default(), [0.0; 3], ChannelKind::Color, inputs)?;
        inputs.generator_sources.push(operation);
        Some(inputs.generator_sources.len() as u32)
    };
    let main = source(pattern.kind, pattern.repeats_per_scale, inputs)?;
    let breakup = match pattern.breakup {
        Some((kind, repeats_per_scale)) => source(kind, repeats_per_scale, inputs)?,
        None => 0,
    };
    let offset = inputs.pack_words(&[0, main, breakup, pattern.mean.to_bits(), pattern.spread.to_bits()])?;

    // The source words become absolute operation indices once the program lays out its sources
    let first = (offset - super::LOOKUP_WORDS) as usize;
    inputs.micro.source_words.push(first + 1);
    if breakup != 0 {
        inputs.micro.source_words.push(first + 2);
    }
    Some(offset)
}

/// Make the grunge records' source words absolute: the generator sources follow the program's
/// `operations` first operations (`bind_generator_sources`)
pub(super) fn bind_sources(inputs: &mut ProceduralInputs, operations: u32) {
    for &word in &inputs.micro.source_words {
        if let Some(value) = inputs.ramp_words.get_mut(word) {
            *value += operations;
        }
    }
}

/// The maps one compose reads: the micro map layers, and the bake with the claimed micro
/// curvature added for the detail sizes (None: they blur the bake itself)
pub(super) struct MicroMaps {
    pub(super) maps: wgpu::TextureView,
    pub(super) bake: Option<wgpu::TextureView>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    block: [u32; 4],
    slots: [u32; 4],
    uniforms: [f32; 4],
    origins: [u32; 4],
    place: [u32; 4],
    lines: [u32; 4],
    gains: [f32; 4],
    scales: [f32; 4],
    intensity: [f32; 4],
    kernel: [[f32; 4]; KERNEL_TAPS / 4],
    directions: [[f32; 4]; OCCLUSION_DIRECTIONS],
    taps: [[f32; 4]; OCCLUSION_DIRECTIONS * OCCLUSION_STEPS],
}

/// The fields `blur` reads and writes (`read_line` in WGSL)
const HEIGHTS: u32 = 0;
const FIRST: u32 = 1;
const SECOND: u32 = 2;

/// The blocks of one size in a map: the first origin, the step between origins, how many across
/// and down, and their size
struct BlockGroup {
    origin: [u32; 2],
    step: [u32; 2],
    count: [u32; 2],
    size: [u32; 2],
}

/// The UV tile blocks of a `size` map split `grid` ways, grouped by their clipped size (the
/// CPU's `by_blocks`: full blocks, then the clipped last column, row and corner)
fn block_groups(size: [u32; 2], grid: [u32; 2]) -> Vec<BlockGroup> {
    let block = [(size[0] / grid[0].max(1)).clamp(1, size[0]), (size[1] / grid[1].max(1)).clamp(1, size[1])];
    let full = [size[0] / block[0], size[1] / block[1]];
    let rest = [size[0] % block[0], size[1] % block[1]];
    let columns = [(0, full[0], block[0]), (full[0] * block[0], u32::from(rest[0] > 0), rest[0])];
    let rows = [(0, full[1], block[1]), (full[1] * block[1], u32::from(rest[1] > 0), rest[1])];
    rows.iter()
        .flat_map(|&(top, down, height)| {
            columns.iter().map(move |&(left, across, width)| BlockGroup { origin: [left, top], step: block, count: [across, down], size: [width, height] })
        })
        .filter(|group| group.count[0] > 0 && group.count[1] > 0)
        .collect()
}

/// A gaussian of `sigma` sampled at whole texels each side of its centre, its weights summing to
/// 1 (compose's `gaussian_kernel`)
fn gaussian_kernel(sigma: f32) -> Vec<f32> {
    let reach = (KERNEL_REACH_SIGMAS * sigma).ceil().max(1.0) as i64;
    let weights: Vec<f32> = (-reach..=reach).map(|offset| (-(offset * offset) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let total: f32 = weights.iter().sum();
    weights.into_iter().map(|weight| weight / total).collect()
}

/// The fitted constants of a curvature type: what it adds everywhere, its gain on the height and
/// on the normal (compose's `type_constants`)
fn type_constants(curvature_type: MicroCurvatureType) -> (f32, f32, f32) {
    match curvature_type {
        MicroCurvatureType::Standard => (0.00876, 12.36, 9.49),
        MicroCurvatureType::Sobel => (0.0, 5.72, 4.46),
        MicroCurvatureType::Smooth => (-0.00332, 0.969, 34.2),
    }
}

/// The occlusion's directions, each direction's steps (texel offsets and distance), the steps
/// per direction (0: the radius occludes nothing) and its depth, over a block `width` texels
/// wide (compose's `occlusion_factor`)
#[allow(clippy::type_complexity)]
fn occlusion_taps(micro: &MicroDetails, width: u32) -> ([[f32; 4]; OCCLUSION_DIRECTIONS], Vec<[f32; 4]>, usize, f32) {
    let tile_scale = width as f32 / (REFERENCE_TILE_TEXELS / 2.0);
    let radius_1k = OCCLUSION_RADIUS_TEXELS_1K * micro.ao_radius.powf(OCCLUSION_RADIUS_POWER);
    let radius = (radius_1k * tile_scale).round() as usize;
    let mut directions = [[0.0; 4]; OCCLUSION_DIRECTIONS];
    if radius_1k < MIN_OCCLUSION_RADIUS_TEXELS_1K || radius < 1 {
        return (directions, Vec::new(), 0, 0.0);
    }

    let depth = OCCLUSION_DEPTH_SCALE_1K * micro.ao_depth.powf(OCCLUSION_DEPTH_POWER) * tile_scale;
    let steps: Vec<f32> = if radius <= OCCLUSION_STEPS {
        (1..=radius).map(|step| step as f32).collect()
    } else {
        (1..=OCCLUSION_STEPS).map(|step| 1.0 + (radius - 1) as f32 * (step - 1) as f32 / (OCCLUSION_STEPS - 1) as f32).collect()
    };
    let mut taps = Vec::with_capacity(OCCLUSION_DIRECTIONS * steps.len());
    for (index, direction) in directions.iter_mut().enumerate() {
        let angle = std::f32::consts::TAU * index as f32 / OCCLUSION_DIRECTIONS as f32;
        let (dx, dy) = (angle.cos(), angle.sin());
        *direction = [dx, dy, 0.0, 0.0];
        taps.extend(steps.iter().map(|&distance| [(dx * distance).round(), (dy * distance).round(), distance, 0.0]));
    }
    (directions, taps, steps.len(), depth)
}

/// A slot's words for the parameters: its image's size (0: uniform) and its uniform value
fn slot_size(image: Option<&Arc<ImageRes>>) -> [u32; 2] {
    image.map_or([0; 2], |image| [image.width, image.height])
}

/// The cache key of a program's micro maps: the requests, their images, the bake's maps and the
/// layout they are built in
struct MicroKey {
    requests: Vec<MicroRequest>,
    images: Vec<[Option<Weak<ImageRes>>; 2]>,
    maps: [Option<Weak<Vec<f32>>>; 2],
    size: [u32; 3],
    grid: [u32; 2],
    levels: Option<Option<usize>>,
}

impl MicroKey {
    fn matches(&self, other: &MicroKey) -> bool {
        fn same<T>(a: &Option<Weak<T>>, b: &Option<Weak<T>>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => Weak::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        }

        self.requests == other.requests
            && self.size == other.size
            && self.grid == other.grid
            && self.levels == other.levels
            && self.maps.iter().zip(&other.maps).all(|(a, b)| same(a, b))
            && self.images.len() == other.images.len()
            && self.images.iter().zip(&other.images).all(|(a, b)| a.iter().zip(b).all(|(a, b)| same(a, b)))
    }
}

struct MicroCache {
    key: MicroKey,
    maps: Arc<MicroMaps>,
}

/// The micro detail passes, and each texture set's last maps
pub(super) struct MicroGpu {
    layout: wgpu::BindGroupLayout,
    sample: wgpu::ComputePipeline,
    tilt: wgpu::ComputePipeline,
    blur: wgpu::ComputePipeline,
    accumulate: wgpu::ComputePipeline,
    term: wgpu::ComputePipeline,
    occlusion: wgpu::ComputePipeline,
    adjust: wgpu::ComputePipeline,
    cache: BTreeMap<usize, MicroCache>,
}

/// A `size` texture of `format` with `layers` layers
fn texture(device: &wgpu::Device, label: &str, size: [u32; 3], format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: size[2] },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// The empty micro maps a program without micro details binds
pub(super) fn empty_maps(device: &wgpu::Device) -> wgpu::TextureView {
    texture(device, "tc_micro_maps_empty", [1, 1, 1], wgpu::TextureFormat::R32Float, wgpu::TextureUsages::TEXTURE_BINDING)
        .create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() })
}

impl MicroGpu {
    fn new(device: &wgpu::Device) -> Self {
        let float_array = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tc_micro_inputs"),
            entries: &[
                entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                storage(1, true),
                storage(2, false),
                storage(3, false),
                storage(4, false),
                storage(5, false),
                storage(6, false),
                storage(7, false),
                entry(
                    8,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                    },
                ),
                entry(
                    9,
                    wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                ),
                entry(10, float_array),
                entry(
                    11,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba32Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tc_micro_pipeline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::include_wgsl!("shaders/generator_p2_micro.wgsl"));
        let pipeline = |name| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(name),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        Self {
            sample: pipeline("sample_slots"),
            tilt: pipeline("tilt"),
            blur: pipeline("blur"),
            accumulate: pipeline("accumulate"),
            term: pipeline("term"),
            occlusion: pipeline("occlusion"),
            adjust: pipeline("adjust_bake"),
            layout,
            cache: BTreeMap::new(),
        }
    }

    /// The micro maps `program` reads on texture set `ts`, built on first use and kept while its
    /// requests, images and bake stay the same (None without micro details). `baked` is the
    /// program's bake texture, `supported` whether the composite binds micro maps.
    pub(super) fn maps(
        slot: &mut Option<MicroGpu>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        (ts, supported): (usize, bool),
        program: &CompositeProgram,
        baked: &wgpu::TextureView,
    ) -> Result<Option<Arc<MicroMaps>>, String> {
        let plan = &program.micro;
        if plan.is_empty() {
            if let Some(gpu) = slot {
                gpu.cache.remove(&ts);
            }
            return Ok(None);
        }
        if !supported {
            return Err("Micro Details exceed the GPU adapter's binding limits.".into());
        }

        let bake = program.baked.as_ref();
        let width = bake.map_or(1, |baked| baked.width);
        let map_rows = |index: usize| bake.and_then(|baked| baked.maps[index].as_ref().map(|_| baked.heights[index])).unwrap_or(0);
        let (occlusion_rows, curvature_rows) = (map_rows(0), map_rows(2));
        let grid = program.udim_layout.as_ref().map_or([1, 1], |layout| [layout.grid.cols, layout.grid.rows]);
        let images = plan.images.iter().map(|pair| pair.each_ref().map(|image| image.as_ref().map(Arc::downgrade))).collect();
        let maps = [0, 2].map(|index| bake.and_then(|baked| baked.maps[index].as_ref().map(Arc::downgrade)));
        let key = MicroKey { requests: plan.requests.clone(), images, maps, size: [width, occlusion_rows, curvature_rows], grid, levels: plan.levels };
        let gpu = slot.get_or_insert_with(|| MicroGpu::new(device));
        if let Some(cache) = gpu.cache.get(&ts).filter(|cache| cache.key.matches(&key)) {
            return Ok(Some(cache.maps.clone()));
        }

        gpu.cache.remove(&ts);
        let maps = Arc::new(gpu.build(device, queue, program, baked, (width, occlusion_rows, curvature_rows), grid)?);
        gpu.cache.insert(ts, MicroCache { key, maps: maps.clone() });
        Ok(Some(maps))
    }

    fn build(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        program: &CompositeProgram,
        baked: &wgpu::TextureView,
        (width, occlusion_rows, curvature_rows): (u32, u32, u32),
        grid: [u32; 2],
    ) -> Result<MicroMaps, String> {
        let plan = &program.micro;
        let limits = device.limits();
        // The row counts sit after the tallest map's rows, after one row where no map is baked
        let tallest = occlusion_rows.max(curvature_rows).max(1);
        let layers = (plan.requests.len() * 2) as u32;
        if width.max(tallest + 1) > limits.max_texture_dimension_2d || layers > limits.max_texture_array_layers {
            return Err("Micro Details exceed the GPU texture dimensions.".into());
        }

        let usage = wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
        let maps = texture(device, "tc_micro_maps", [width, tallest + 1, layers], wgpu::TextureFormat::R32Float, usage);
        let maps_view = maps.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let dummy = Dummies::new(device);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("tc_micro") });

        for (index, request) in plan.requests.iter().enumerate() {
            // Images not resolved yet (`set_tile_images`) read as not loaded
            let images = plan.images.get(index).cloned().unwrap_or_default();
            let slots = SlotData::new(device, request, &images, limits.max_storage_buffer_binding_size.min(limits.max_buffer_size))?;
            let layers = [(request.curvature, curvature_rows, 0), (request.occlusion, occlusion_rows, 1)];
            for (built, rows, offset) in layers {
                let layer = index as u32 * 2 + offset;

                // Each layer's own row count, after the tallest map's rows
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &maps,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x: 0, y: tallest, z: layer },
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytemuck::bytes_of(&(rows as f32)),
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: None, rows_per_image: None },
                    wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                );
                if !built {
                    continue;
                }

                // A map not baked reads neutral, its one row: no curvature term, no occlusion
                if rows == 0 {
                    if offset == 1 {
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &maps,
                                mip_level: 0,
                                origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                                aspect: wgpu::TextureAspect::All,
                            },
                            bytemuck::cast_slice(&vec![1.0f32; width as usize]),
                            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: None, rows_per_image: None },
                            wgpu::Extent3d { width, height: 1, depth_or_array_layers: 1 },
                        );
                    }
                    continue;
                }
                for group in block_groups([width, rows], grid) {
                    let run = Run { gpu: self, device, slots: &slots, dummy: &dummy, target: &maps_view, group: &group, layer };
                    run.fits(limits.max_storage_buffer_binding_size.min(limits.max_buffer_size))?;
                    if offset == 0 {
                        run.curvature(&mut encoder, request);
                    } else {
                        run.occlusion(&mut encoder, request);
                    }
                }
            }
        }

        // The detail sizes blur the bake with the claimed micro curvature added
        let adjusted = match plan.levels {
            Some(Some(index)) if curvature_rows > 0 => {
                let bake = texture(
                    device,
                    "tc_micro_bake",
                    [width, curvature_rows, 1],
                    wgpu::TextureFormat::Rgba32Uint,
                    wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                );
                let view = bake.create_view(&Default::default());
                let params = Params { place: [0, 0, index as u32 * 2, 0], ..Params::zeroed() };
                let bindings = self.bindings(device, &params, &dummy, (&dummy.maps_out, baked, &maps_view, &view));
                dispatch(&mut encoder, &self.adjust, &bindings, [width.div_ceil(WORKGROUP), curvature_rows.div_ceil(WORKGROUP)]);
                Some(view)
            }
            _ => None,
        };
        queue.submit([encoder.finish()]);
        Ok(MicroMaps { maps: maps_view, bake: adjusted })
    }

    /// A pass's bind group: `params`, the run's fields and the images (`maps_out`, `bake`,
    /// `maps_in`, `bake_out`)
    fn bindings(
        &self,
        device: &wgpu::Device,
        params: &Params,
        fields: &dyn FieldBuffers,
        (maps_out, bake, maps_in, bake_out): (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView),
    ) -> wgpu::BindGroup {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tc_micro_params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let buffers = fields.buffers();
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }];
        entries
            .extend(buffers.iter().enumerate().map(|(index, buffer)| wgpu::BindGroupEntry { binding: index as u32 + 1, resource: buffer.as_entire_binding() }));
        entries.extend([
            wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(maps_out) },
            wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(bake) },
            wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(maps_in) },
            wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(bake_out) },
        ]);
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("tc_micro_pass"), layout: &self.layout, entries: &entries })
    }
}

fn dispatch(encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline, bindings: &wgpu::BindGroup, groups: [u32; 2]) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tc_micro_pass"), timestamp_writes: None });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bindings, &[]);
    pass.dispatch_workgroups(groups[0].max(1), groups[1].max(1), 1);
}

/// The seven storage buffers of a pass (bindings 1 to 7): the slots, then the fields
trait FieldBuffers {
    fn buffers(&self) -> [&wgpu::Buffer; 7];
}

/// Placeholder buffers and images for the bindings a pass does not read
struct Dummies {
    buffers: [wgpu::Buffer; 7],
    maps_out: wgpu::TextureView,
    maps_in: wgpu::TextureView,
    bake: wgpu::TextureView,
    bake_out: wgpu::TextureView,
}

impl Dummies {
    fn new(device: &wgpu::Device) -> Self {
        let buffer = || {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tc_micro_unused"),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let array = wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() };
        let storage = wgpu::TextureUsages::STORAGE_BINDING;
        let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
        Self {
            buffers: std::array::from_fn(|_| buffer()),
            maps_out: texture(device, "tc_micro_unused_out", [1, 1, 1], wgpu::TextureFormat::R32Float, storage).create_view(&array),
            maps_in: texture(device, "tc_micro_unused_in", [1, 1, 1], wgpu::TextureFormat::R32Float, sampled).create_view(&array),
            bake: texture(device, "tc_micro_unused_bake", [1, 1, 1], wgpu::TextureFormat::Rgba32Uint, sampled).create_view(&Default::default()),
            bake_out: texture(device, "tc_micro_unused_bake_out", [1, 1, 1], wgpu::TextureFormat::Rgba32Uint, storage).create_view(&Default::default()),
        }
    }
}

impl FieldBuffers for Dummies {
    fn buffers(&self) -> [&wgpu::Buffer; 7] {
        self.buffers.each_ref()
    }
}

/// A request's slots for the passes: the height image's gray (the mean of its channels, as
/// `Slot::heights` reads it) then the normal image's red and green, and the uniform values
struct SlotData {
    buffer: wgpu::Buffer,
    normal_start: u32,
    sizes: [u32; 4],
    uniforms: [f32; 4],
    reads: u32,
}

impl SlotData {
    fn new(device: &wgpu::Device, request: &MicroRequest, images: &[Option<Arc<ImageRes>>; 2], limit: u64) -> Result<Self, String> {
        let [height_image, normal_image] = images.each_ref().map(Option::as_ref);
        let mut words: Vec<f32> =
            height_image.map_or_else(Vec::new, |image| image.rgba.iter().map(|[red, green, blue, _]| (red + green + blue) / 3.0).collect());
        let normal_start = words.len() as u32;
        if let Some(image) = normal_image {
            words.extend(image.rgba.iter().flat_map(|[red, green, _, _]| [*red, *green]));
        }
        words.resize(words.len().max(4), 0.0);
        if (words.len() * 4) as u64 > limit {
            return Err("A Micro Details image exceeds the GPU storage-buffer limit.".into());
        }

        // An image slot whose image is not loaded reads black
        let uniform = |slot: &Option<MicroSlot>| match slot {
            Some(MicroSlot::Uniform(value)) => *value,
            _ => [0.0; 3],
        };
        let height = uniform(&request.height);
        let normal = uniform(&request.normal);
        let flip_green = matches!(request.normal, Some(MicroSlot::Image { flip_green: true, .. })) && normal_image.is_some();
        let [height_size, normal_size] = [slot_size(height_image), slot_size(normal_image)];
        let reads = if request.height.is_some() { READS_HEIGHT } else { 0 } | if request.normal.is_some() { READS_NORMAL } else { 0 };
        Ok(Self {
            buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tc_micro_slots"),
                contents: bytemuck::cast_slice(&words),
                usage: wgpu::BufferUsages::STORAGE,
            }),
            normal_start,
            sizes: [height_size[0], height_size[1], normal_size[0], normal_size[1]],
            uniforms: [height.iter().sum::<f32>() / 3.0, normal[0] * 2.0 - 1.0, normal[1] * 2.0 - 1.0, if flip_green { -1.0 } else { 1.0 }],
            reads,
        })
    }
}

/// The fields of one block size's passes, with the request's slots
struct Fields<'a> {
    slots: &'a wgpu::Buffer,
    fields: [wgpu::Buffer; 6],
}

impl FieldBuffers for Fields<'_> {
    fn buffers(&self) -> [&wgpu::Buffer; 7] {
        let [heights, first, second, detail, normals, tilted] = &self.fields;
        [self.slots, heights, first, second, detail, normals, tilted]
    }
}

/// The passes of one request's map over one block size
struct Run<'a> {
    gpu: &'a MicroGpu,
    device: &'a wgpu::Device,
    slots: &'a SlotData,
    dummy: &'a Dummies,
    target: &'a wgpu::TextureView,
    group: &'a BlockGroup,
    layer: u32,
}

impl Run<'_> {
    /// Whether the block's fields fit the storage-buffer limit
    fn fits(&self, limit: u64) -> Result<(), String> {
        let texels = u64::from(self.group.size[0]) * u64::from(self.group.size[1]);
        if texels * 8 > limit {
            return Err("A Micro Details block exceeds the GPU storage-buffer limit.".into());
        }
        Ok(())
    }

    fn buffers(&self) -> Fields<'_> {
        let texels = u64::from(self.group.size[0]) * u64::from(self.group.size[1]);
        let buffer = |bytes: u64| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tc_micro_field"),
                size: bytes.max(16),
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        Fields {
            slots: &self.slots.buffer,
            fields: [buffer(texels * 4), buffer(texels * 4), buffer(texels * 4), buffer(texels * 4), buffer(texels * 8), buffer(texels * 8)],
        }
    }

    /// The parameters every pass of this block size shares
    fn params(&self) -> Params {
        let group = self.group;
        Params {
            block: [group.size[0], group.size[1], self.slots.normal_start, 0],
            slots: self.slots.sizes,
            uniforms: self.slots.uniforms,
            origins: [group.origin[0], group.origin[1], group.step[0], group.step[1]],
            place: [group.count[0], group.count[1], self.layer, 0],
            ..Params::zeroed()
        }
    }

    fn pass(&self, encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline, params: &Params, fields: &Fields, lines: Option<u32>) {
        let bindings = self.gpu.bindings(self.device, params, fields, (self.target, &self.dummy.bake, &self.dummy.maps_in, &self.dummy.bake_out));
        let groups = match lines {
            Some(count) => [count.div_ceil(LINE_WORKGROUP), 1],
            None => [self.group.size[0].div_ceil(WORKGROUP), self.group.size[1].div_ceil(WORKGROUP)],
        };
        dispatch(encoder, pipeline, &bindings, groups);
    }

    /// The curvature the micro details add over this block size (compose's `curvature_term`)
    fn curvature(&self, encoder: &mut wgpu::CommandEncoder, request: &MicroRequest) {
        let micro = &request.micro;
        let fields = self.buffers();
        let (offset, height_gain, normal_gain) = type_constants(micro.curvature_type);
        let texel_scale = self.group.size[0] as f32 / REFERENCE_TILE_TEXELS;
        let smooth = micro.curvature_type == MicroCurvatureType::Smooth;
        let mut params = self.params();
        params.block[3] = u32::from(smooth);
        params.gains = [
            offset,
            micro.curvature_intensity * height_gain,
            micro.curvature_intensity * normal_gain,
            HEIGHT_NORMAL_STRENGTH * micro.height_intensity * texel_scale,
        ];
        params.scales = [texel_scale, 0.0, 0.0, self.slots.reads as f32];
        params.intensity = [micro.height_intensity, 0.0, 0.0, 0.0];
        self.pass(encoder, &self.gpu.sample, &params, &fields, None);

        if self.slots.reads & READS_HEIGHT != 0 {
            if smooth {
                for sigma in SMOOTH_SIGMAS {
                    self.blur(encoder, &params, &fields, sigma * texel_scale);
                    self.pass(encoder, &self.gpu.accumulate, &params, &fields, None);
                }
            } else {
                self.pass(encoder, &self.gpu.tilt, &params, &fields, None);
            }
        }
        self.pass(encoder, &self.gpu.term, &params, &fields, None);
    }

    /// The heights blurred to `sigma` into the second field, wrapping around the block (compose's
    /// `gaussian_wrapped`): rows then columns, each a sampled kernel or three box passes
    fn blur(&self, encoder: &mut wgpu::CommandEncoder, shared: &Params, fields: &Fields, sigma: f32) {
        let mut params = *shared;
        let [width, height] = self.group.size;
        let route = |source: u32, destination: u32| source | destination << 2;
        let steps: Vec<(u32, u32)> = if sigma < BOX_BLUR_MIN_SIGMA {
            let kernel = gaussian_kernel(sigma);
            for (index, weight) in kernel.iter().enumerate().take(KERNEL_TAPS) {
                params.kernel[index / 4][index % 4] = *weight;
            }
            params.lines[1] = 0;
            params.lines[2] = (kernel.len() / 2) as u32;
            vec![(0, route(HEIGHTS, FIRST)), (1, route(FIRST, SECOND))]
        } else {
            // Three box passes of radius r spread a value with variance r(r + 1)
            params.lines[1] = (((1.0 + 4.0 * sigma * sigma).sqrt() - 1.0) / 2.0).round().max(1.0) as u32;
            let rows = [route(HEIGHTS, FIRST), route(FIRST, SECOND), route(SECOND, FIRST)];
            let columns = [route(FIRST, SECOND), route(SECOND, FIRST), route(FIRST, SECOND)];
            rows.into_iter().map(|step| (0, step)).chain(columns.into_iter().map(|step| (1, step))).collect()
        };
        for (axis, step) in steps {
            params.lines[0] = axis;
            params.lines[3] = step;
            let lines = if axis == 0 { height } else { width };
            self.pass(encoder, &self.gpu.blur, &params, fields, Some(lines));
        }
    }

    /// What the micro height multiplies the occlusion by over this block size (compose's
    /// `occlusion_factor`)
    fn occlusion(&self, encoder: &mut wgpu::CommandEncoder, request: &MicroRequest) {
        let fields = self.buffers();
        let (directions, taps, steps, depth) = occlusion_taps(&request.micro, self.group.size[0]);
        let mut params = self.params();
        params.directions = directions;
        for (slot, tap) in params.taps.iter_mut().zip(taps) {
            *slot = tap;
        }
        params.scales = [0.0, depth, steps as f32, READS_HEIGHT as f32];
        self.pass(encoder, &self.gpu.sample, &params, &fields, None);
        self.pass(encoder, &self.gpu.occlusion, &params, &fields, None);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageRes, ImageStore, PixelStore};
    use texturecraft_core::{
        BakedMap, Channel, Effect, EffectKind, Fill, Generator, GeneratorKind, Layer, LayerKind, Mask, MaskBackground, MicroCurvatureType, MicroDetails,
        NormalFormat, Project, Source, TextureLayer, TextureSet,
    };
    use texturecraft_geom::primitives;
    use texturecraft_texel::TILE;

    use super::block_groups;
    use crate::gpu_composite::oracle::{OracleCase, Tolerance, assert_matches_cpu, earlier_generator, headless};

    /// Blocks of one size come out together, the clipped last column, row and corner apart
    #[test]
    fn blocks_group_by_their_clipped_size() {
        let groups = block_groups([10, 7], [3, 2]);

        let sizes: Vec<_> = groups.iter().map(|group| (group.origin, group.count, group.size)).collect();
        assert_eq!(sizes, vec![([0, 0], [3, 2], [3, 3]), ([9, 0], [1, 2], [1, 3]), ([0, 6], [3, 1], [3, 1]), ([9, 6], [1, 1], [1, 1])]);
        assert_eq!(block_groups([8, 8], [1, 1]).len(), 1, "a classic set is one block");
    }

    /// An image of `size` texels of hashed noise, each channel its own
    fn noise(size: (u32, u32), seed: u32) -> Arc<ImageRes> {
        let hash = |texel: u32, channel: u32| {
            let mixed = (texel.wrapping_mul(0x9e37_79b9) ^ (channel + seed).wrapping_mul(0x85eb_ca6b)).wrapping_mul(0xc2b2_ae35);
            (mixed >> 8) as f32 / (1 << 24) as f32
        };
        let rgba = (0..size.0 * size.1).map(|texel| [hash(texel, 0), hash(texel, 1), hash(texel, 2), 1.0]).collect();
        Arc::new(ImageRes::new(size.0, size.1, rgba, false))
    }

    /// Micro Details on the earlier model match the CPU: the Sobel, Standard and Smooth curvature
    /// of a height and a normal image (Sobel's normal DirectX, with the detail sizes blurring the
    /// micro curvature; Smooth's kernel and box blurs), a uniform height and an empty normal slot,
    /// the occlusion of a height image over at most and over more than 16 steps, a radius that
    /// occludes nothing and no baked occlusion; a texture and a custom grunge leave the earlier
    /// model as it is
    #[test]
    fn micro_details_match_the_cpu() {
        let Some(mut gpu) = headless() else { return };

        // A 512 bake: Smooth's sigmas reach the box blur, AO Radius 1 more than 16 steps
        const BAKE: u32 = 512;
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let map = |phase: f32| -> Arc<Vec<f32>> {
            Arc::new((0..BAKE * BAKE).map(|texel| 0.5 + 0.2 * ((texel % BAKE) as f32 * 0.07 + phase).sin() * ((texel / BAKE) as f32 * 0.05).cos()).collect())
        };
        let baked = BakedMaps { resolution: BAKE, maps: BTreeMap::from([(BakedMap::Curvature, map(0.0)), (BakedMap::AmbientOcclusion, map(1.3))]) };
        let images = ImageStore::from([("height".to_string(), noise((101, 97), 1)), ("normal".to_string(), noise((89, 83), 2))]);
        let height = Some(Source::Image { name: "height".into(), normal_format: NormalFormat::OpenGl });
        let normal = Some(Source::Image { name: "normal".into(), normal_format: NormalFormat::DirectX });
        let micro = |kind| MicroDetails { enabled: true, ..MicroDetails::defaults(kind).unwrap() };
        let curvature = |micro: MicroDetails| Generator { detail: None, micro: Some(Box::new(micro)), ..earlier_generator(GeneratorKind::Curvature) };
        let images_of = |curvature_type| MicroDetails {
            curvature_type,
            height_source: height.clone(),
            normal_source: Some(Source::image("normal")),
            ..micro(GeneratorKind::Curvature)
        };
        let unbaked = BakedMaps::default();
        let cases = [
            (
                "Sobel with detail sizes",
                Generator {
                    micro: Some(Box::new(MicroDetails { normal_source: normal, curvature_intensity: 0.6, ..images_of(MicroCurvatureType::Sobel) })),
                    ..earlier_generator(GeneratorKind::Curvature)
                },
                &baked,
            ),
            ("Standard", curvature(images_of(MicroCurvatureType::Standard)), &baked),
            ("Smooth", curvature(MicroDetails { height_intensity: 4.0, ..images_of(MicroCurvatureType::Smooth) }), &baked),
            (
                "a uniform height and an empty normal",
                curvature(MicroDetails {
                    curvature_type: MicroCurvatureType::Standard,
                    height_source: Some(Source::Uniform([0.3, 0.5, 0.7])),
                    ..micro(GeneratorKind::Curvature)
                }),
                &baked,
            ),
            (
                "occlusion over more than 16 steps",
                Generator {
                    micro: Some(Box::new(MicroDetails {
                        height_source: height.clone(),
                        ao_radius: 1.0,
                        ao_depth: 0.3,
                        ..micro(GeneratorKind::AmbientOcclusion)
                    })),
                    ..earlier_generator(GeneratorKind::AmbientOcclusion)
                },
                &baked,
            ),
            (
                "occlusion over 16 steps at most",
                Generator {
                    micro: Some(Box::new(MicroDetails {
                        height: true,
                        height_source: height.clone(),
                        ao_radius: 0.5,
                        ao_depth: 0.5,
                        ..micro(GeneratorKind::Dirt)
                    })),
                    ..earlier_generator(GeneratorKind::Dirt)
                },
                &baked,
            ),
            (
                "an occlusion radius too short to occlude",
                Generator {
                    micro: Some(Box::new(MicroDetails { height_source: height.clone(), ao_radius: 0.1, ..micro(GeneratorKind::AmbientOcclusion) })),
                    ..earlier_generator(GeneratorKind::AmbientOcclusion)
                },
                &baked,
            ),
            (
                "occlusion without a baked map",
                Generator {
                    micro: Some(Box::new(MicroDetails { height_source: height.clone(), ao_radius: 1.0, ..micro(GeneratorKind::AmbientOcclusion) })),
                    ..earlier_generator(GeneratorKind::AmbientOcclusion)
                },
                &unbaked,
            ),
            (
                "a texture and a custom grunge, which the earlier model does not read",
                Generator {
                    detail: None,
                    texture: Some(Box::new(TextureLayer { enabled: true, opacity: 0.8, source: height.clone(), ..TextureLayer::default() })),
                    grunge: height.clone(),
                    ..earlier_generator(GeneratorKind::Curvature)
                },
                &baked,
            ),
        ];
        for (label, generator, baked) in cases {
            let mut project = Project::new("Micro Details", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, TILE));
            let owner = project.alloc_id();
            let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.9, 0.6, 0.3]))]), ..Default::default() };
            let mut layer = Layer::new(owner, "Micro", LayerKind::Fill(fill), &[Channel::BaseColor]);
            let mut mask = Mask::new(MaskBackground::Black);
            mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Generator(generator)));
            layer.mask = Some(mask);
            project.insert_layer(0, layer, None, false).unwrap();

            let pixels = PixelStore::default();
            let mut case = OracleCase::new(&project, &pixels, baked, &images, &mesh);
            case.mask_owner = Some(owner);
            assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, label);
        }
    }
}
