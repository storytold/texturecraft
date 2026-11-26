//! P2 of the generator GPU port: Micro Details. A generator's micro height and normal add
//! curvature and occlusion to its mesh maps; compose's `micro_detail/mod.rs` builds those maps
//! once per map, image and settings, in the bake's layout. Here the GPU builds them
//! (`shaders/generator_p2_micro.wgsl`) before the composite reads them
//! (`shaders/generator_p2.wgsl`), and the curvature detail sizes blur the bake with the micro
//! curvature added, as `GeneratorMaps::of` blurs `MicroCurvature::curvature`.
//!
//! A generator's P2 record (`Operation::uv[3]`, the side-table offset; 0: none) holds its micro
//! curvature layer plus one, its micro occlusion layer plus one, its texture and grunge records
//! (0: none), and the micro curvature set its detail sizes read (its request plus one; 0: the
//! bake's own, the curvature levels binding). The texture and the grunge are read on the new model's section and wear
//! controls alone (P1's paths call `generator_texture` and `generator_grunge` in WGSL), so on the
//! earlier model their records stay 0.

use std::sync::Arc;

use texturecraft_compose::ImageRes;
use texturecraft_core::{
    ChannelKind, Generator, GeneratorGrayscale, GeneratorKind, MicroCurvatureType, MicroDetails, Procedural, ProceduralKind, Projection, Sampling, Source,
    TextureBlend, WearParameters,
};

use super::{ImageUse, ProceduralInputs, source_operation};

mod block_group;
mod dummies;
mod field_buffers;
mod fields;
mod grunge_pattern;
mod micro_cache;
mod micro_gpu;
mod micro_key;
mod micro_maps;
mod micro_plan;
mod micro_request;
mod micro_slot;
mod params;
mod run;
mod slot_data;
mod slot_placement;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

use block_group::BlockGroup;
use grunge_pattern::{DIRT_GRUNGE, EDGE_WEAR_GRUNGE, FIBER_GLASS_GRUNGE, TRIPLANAR_SHARPNESS};
pub(super) use micro_gpu::MicroGpu;
pub(super) use micro_plan::MicroPlan;
use micro_request::MicroRequest;
use micro_slot::MicroSlot;
use slot_placement::SlotPlacement;

/// Words of a generator's P2 record
const RECORD_WORDS: usize = 5;

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

/// The micro maps an earlier-model generator reads: its curvature input (Curvature, Edge Wear)
/// and its occlusion input (Ambient Occlusion, Dirt). P1's wear controls read both.
fn reads_micro(kind: GeneratorKind) -> (bool, bool) {
    let curvature = MicroDetails::has_curvature(kind) && matches!(kind, GeneratorKind::Curvature | GeneratorKind::EdgeWear);
    let occlusion = MicroDetails::has_occlusion(kind) && matches!(kind, GeneratorKind::AmbientOcclusion | GeneratorKind::Dirt);
    (curvature, occlusion)
}

/// The generator's P2 record for `Operation::uv[3]` (0: none), `reads_levels` when it reads
/// curvature detail sizes; None when the GPU leaves something it reads to the CPU (a micro height
/// read from an anchor point, a texture or grunge slot holding a procedural or an anchor)
pub(super) fn generator_record(generator: &Generator, reads_levels: bool, inputs: &mut ProceduralInputs) -> Option<f32> {
    let (micro_words, micro) = micro_record(generator, inputs)?;
    let texture = texture_record(generator, inputs)?;
    let grunge = grunge_record(generator, inputs)?;

    // Detail sizes read with a micro curvature blur the bake with it added: a set of their own
    let levels = match micro.filter(|_| reads_levels) {
        Some(index) => {
            inputs.micro.level_sets.insert(index);
            index as u32 + 1
        }
        None => 0,
    };
    let words = [micro_words[0], micro_words[1], texture, grunge, levels];
    if words == [0; RECORD_WORDS] {
        return Some(0.0);
    }

    let offset = inputs.pack_words(&words)?;

    // The record's offset travels as an exact float
    (offset < 1 << 24).then_some(offset as f32)
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

/// The fields `blur` reads and writes (`read_line` in WGSL)
const HEIGHTS: u32 = 0;
const FIRST: u32 = 1;
const SECOND: u32 = 2;

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

fn dispatch(encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline, bindings: &wgpu::BindGroup, groups: [u32; 2]) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("tc_micro_pass"), timestamp_writes: None });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, bindings, &[]);
    pass.dispatch_workgroups(groups[0].max(1), groups[1].max(1), 1);
}
