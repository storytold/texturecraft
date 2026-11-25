//! The CPU oracle for GPU compositor tests, shared by the GPU port packages (Package 0).
//!
//! [`assert_matches_cpu`] composites a project on the CPU (`compose::eval`, the reference the
//! export also uses) and on the GPU, and compares every display map the GPU program writes, the
//! mask preview of one layer, and optionally a partial update against the full one. Tolerances
//! follow plan section 6; never loosen them, investigate instead. [`compile_effects`] is the
//! compile-only probe of the packages' `still_on_cpu` tables.
// Test support: failing loudly is the point, and the packages use the helpers as they port.
#![allow(clippy::unwrap_used, clippy::panic, dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use texturecraft_compose::eval::{Composite, Inputs, composite_tiles, mask_image_sized};
use texturecraft_compose::{BakedMaps, DisplayKind, ImageStore, PixelStore, tile_bytes};
use texturecraft_core::{Channel, ContentsOffset, Effect, EffectKind, Fill, Layer, LayerId, LayerKind, Mask, MaskBackground, Project, Source, TextureSet};
use texturecraft_geom::{Mesh, UvGBuffer};
use texturecraft_texel::TILE;

use super::{CompositeProgram, CompositeRegion};
use crate::headless::Headless;
use crate::readback::Readback;

/// A `kind` generator on the earlier model (no wear, section, light or kind controls), the one
/// the GPU composes; [`texturecraft_core::Generator::new`] gives the reference app's controls,
/// which compose on the CPU
pub(crate) fn earlier_generator(kind: texturecraft_core::GeneratorKind) -> texturecraft_core::Generator {
    texturecraft_core::Generator { wear: None, section: None, light: None, controls: None, ..texturecraft_core::Generator::new(kind) }
}

/// The GPU for an oracle test, or None without an adapter: the test then returns, as the other
/// GPU tests do. With `TC_CI_LAVAPIPE` set (CI promises an adapter) a missing one fails the test.
pub(crate) fn headless() -> Option<Headless> {
    match Headless::new() {
        Ok(gpu) => Some(gpu),
        Err(error) => {
            assert!(std::env::var_os("TC_CI_LAVAPIPE").is_none(), "TC_CI_LAVAPIPE is set but the GPU is unavailable: {error}");
            None
        }
    }
}

/// Whether the full per-path parity matrix runs. The CI gate runs each family's packed test
/// (one test that covers every feature of the family once, with few shader programs); the
/// per-path tests return at once unless `TC_GPU_PARITY_FULL=1`, as in
/// `TC_GPU_PARITY_FULL=1 cargo test -p texturecraft-render --lib`.
pub(crate) fn full_parity_matrix() -> bool {
    full_matrix_asked() || PACKED_RUN.get()
}

/// `TC_GPU_PARITY_FULL=1`
fn full_matrix_asked() -> bool {
    std::env::var("TC_GPU_PARITY_FULL").is_ok_and(|value| value == "1")
}

thread_local! {
    /// Set while a family's packed test runs its per-path tests with one case each ([`packed_run`])
    static PACKED_RUN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run a family's per-path tests from its packed test, each with only its first case
/// ([`gate_cases`]), unless the full matrix runs anyway
pub(crate) fn packed_run(tests: impl FnOnce()) {
    PACKED_RUN.set(true);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(tests));
    PACKED_RUN.set(false);
    if let Err(failure) = result {
        std::panic::resume_unwind(failure);
    }
}

/// `cases` in the full matrix; their first one alone inside a family's packed test
pub(crate) fn gate_cases<T>(cases: impl IntoIterator<Item = T>) -> Vec<T> {
    let reduced = PACKED_RUN.get() && !full_matrix_asked();
    cases.into_iter().take(if reduced { 1 } else { usize::MAX }).collect()
}

/// Run `packed`, a family's check of all its cases at once; when it fails, run `one_by_one`,
/// which checks each case on its own and panics naming the first one that differs, and fail
/// with the packed check's panic when every case passes alone.
pub(crate) fn check_packed(gpu: &mut Headless, packed: impl FnOnce(&mut Headless), one_by_one: impl FnOnce(&mut Headless)) {
    let Err(failure) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| packed(gpu))) else { return };
    one_by_one(gpu);
    std::panic::resume_unwind(failure);
}

/// How far the GPU may differ from the CPU oracle (plan section 6).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tolerance {
    /// Largest difference of an 8-bit component, and of the 8-bit mask preview.
    pub(crate) lsb: u8,
    /// Largest difference of a half-float component (Height, Normal maps).
    pub(crate) f16: f32,
    /// Normal map texels compare by the dot product of their directions instead, when set.
    pub(crate) normal_dot: Option<f32>,
    /// Texels allowed beyond the limits: only for hard decisions (thresholds, steps, cell
    /// centres, nearest sampling, depth culling) on inputs chosen away from the boundary.
    pub(crate) flips: usize,
}

impl Tolerance {
    /// Point filters, sampling, projections and masks.
    pub(crate) const POINT: Self = Self { lsb: 1, f16: 0.001, normal_dot: None, flips: 0 };
    /// Bilinear image sampling.
    pub(crate) const BILINEAR: Self = Self { lsb: 2, f16: 0.003, normal_dot: None, flips: 0 };
    /// The box family and distance filters (the raw distances must match exactly, test them apart).
    pub(crate) const NEIGHBOURHOOD: Self = Self { lsb: 2, f16: 0.003, normal_dot: None, flips: 0 };
    /// Gather, warp, Kuwahara, FXAA, OKLab and SH lighting (transcendentals); normals by direction.
    pub(crate) const TRANSCENDENTAL: Self = Self { lsb: 2, f16: 0.003, normal_dot: Some(0.99), flips: 0 };

    /// A hard decision: allow at most `flips` texels (keep it at or below 0.1 % of the texels).
    pub(crate) const fn with_flips(self, flips: usize) -> Self {
        Self { flips, ..self }
    }
}

/// One project to composite on both sides.
pub(crate) struct OracleCase<'a> {
    pub(crate) project: &'a Project,
    pub(crate) ts: usize,
    pub(crate) pixels: &'a PixelStore,
    pub(crate) baked: &'a BakedMaps,
    pub(crate) images: &'a ImageStore,
    pub(crate) mesh: &'a Arc<Mesh>,
    /// Also compare this layer's mask preview with `mask_image_sized` at the mask view's extent.
    pub(crate) mask_owner: Option<LayerId>,
    /// Also recomposite only this region and require the full result (a filter's halo).
    pub(crate) region: Option<CompositeRegion>,
}

impl<'a> OracleCase<'a> {
    /// The material of texture set 0 with no bakes, images, mask comparison or region.
    pub(crate) fn new(project: &'a Project, pixels: &'a PixelStore, baked: &'a BakedMaps, images: &'a ImageStore, mesh: &'a Arc<Mesh>) -> Self {
        Self { project, ts: 0, pixels, baked, images, mesh, mask_owner: None, region: None }
    }
}

/// Composite `case` on the CPU and the GPU and panic, naming `label`, where they differ beyond
/// `tolerance`: every display map the GPU program writes, then the mask preview and the partial
/// update when the case asks for them. The texture set's sides must be multiples of the tile.
pub(crate) fn assert_matches_cpu(gpu: &mut Headless, case: &OracleCase, tolerance: Tolerance, label: &str) {
    let set = &case.project.texture_sets[case.ts];
    let (width, height) = set.size();
    assert!(width.is_multiple_of(TILE) && height.is_multiple_of(TILE), "{label}: oracle sets are whole tiles");
    let gbuf = Arc::new(UvGBuffer::new(case.mesh, set.material, width, height));
    let channels: Vec<Channel> = set.channels.iter().map(|&(channel, _)| channel).collect();
    let inputs = Inputs {
        project: case.project,
        ts: case.ts,
        mesh: case.mesh,
        gbuf: &gbuf,
        pixels: case.pixels,
        baked: case.baked,
        block_scales: set.block_scales(),
        preview: None,
        images: case.images,
        bounds: case.mesh.bounds(),
    };
    let tiles: Vec<(u32, u32)> = (0..height / TILE).flat_map(|ty| (0..width / TILE).map(move |tx| (tx, ty))).collect();
    // A UV tile atlas's displays read the bakes and tile scales per block, as the session's do
    let mut oracle = Composite { block_scales: set.block_scales(), ..Composite::with_size(&channels, width, height) };
    composite_tiles(&inputs, &mut oracle, &tiles);

    let sets: Vec<_> = case.project.texture_sets.iter().map(|set| (set.size(), set.material)).collect();
    let materials = case.project.texture_sets.iter().map(|set| set.material + 1).max().unwrap_or(1);
    gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &sets, materials);
    let mut program = CompositeProgram::try_compile_region(case.project, case.pixels, case.ts, None).unwrap_or_else(|reason| panic!("{label}: {reason}"));
    program.set_surface_inputs(case.mesh, &gbuf, Some(case.baked)).unwrap_or_else(|reason| panic!("{label}: {reason}"));
    program.set_tile_images(case.images);
    gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, case.ts, &program).unwrap_or_else(|error| panic!("{label}: {error}"));
    let kinds: Vec<DisplayKind> = DisplayKind::ALL.into_iter().filter(|&kind| program.writes_display(kind)).collect();
    let read = |gpu: &Headless, kind: DisplayKind| {
        let texture = gpu.renderer.display_textures()[case.ts][kind as usize];
        let (w, h, bytes) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap_or_else(|| panic!("{label}: {kind:?} readback"));
        // A neutral program displays one texel
        if (w, h) == (1, 1) { bytes.repeat((width * height) as usize) } else { bytes }
    };
    let full: Vec<Vec<u8>> = kinds.iter().map(|&kind| read(&*gpu, kind)).collect();
    for (&kind, actual) in kinds.iter().zip(&full) {
        let bytes = kind.bytes_per_texel();
        let mut expected = vec![0; actual.len()];
        for &(tx, ty) in &tiles {
            let tile = tile_bytes(&oracle, case.baked, kind, tx, ty);
            for (y, row) in tile.chunks_exact(TILE as usize * bytes).enumerate() {
                let start = ((ty * TILE + y as u32) * width + tx * TILE) as usize * bytes;
                expected[start..start + row.len()].copy_from_slice(row);
            }
        }
        compare(actual, &expected, kind, tolerance, &format!("{label} {kind:?}"));
    }

    if let Some(region) = case.region {
        program.region = Some(region);
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, case.ts, &program).unwrap_or_else(|error| panic!("{label}: {error}"));
        for (&kind, full) in kinds.iter().zip(&full) {
            let partial = read(&*gpu, kind);
            compare(&partial, full, kind, Tolerance { lsb: 1, flips: 0, ..tolerance }, &format!("{label} {kind:?} partial update"));
        }
    }

    if let Some(owner) = case.mask_owner {
        let extent = crate::mask_view_extent((width, height));
        let expected = mask_image_sized(&inputs, owner, extent).unwrap_or_else(|| panic!("{label}: no CPU mask"));
        let mut mask = CompositeProgram::try_compile_mask_preview(case.project, case.pixels, case.ts, owner, None, None)
            .unwrap_or_else(|reason| panic!("{label}: {reason}"));
        mask.set_surface_inputs(case.mesh, &gbuf, Some(case.baked)).unwrap_or_else(|reason| panic!("{label}: {reason}"));
        mask.set_tile_images(case.images);
        gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, case.ts, &mask).unwrap_or_else(|error| panic!("{label}: {error}"));
        let texture = gpu.renderer.aux_texture(case.ts).unwrap_or_else(|| panic!("{label}: no mask texture"));
        let (_, _, rgba) = Readback::blocking(&gpu.device, &gpu.queue, texture).unwrap_or_else(|| panic!("{label}: mask readback"));
        assert_eq!(rgba.len(), expected.len() * 4, "{label} mask size");
        let mut off = Vec::new();
        for (index, (&expected, &[r, g, b, a])) in expected.iter().zip(rgba.as_chunks::<4>().0).enumerate() {
            if r.abs_diff(expected) > tolerance.lsb || r != g || r != b || a != 255 {
                off.push(format!("texel {index}: GPU {r} {g} {b} {a}, CPU {expected}"));
            }
        }
        assert!(off.len() <= tolerance.flips, "{label} mask: {} texels differ, first {:?}", off.len(), &off[..off.len().min(8)]);
    }
}

fn compare(actual: &[u8], expected: &[u8], kind: DisplayKind, tolerance: Tolerance, label: &str) {
    assert_eq!(actual.len(), expected.len(), "{label}: size");
    let bytes = kind.bytes_per_texel();
    let mut off = Vec::new();
    for (index, (gpu, cpu)) in actual.chunks_exact(bytes).zip(expected.chunks_exact(bytes)).enumerate() {
        let matches = if bytes == 8 {
            let decode = |texel: &[u8]| -> [f32; 4] { std::array::from_fn(|i| half::f16::from_le_bytes([texel[2 * i], texel[2 * i + 1]]).to_f32()) };
            let (gpu, cpu) = (decode(gpu), decode(cpu));
            match tolerance.normal_dot.filter(|_| kind == DisplayKind::Normal) {
                Some(limit) => {
                    let length = |v: &[f32; 4]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
                    let dot = (gpu[0] * cpu[0] + gpu[1] * cpu[1] + gpu[2] * cpu[2]) / (length(&gpu) * length(&cpu));
                    dot > limit && (gpu[3] - cpu[3]).abs() <= tolerance.f16
                }
                None => gpu.iter().zip(&cpu).all(|(a, b)| (a - b).abs() <= tolerance.f16),
            }
        } else {
            gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(*b) <= tolerance.lsb)
        };
        if !matches {
            off.push(format!("texel {index}: GPU {gpu:?}, CPU {cpu:?}"));
        }
    }
    assert!(off.len() <= tolerance.flips, "{label}: {} texels differ, first {:?}", off.len(), &off[..off.len().min(8)]);
}

/// A content or mask effect for the compile probes.
pub(crate) fn effect(kind: EffectKind) -> Effect {
    Effect::new(LayerId(900), kind)
}

/// Compile a fill layer inside a folder with `folder`'s Scale and Rotation, carrying `content`
/// as a content effect and `mask` in a white mask: (material, mask preview), each Ok when the
/// GPU program evaluates it. The packages' `still_on_cpu` tables use it.
pub(crate) fn compile_effects(content: Option<Effect>, mask: Option<Effect>, folder: ContentsOffset) -> (Result<(), &'static str>, Result<(), &'static str>) {
    let channels = [Channel::BaseColor, Channel::Height];
    let mut project = Project::new("CPU effects", "builtin:plane");
    project.texture_sets.push(TextureSet::new("Plane", 0, 64));
    let folder_id = project.alloc_id();
    let mut group = Layer::new(folder_id, "Folder", LayerKind::folder(), &channels);
    if let LayerKind::Group { contents, .. } = &mut group.kind {
        *contents = folder;
    }
    project.insert_layer(0, group, None, false).unwrap();
    let owner = project.alloc_id();
    let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.6, 0.3, 0.2]))]), ..Default::default() };
    let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(fill), &channels);
    layer.content_effects.extend(content);
    let mut layer_mask = Mask::new(MaskBackground::White);
    layer_mask.effects.extend(mask);
    layer.mask = Some(layer_mask);
    project.insert_layer(0, layer, Some(folder_id), true).unwrap();
    let pixels = PixelStore::default();
    let material = CompositeProgram::try_compile_region(&project, &pixels, 0, None).map(|_| ());
    let mask_preview = CompositeProgram::try_compile_mask_preview(&project, &pixels, 0, owner, None, None).map(|_| ());
    (material, mask_preview)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use texturecraft_compose::{BakedMaps, ImageStore, PixelStore};
    use texturecraft_core::{
        Channel, Effect, EffectKind, Fill, Filter, Layer, LayerKind, Mask, MaskBackground, Project, Projection, Sampling, Source, TextureSet,
    };
    use texturecraft_geom::primitives;
    use texturecraft_texel::TILE;

    use super::{CompositeRegion, OracleCase, Tolerance, assert_matches_cpu, headless};

    /// The harness itself on a stack the GPU already evaluates: a fill with a blurred, inverted
    /// mask and content filters, in every map, the mask preview and a partial update.
    #[test]
    fn the_oracle_accepts_a_stack_the_gpu_evaluates() {
        let Some(mut gpu) = headless() else { return };
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let channels = [Channel::BaseColor, Channel::Roughness, Channel::Height];
        let mut project = Project::new("Oracle", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, TILE * 2));
        let owner = project.alloc_id();
        let sources = [
            (Channel::BaseColor, Source::Uniform([0.47, 0.3, 0.58])),
            (Channel::Roughness, Source::Uniform([0.41; 3])),
            (Channel::Height, Source::Uniform([-0.34; 3])),
        ];
        let mut layer = Layer::new(owner, "Surface", LayerKind::Fill(Fill { sources: sources.into(), ..Default::default() }), &channels);
        let mut invert = Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Invert));
        invert.opacity = 0.77;
        layer.content_effects.push(invert);
        let mut mask = Mask::new(MaskBackground::Black);
        let fill = EffectKind::Fill { source: Source::Uniform([0.36; 3]), projection: Projection::default(), sampling: Sampling::SAVED_BEFORE };
        mask.effects.push(Effect::new(project.alloc_id(), fill));
        mask.effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius: 24.0, wrap: false })));
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();

        let (pixels, baked, images) = (PixelStore::default(), BakedMaps::default(), ImageStore::default());
        let mut case = OracleCase::new(&project, &pixels, &baked, &images, &mesh);
        case.mask_owner = Some(owner);
        case.region = Some(CompositeRegion { min: [61, 57], max: [75, 73] });
        assert_matches_cpu(&mut gpu, &case, Tolerance::NEIGHBOURHOOD, "oracle self-test");
    }
}
