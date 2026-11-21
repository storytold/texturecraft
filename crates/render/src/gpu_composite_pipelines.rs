use std::collections::BTreeMap;

use super::{ANCHOR_SOURCE, CompositeProgram, FIRST_EXTENDED_PATTERN, FIRST_GENERATOR, TILE_GENERATOR_OPCODE};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Features {
    patterns: bool,
    tile_generator: bool,
    authored_images: bool,
    generators: bool,
    curvature: bool,
    sampled_generator_inputs: bool,
    spatial: bool,
    anchors: bool,
    material_anchors: bool,
}

impl Features {
    fn of(program: &CompositeProgram) -> Self {
        let mut features = Self { authored_images: !program.material_images.is_empty(), spatial: !program.filters.stages.is_empty(), ..Self::default() };
        features.material_anchors = program.filters.stages.iter().any(|stage| stage.anchor.is_some());
        // Inspect the compiled operations, including sources referenced by
        // generator overrides and fills in the mask preview.
        for op in &program.operations {
            let kind = op.ids[1];
            features.anchors |= kind == ANCHOR_SOURCE;
            features.patterns |= matches!(kind, 1..=9 | 11..=13 | FIRST_EXTENDED_PATTERN..=TILE_GENERATOR_OPCODE);
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

    fn source(self, supports_curvature: bool) -> String {
        fn include(enabled: bool, full: &'static str, stub: &'static str) -> &'static str {
            if enabled { full } else { stub }
        }

        // Naga substitutes overrides on GL, but still emits the inactive
        // function bodies. Omit those helpers so Mesa does not inline them
        // before its dead-code pass.
        [
            "override enable_authored_images: bool = false;\n             override enable_generators: bool = false;\n             override enable_curvature_detail: bool = false;\n             override enable_spatial_fields: bool = false;\n             override enable_anchor_reads: bool = false;",
            include_str!("shaders/noise.wgsl"),
            include(self.patterns, include_str!("shaders/procedural_extra.wgsl"), ""),
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
                "fn previous_field(start: u32, count: u32) -> u32 { return 0u; }\n                 fn filter_value(slot: u32, pixel: vec2<u32>) -> vec4<f32> { return vec4(0.0); }",
            ),
            include(
                self.material_anchors,
                include_str!("shaders/composite_anchors.wgsl"),
                "fn capture_material_anchor(texel: LayerTexel, end: u32, triangle: u32) -> vec3<f32> { return params.anchor_default.rgb; }",
            ),
            include_str!("shaders/composite_groups.wgsl"),
            include_str!("shaders/composite.wgsl"),
        ]
        .join("\n")
    }

    fn constants(self, grouped: bool) -> [(&'static str, f64); 6] {
        [
            ("enable_authored_images", f64::from(u8::from(self.authored_images))),
            ("enable_generators", f64::from(u8::from(self.generators))),
            ("enable_curvature_detail", f64::from(u8::from(self.curvature))),
            ("enable_spatial_fields", f64::from(u8::from(self.spatial))),
            ("enable_anchor_reads", f64::from(u8::from(self.anchors))),
            ("composite_groups", f64::from(u8::from(grouped))),
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
    variants: BTreeMap<Features, Variants>,
}

impl CompositePipelines {
    pub(super) fn new(supports_curvature: bool) -> Self {
        Self { supports_curvature, variants: BTreeMap::new() }
    }

    pub(super) fn get(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        program: &CompositeProgram,
    ) -> (wgpu::ComputePipeline, Option<wgpu::ComputePipeline>) {
        let features = Features::of(program);
        let supports_curvature = self.supports_curvature;
        let variants = self.variants.entry(features).or_insert_with(|| Variants {
            module: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("tc_gpu_composite"),
                source: wgpu::ShaderSource::Wgsl(features.source(supports_curvature).into()),
            }),
            render: [None, None],
            prepare: [None, None],
        });
        let module = &variants.module;
        let pipeline = |entry, label, grouped| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &features.constants(grouped), ..Default::default() },
                cache: None,
            })
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
