use std::collections::BTreeMap;

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
                "fn authored_image_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec3<f32> { return layer.low.rgb; }\n                 fn decal_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> { return vec4(layer.low.rgb, 1.0); }",
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
                self.material_anchors,
                include_str!("shaders/composite_anchors.wgsl"),
                "fn capture_material_anchor(texel: LayerTexel, end: u32, triangle: u32) -> vec3<f32> { return params.anchor_default.rgb; }",
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
        .chain(packages::shader_sources(self.packages))
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

struct Variants {
    module: wgpu::ShaderModule,
    render: [Option<wgpu::ComputePipeline>; 2],
    prepare: [Option<wgpu::ComputePipeline>; 2],
}

pub(super) struct CompositePipelines {
    supports_curvature: bool,
    supports_id: bool,
    variants: BTreeMap<Features, Variants>,
}

impl CompositePipelines {
    pub(super) fn new(supports_curvature: bool, supports_id: bool) -> Self {
        Self { supports_curvature, supports_id, variants: BTreeMap::new() }
    }

    pub(super) fn get(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        program: &CompositeProgram,
    ) -> (wgpu::ComputePipeline, Option<wgpu::ComputePipeline>) {
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
        let module = &variants.module;
        let pipeline = |entry, label, grouped| {
            #[cfg(not(target_arch = "wasm32"))]
            let started = std::time::Instant::now();
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &features.constants(grouped), ..Default::default() },
                cache: None,
            });
            #[cfg(not(target_arch = "wasm32"))]
            log::debug!("GPU pipeline {label} compiled in {:.1} ms ({:?})", started.elapsed().as_secs_f64() * 1000.0, device.adapter_info().backend);
            pipeline
        };
        let grouped = program.groups.depth != 0;
        let render = variants.render[usize::from(grouped)].get_or_insert_with(|| pipeline("compose", "tc_gpu_composite", grouped)).clone();
        // Only material anchor capture needs to walk the group structure while preparing a source.
        let grouped_capture = grouped && features.material_anchors;
        let prepare = features.spatial.then(|| {
            variants.prepare[usize::from(grouped_capture)].get_or_insert_with(|| pipeline("prepare_filter", "tc_spatial_source", grouped_capture)).clone()
        });
        (render, prepare)
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

    /// The same programs as compute pipelines on the adapter, as `CompositePipelines::get`
    /// creates them (both group variants), and the neighbourhood pipelines.
    #[test]
    fn package_shaders_build_compute_pipelines_in_full_and_as_stubs() {
        let Some(gpu) = oracle::headless() else { return };
        let compositor = GpuCompositor::new(&gpu.device);
        let (curvature, id) = (compositor.pipelines.supports_curvature, compositor.pipelines.supports_id);
        // Every package variant; the program with every other feature too is left to the naga
        // check above: Mesa's GL compiler needs over 28 GB for it (llvmpipe, measured)
        // The program with every other feature too stays with the naga check above: Mesa's GL
        // compiler (llvmpipe) runs out of 28 GB on its prepare_filter pipeline, with the package
        // files as stubs too (a property of the base features, not of the packages).
        for features in variants().into_iter().filter(|features| !features.generators) {
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
