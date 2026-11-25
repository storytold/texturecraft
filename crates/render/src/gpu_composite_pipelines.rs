use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::packages::{self, ShaderFlags};
use super::{ANCHOR_PUBLISH_LAYER, ANCHOR_SOURCE, COLOR_SELECTION_OPCODE, CompositeProgram, FIRST_EXTENDED_PATTERN, FIRST_GENERATOR, TILE_GENERATOR_OPCODE};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Features {
    patterns: bool,
    extended_patterns: bool,
    tile_generator: bool,
    authored_images: bool,
    mask_preview: bool,
    extended_blend_modes: bool,
    generators: bool,
    curvature: bool,
    sampled_generator_inputs: bool,
    spatial: bool,
    mask_effects: bool,
    content_effects: bool,
    height_mask: bool,
    anchors: bool,
    material_anchors: bool,
    color_selection: bool,
    // The GPU port packages' WGSL files compiled in full (gpu_composite_packages.rs)
    packages: ShaderFlags,
}

impl Features {
    fn of(program: &CompositeProgram) -> Self {
        let mut features = Self {
            authored_images: !program.material_images.is_empty(),
            mask_preview: program.is_mask_preview(),
            spatial: !program.filters.stages.is_empty(),
            packages: ShaderFlags::of(program),
            ..Self::default()
        };
        features.material_anchors = program.filters.stages.iter().any(|stage| stage.anchor.is_some());
        // A capture stage (an anchor or a stack read) runs the prepare pass's anchor path
        features.anchors |= features.material_anchors;
        for op in program.operations.iter().take(program.layer_count as usize).filter(|op| op.ids[1] != ANCHOR_PUBLISH_LAYER) {
            features.mask_effects |= op.effects[1] != 0;
            features.content_effects |= op.effects[3] != 0;
            features.height_mask |= op.constraints[2] != 0;
        }
        // Inspect the compiled operations, including sources referenced by
        // generator overrides and fills in the mask preview.
        for op in &program.operations {
            let kind = op.ids[1];
            features.extended_blend_modes |= op.balance[2] > 8.0;
            features.anchors |= kind == ANCHOR_SOURCE;
            features.color_selection |= kind == COLOR_SELECTION_OPCODE;
            features.patterns |= matches!(kind, 1..=9 | 11..=13 | FIRST_EXTENDED_PATTERN..=TILE_GENERATOR_OPCODE);
            features.extended_patterns |= (FIRST_EXTENDED_PATTERN..TILE_GENERATOR_OPCODE).contains(&kind);
            features.tile_generator |= kind == TILE_GENERATOR_OPCODE;
            if (FIRST_GENERATOR..FIRST_EXTENDED_PATTERN).contains(&kind) {
                features.generators = true;
                features.curvature |= op.uv[1] != 0.0;
                if let Some(source) = op.effects[3].checked_sub(1).and_then(|index| program.operations.get(index as usize)) {
                    features.sampled_generator_inputs |= source.ids[1] != 0;
                }
            }
        }
        features
    }

    fn source(self, supports_curvature: bool, supports_id: bool) -> String {
        fn include(enabled: bool, full: &'static str, stub: &'static str) -> &'static str {
            if enabled { full } else { stub }
        }

        // Naga substitutes overrides on GL, but still emits the inactive
        // function bodies. Omit those helpers so Mesa does not inline them
        // before its dead-code pass.
        [
            "override enable_authored_images: bool = false;\n             override enable_generators: bool = false;\n             override enable_curvature_detail: bool = false;\n             override enable_spatial_fields: bool = false;\n             override enable_anchor_reads: bool = false;",
            include_str!("shaders/noise.wgsl"),
            // Keep the gradient fallback when the program has no extended patterns.
            include(
                self.extended_patterns,
                include_str!("shaders/procedural_extra.wgsl"),
                "fn extended_pattern(kind: u32, p: vec3<f32>, seed: u32) -> f32 { return fract(p.x); }",
            ),
            include(
                self.tile_generator,
                include_str!("shaders/tile_generator.wgsl"),
                "fn tile_generator(offset: u32, uv: vec2<f32>) -> f32 { return 0.0; }",
            ),
            include(
                self.patterns,
                include_str!("shaders/composite_patterns.wgsl"),
                "fn procedural_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec3<f32> { return layer.low.rgb; }",
            ),
            include(
                self.authored_images,
                include_str!("shaders/material_image.wgsl"),
                "fn authored_image_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec3<f32> { return layer.low.rgb; }\n                 fn decal_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> { return vec4(layer.low.rgb, 1.0); }\n                 fn material_image_texel(image: u32, pixel: vec2<i32>, size: vec2<i32>) -> vec4<f32> { return vec4(0.0); }\n                 const MISSING_IMAGE: u32 = 0xffffffffu;\n                 fn decode_image_sample(layer: Operation, rgb: vec3<f32>) -> vec3<f32> { return rgb; }",
            ),
            include_str!("shaders/stencil_projection.wgsl"),
            include(
                self.curvature && supports_curvature,
                include_str!("shaders/curvature_sample.wgsl"),
                "fn curvature_level_at(pixel: vec2<u32>, level: u32) -> f32 { return 0.0; }",
            ),
            include(
                self.curvature,
                include_str!("shaders/composite_curvature_detail.wgsl"),
                "fn generator_curvature(effect: Operation, pixel: vec2<u32>, finest: f32, mode: u32, micro: f32) -> f32 { return 0.0; }",
            ),
            include(
                self.sampled_generator_inputs,
                "fn generator_source_value(index: u32, texel: LayerTexel) -> f32 {\n                     return fill_value(layers[index], texel.uv, texel.surface).x;\n                 }",
                "fn generator_source_value(index: u32, texel: LayerTexel) -> f32 { return layers[index].low.x; }",
            ),
            include(
                self.generators,
                include_str!("shaders/composite_generators.wgsl"),
                "fn generator_ramp(effect: Operation, texel: LayerTexel) -> f32 { return 0.0; }\n                 fn finish_generator(effect: Operation, ramp: f32) -> f32 { return ramp; }",
            ),
            include(
                self.spatial,
                include_str!("shaders/composite_spatial.wgsl"),
                "fn previous_field(start: u32, count: u32) -> u32 { return 0u; }\n                 fn filter_value(slot: u32, pixel: vec2<u32>) -> vec4<f32> { return vec4(0.0); }\n                 var<private> preceding_field: u32 = 0xffffffffu;\n                 fn preceding_stage_word(pixel: vec2<u32>) -> f32 { return 0.0; }",
            ),
            include(
                self.color_selection && supports_id,
                include_str!("shaders/id_sample.wgsl"),
                "fn id_color_at(pixel: vec2<u32>) -> vec3<f32> { return vec3(0.0); }",
            ),
            include(
                self.mask_effects,
                include_str!("shaders/composite_mask_effects.wgsl"),
                "fn layer_mask(layer: Operation, texel: LayerTexel, count: u32) -> f32 { return layer.balance.y; }",
            ),
            include(
                self.content_effects,
                include_str!("shaders/composite_content_effects.wgsl"),
                "fn layer_content(layer: Operation, texel: LayerTexel, count: u32) -> vec4<f32> {
                    let content = base_content(layer, texel);
                    return select(vec4(0.0), content, content.a > 0.0);
                }",
            ),
            include(
                self.height_mask,
                include_str!("shaders/composite_height_mask.wgsl"),
                "fn mask_with_height(layer: Operation, texel: LayerTexel, mask: f32) -> f32 { return mask; }",
            ),
            include_str!("shaders/composite_groups.wgsl"),
            include_str!("shaders/composite.wgsl"),
        ]
        .into_iter()
        .map(str::to_string)
        .chain(packages::shader_sources(self.packages).into_iter().map(str::to_string))
        .chain(std::iter::once(super::channel_defaults_wgsl()))
        .collect::<Vec<_>>()
        .join("\n")
    }

    fn constants(self, grouped: bool) -> [(&'static str, f64); 8] {
        [
            ("enable_authored_images", f64::from(u8::from(self.authored_images))),
            ("enable_generators", f64::from(u8::from(self.generators))),
            ("enable_curvature_detail", f64::from(u8::from(self.curvature))),
            ("enable_spatial_fields", f64::from(u8::from(self.spatial))),
            ("enable_anchor_reads", f64::from(u8::from(self.anchors))),
            ("composite_groups", f64::from(u8::from(grouped))),
            ("composite_mask_preview", f64::from(u8::from(self.mask_preview))),
            ("enable_extended_blend_modes", f64::from(u8::from(self.extended_blend_modes))),
        ]
    }
}

/// A pipeline compiled, or compiling on a worker thread (`CompositePipelines::asynchronous`)
enum Slot {
    Ready(wgpu::ComputePipeline),
    // The web compiles in place: no worker threads there
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Compiling(Arc<Mutex<Option<wgpu::ComputePipeline>>>),
}

impl Slot {
    /// The pipeline once compiled; `wait` waits for a compile in flight (a caller that compiles
    /// in place, after a pre-warm started one on a worker thread)
    fn ready(&mut self, wait: bool) -> Option<wgpu::ComputePipeline> {
        if let Slot::Compiling(compiled) = self {
            let pipeline = loop {
                if let Some(pipeline) = compiled.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    break pipeline;
                }
                if !wait {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            };
            *self = Slot::Ready(pipeline);
        }
        match self {
            Slot::Ready(pipeline) => Some(pipeline.clone()),
            Slot::Compiling(_) => None,
        }
    }
}

struct Variants {
    module: wgpu::ShaderModule,
    render: [Option<Slot>; 2],
    prepare: [Option<Slot>; 2],
}

pub(super) struct CompositePipelines {
    supports_curvature: bool,
    supports_id: bool,
    variants: BTreeMap<Features, Variants>,
}

/// The pipelines a compositor uses: its own, or when `shared` (a compositor on the device tests
/// share, [`crate::headless::Headless::for_tests`]) one set for every such compositor, so a test
/// does not compile the variants an earlier one compiled (llvmpipe keeps what it compiled until
/// the process ends, and compiling dominates its tests' time).
pub(super) fn shared(shared: bool, supports_curvature: bool, supports_id: bool) -> Arc<Mutex<CompositePipelines>> {
    #[cfg(all(any(test, feature = "test-support"), not(target_arch = "wasm32")))]
    if shared {
        static SHARED: std::sync::OnceLock<Arc<Mutex<CompositePipelines>>> = std::sync::OnceLock::new();
        return SHARED.get_or_init(|| Arc::new(Mutex::new(CompositePipelines::new(supports_curvature, supports_id)))).clone();
    }
    #[cfg(not(all(any(test, feature = "test-support"), not(target_arch = "wasm32"))))]
    let _ = shared;
    Arc::new(Mutex::new(CompositePipelines::new(supports_curvature, supports_id)))
}

impl CompositePipelines {
    pub(super) fn new(supports_curvature: bool, supports_id: bool) -> Self {
        Self { supports_curvature, supports_id, variants: BTreeMap::new() }
    }

    /// Start compiling `program`'s pipelines without waiting (`get` returns them once ready).
    pub(super) fn prewarm(&mut self, device: &wgpu::Device, layout: &wgpu::PipelineLayout, program: &CompositeProgram) {
        let _ = self.get(device, layout, program, !cfg!(target_arch = "wasm32"));
    }

    /// The program's compose and prepare pipelines; None while one still compiles (new ones
    /// compile on worker threads when `asynchronous`: the viewport keeps its last result).
    pub(super) fn get(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        program: &CompositeProgram,
        asynchronous: bool,
    ) -> Option<(wgpu::ComputePipeline, Option<wgpu::ComputePipeline>)> {
        let features = Features::of(program);
        let supports_curvature = self.supports_curvature;
        let supports_id = self.supports_id;
        let variants = self.variants.entry(features).or_insert_with(|| Variants {
            module: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("tc_gpu_composite"),
                source: wgpu::ShaderSource::Wgsl(features.source(supports_curvature, supports_id).into()),
            }),
            render: [None, None],
            prepare: [None, None],
        });
        let module = variants.module.clone();
        let pipeline = |entry: &'static str, label: &'static str, grouped: bool| {
            let compile = {
                let (device, layout, module) = (device.clone(), layout.clone(), module.clone());
                move || {
                    #[cfg(not(target_arch = "wasm32"))]
                    let started = std::time::Instant::now();
                    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(label),
                        layout: Some(&layout),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: wgpu::PipelineCompilationOptions { constants: &features.constants(grouped), ..Default::default() },
                        cache: None,
                    });
                    #[cfg(not(target_arch = "wasm32"))]
                    log::debug!("GPU pipeline {label} compiled in {:.1} ms ({:?})", started.elapsed().as_secs_f64() * 1000.0, device.adapter_info().backend);
                    pipeline
                }
            };
            #[cfg(not(target_arch = "wasm32"))]
            if asynchronous {
                let compiled = Arc::new(Mutex::new(None));
                let sink = compiled.clone();
                std::thread::spawn(move || {
                    let pipeline = compile();
                    *sink.lock().unwrap_or_else(PoisonError::into_inner) = Some(pipeline);
                });
                return Slot::Compiling(compiled);
            }
            Slot::Ready(compile())
        };
        let grouped = program.groups.depth != 0;
        let render = variants.render[usize::from(grouped)].get_or_insert_with(|| pipeline("compose", "tc_gpu_composite", grouped)).ready(!asynchronous);
        // Only material anchor capture needs to walk the group structure while preparing a source.
        let grouped_capture = grouped && features.material_anchors;
        let prepare = match features.spatial {
            true => Some(
                variants.prepare[usize::from(grouped_capture)]
                    .get_or_insert_with(|| pipeline("prepare_filter", "tc_spatial_source", grouped_capture))
                    .ready(!asynchronous)?,
            ),
            false => None,
        };
        Some((render?, prepare))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{Features, ShaderFlags};
    use crate::gpu_composite::{GpuCompositor, oracle};
    use crate::gpu_neighbourhood::NeighbourhoodGpu;

    /// Every package stub, each package in full alone and all in full, each with and without
    /// neighbourhood fields, plus a mask preview and a program using every other feature.
    fn variants() -> Vec<Features> {
        let mut packages = vec![ShaderFlags::default(), ShaderFlags::ALL];
        packages.extend(ShaderFlags::EACH);
        let mut variants: Vec<Features> = packages
            .into_iter()
            .flat_map(|packages| [false, true].map(|spatial| Features { spatial, mask_effects: true, content_effects: true, packages, ..Features::default() }))
            .collect();
        variants.push(Features { mask_preview: true, spatial: true, mask_effects: true, packages: ShaderFlags::ALL, ..Features::default() });
        variants.push(Features {
            patterns: true,
            extended_patterns: true,
            tile_generator: true,
            authored_images: true,
            mask_preview: false,
            extended_blend_modes: true,
            generators: true,
            curvature: true,
            sampled_generator_inputs: true,
            spatial: true,
            mask_effects: true,
            content_effects: true,
            height_mask: true,
            anchors: true,
            material_anchors: true,
            color_selection: true,
            packages: ShaderFlags::ALL,
        });
        variants
    }

    fn entry_points(features: Features) -> Vec<&'static str> {
        if features.spatial { vec!["compose", "prepare_filter"] } else { vec!["compose"] }
    }

    #[allow(clippy::panic)] // A test helper: a shader that does not translate fails the test
    /// Parse and validate `source`, then write GLSL (GLES 3.1 and desktop 4.3, the GL backends'
    /// path) for every entry point with the override constants of both group variants.
    fn naga_check(label: &str, source: &str, entries: &[&str], constants: &[Vec<(&'static str, f64)>]) {
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|error| panic!("{label}: {}", error.emit_to_string(source)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default() | naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32,
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("{label}: {error:?}"));
        for constants in constants {
            let mut values = naga::back::PipelineConstants::default();
            values.extend(constants.iter().map(|&(name, value)| (name.to_owned(), value)));
            for &entry in entries {
                let (module, info) = naga::back::pipeline_constants::process_overrides(&module, &info, Some((naga::ShaderStage::Compute, entry)), &values)
                    .unwrap_or_else(|error| panic!("{label} {entry}: {error}"));
                for version in [naga::back::glsl::Version::new_gles(310), naga::back::glsl::Version::Desktop(430)] {
                    let options = naga::back::glsl::Options { version, ..Default::default() };
                    let pipeline =
                        naga::back::glsl::PipelineOptions { shader_stage: naga::ShaderStage::Compute, entry_point: entry.to_owned(), multiview: None };
                    let mut glsl = String::new();
                    naga::back::glsl::Writer::new(&mut glsl, &module, &info, &options, &pipeline, naga::proc::BoundsCheckPolicies::default())
                        .and_then(|mut writer| writer.write())
                        .unwrap_or_else(|error| panic!("{label} {entry} {version:?} {constants:?}: {error}"));
                }
            }
        }
    }

    /// Every package WGSL file and stub, in the composite and the neighbourhood shader, without
    /// an adapter: so this never skips.
    #[test]
    fn package_shaders_validate_and_write_glsl_without_an_adapter() {
        for features in variants() {
            for (curvature, id) in [(false, false), (true, true)] {
                let constants = [false, true].map(|grouped| features.constants(grouped).to_vec());
                naga_check(&format!("{features:?} curvature {curvature} id {id}"), &features.source(curvature, id), &entry_points(features), &constants);
            }
        }
        naga_check("neighbourhood", &NeighbourhoodGpu::source(), &NeighbourhoodGpu::ENTRY_POINTS, &[Vec::new()]);
    }

    /// With asynchronous compiles a new program first answers GPU_PIPELINES_PENDING, leaving the
    /// set's textures as they are, then composites once its pipelines are ready; a pre-warmed
    /// program composites at once.
    #[test]
    #[allow(clippy::unwrap_used)] // A test: a program that does not build fails it
    fn new_pipelines_compile_on_worker_threads() {
        use std::collections::BTreeMap;

        use texturecraft_compose::PixelStore;
        use texturecraft_core::{Channel, Effect, EffectKind, Fill, Filter, Layer, LayerKind, Project, Source, TextureSet};

        use crate::gpu_composite::{CompositeProgram, GPU_PIPELINES_PENDING};

        // A device of its own: the tests share their pipelines per device
        let Ok(mut gpu) = crate::headless::Headless::with_limits(crate::device_limits) else { return };
        let project = |radius: f32| {
            let mut project = Project::new("Asynchronous", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, 64));
            let id = project.alloc_id();
            let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::Uniform([0.6, 0.3, 0.2]))]), ..Default::default() };
            let mut layer = Layer::new(id, "Fill", LayerKind::Fill(fill), &[Channel::BaseColor]);
            layer.content_effects.push(Effect::new(project.alloc_id(), EffectKind::Filter(Filter::Blur { radius, wrap: false })));
            project.insert_layer(0, layer, None, false).unwrap();
            project
        };
        let mesh = std::sync::Arc::new(texturecraft_geom::primitives::plane(1.0, 1));
        let gbuf = std::sync::Arc::new(texturecraft_geom::UvGBuffer::new(&mesh, 0, 64, 64));
        let program = |project: &Project| {
            let mut program = CompositeProgram::compile(project, &PixelStore::default(), 0).unwrap();
            program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
            program
        };
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[((64, 64), 0)], 1);
        gpu.renderer.set_asynchronous_compile(true);
        let first = program(&project(8.0));
        assert_eq!(gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &first), Err(GPU_PIPELINES_PENDING.to_string()));
        let started = std::time::Instant::now();
        loop {
            match gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &first) {
                Err(error) if error == GPU_PIPELINES_PENDING => {
                    assert!(started.elapsed() < std::time::Duration::from_secs(300), "the pipelines never finished");
                    std::thread::yield_now();
                }
                result => {
                    assert_eq!(result, Ok(()));
                    break;
                }
            }
        }
        // The same features again: ready at once
        assert_eq!(gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program(&project(16.0))), Ok(()));
    }

    /// Cold compile times of realistic programs' pipelines (run with the driver's shader cache
    /// off, e.g. `__GL_SHADER_DISK_CACHE=0` on NVIDIA): a material of fills with Blur, Levels and
    /// lighting under a generator mask, a mask comparing the layers below and this layer, and the
    /// test variant with every package and feature.
    #[test]
    #[ignore = "a measurement: cold pipeline compiles"]
    #[allow(clippy::unwrap_used)] // A measurement: a program that does not build fails it
    fn bench_cold_pipeline_compile() {
        use std::collections::BTreeMap;

        use texturecraft_compose::PixelStore;
        use texturecraft_core::{
            Channel, CompareOp, CompareSource, Effect, EffectKind, Fill, Filter, Generator, GeneratorKind, Layer, LayerKind, Levels, Mask, MaskBackground,
            Project, Source, StylizedLighting, TextureSet,
        };

        use crate::gpu_composite::CompositeProgram;

        let Some(gpu) = oracle::headless() else { return };
        let channels = [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Normal, Channel::Height];
        let fill = |values: [f32; 3]| Fill {
            sources: channels.iter().map(|&channel| (channel, Source::Uniform(values))).collect::<BTreeMap<_, _>>(),
            ..Default::default()
        };
        let project = |content: Vec<EffectKind>, mask: Vec<EffectKind>| {
            let mut project = Project::new("Compile", "builtin:plane");
            project.texture_sets.push(TextureSet::new("Plane", 0, 256));
            let below = project.alloc_id();
            project.insert_layer(0, Layer::new(below, "Below", LayerKind::Fill(fill([0.3; 3])), &channels), None, false).unwrap();
            let owner = project.alloc_id();
            let mut layer = Layer::new(owner, "Layer", LayerKind::Fill(fill([0.6; 3])), &channels);
            for kind in content {
                layer.content_effects.push(Effect::new(project.alloc_id(), kind));
            }
            if !mask.is_empty() {
                let mut layer_mask = Mask::new(MaskBackground::White);
                for kind in mask {
                    layer_mask.effects.push(Effect::new(project.alloc_id(), kind));
                }
                layer.mask = Some(layer_mask);
            }
            project.insert_layer(0, layer, None, false).unwrap();
            project
        };
        let lighting = Filter::BakedLighting {
            azimuth: 30.0,
            elevation: 50.0,
            light: 1.0,
            ambient: 1.0,
            occlusion: 0.4,
            stylized: Some(Box::new(StylizedLighting::library())),
        };
        let uv_tiles = |mut project: Project| {
            let set = &mut project.texture_sets[0];
            set.uv_tiles = vec![1001, 1002, 1011, 1012];
            set.tile_resolutions.extend([(1002, 128), (1012, 64)]);
            project
        };
        let compare = |left| EffectKind::Compare {
            op: CompareOp::GreaterThan,
            value: 0.4,
            tolerance: 0.1,
            left,
            right: CompareSource::Constant,
            channel: Channel::Roughness,
            hardness: 0.5,
        };
        let cases = [
            (
                "material: Blur, Levels, lighting, generator mask",
                project(
                    vec![
                        EffectKind::Filter(Filter::Blur { radius: 8.0, wrap: false }),
                        EffectKind::Levels(Levels::default()),
                        EffectKind::Filter(lighting.clone()),
                    ],
                    vec![EffectKind::Generator(Generator::new(GeneratorKind::Dirt))],
                ),
            ),
            ("mask comparing Layers Below and This Layer", project(Vec::new(), vec![compare(CompareSource::LayersBelow), compare(CompareSource::ThisLayer)])),
            ("plain fill", project(Vec::new(), Vec::new())),
            // P8: the same material on a UV tile atlas with three tile scales
            (
                "UV tile atlas: Blur, Levels, lighting, generator mask",
                uv_tiles(project(
                    vec![
                        EffectKind::Filter(Filter::Blur { radius: 8.0, wrap: false }),
                        EffectKind::Levels(Levels::default()),
                        EffectKind::Filter(lighting.clone()),
                    ],
                    vec![EffectKind::Generator(Generator::new(GeneratorKind::Dirt))],
                )),
            ),
            ("UV tile atlas: plain fill", uv_tiles(project(Vec::new(), Vec::new()))),
        ];
        let compositor = GpuCompositor::new(&gpu.device);
        let (curvature, id) = {
            let pipelines = compositor.pipelines();
            (pipelines.supports_curvature, pipelines.supports_id)
        };
        let programs =
            cases.map(|(label, project)| (label, Features::of(&CompositeProgram::try_compile_region(&project, &PixelStore::default(), 0, None).unwrap())));
        let everything =
            variants().into_iter().filter(|features| features.mask_preview || (features.spatial && !features.generators)).map(|features| ("variant", features));
        for (label, features) in programs.into_iter().chain(everything) {
            let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("tc_bench"),
                source: wgpu::ShaderSource::Wgsl(features.source(curvature, id).into()),
            });
            for entry in entry_points(features) {
                let started = std::time::Instant::now();
                let _pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(&compositor.pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions { constants: &features.constants(false), ..Default::default() },
                    cache: None,
                });
                let memory = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
                let peak = memory.lines().filter(|line| line.starts_with("VmHWM") || line.starts_with("VmRSS")).collect::<Vec<_>>().join(" ");
                eprintln!("COMPILE {label} {:?} preview {}, {entry}: {:?} {peak}", features.packages, features.mask_preview, started.elapsed());
            }
        }
    }

    /// The programs with every package in full and with every package as a stub as compute
    /// pipelines on the adapter, as `CompositePipelines::get` creates them (both group variants),
    /// and the neighbourhood pipelines. The variants with one package in full are checked by naga
    /// above: on the adapter they only repeat these compiles.
    #[test]
    fn package_shaders_build_compute_pipelines_in_full_and_as_stubs() {
        let Some(gpu) = oracle::headless() else { return };
        let compositor = GpuCompositor::new(&gpu.device);
        let (curvature, id) = {
            let pipelines = compositor.pipelines();
            (pipelines.supports_curvature, pipelines.supports_id)
        };
        // Every package variant; the program with every other feature too is left to the naga
        // check above: Mesa's GL compiler needs over 28 GB for it (llvmpipe, measured)
        // The program with every other feature too stays with the naga check above: Mesa's GL
        // compiler (llvmpipe) runs out of 28 GB on its prepare_filter pipeline, with the package
        // files as stubs too (a property of the base features, not of the packages).
        let one_package = |features: &Features| ShaderFlags::EACH.contains(&features.packages);
        for features in variants().into_iter().filter(|features| !features.generators && !one_package(features)) {
            let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let module = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("tc_gpu_composite_packages"),
                source: wgpu::ShaderSource::Wgsl(features.source(curvature, id).into()),
            });
            for grouped in [false, true] {
                for entry in entry_points(features) {
                    let _pipeline = gpu.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(&compositor.pipeline_layout),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: wgpu::PipelineCompilationOptions { constants: &features.constants(grouped), ..Default::default() },
                        cache: None,
                    });
                }
            }
            let error = pollster::block_on(scope.pop());
            assert!(error.is_none(), "{features:?} on {}: {error:?}", gpu.adapter_name);
        }
        let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _neighbourhood = NeighbourhoodGpu::new(&gpu.device);
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "neighbourhood on {}: {error:?}", gpu.adapter_name);
    }
}
