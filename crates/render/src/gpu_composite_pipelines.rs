use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use super::{
    ANCHOR_PUBLISH_LAYER, ANCHOR_SOURCE, COLOR_SELECTION_OPCODE, CompositeProgram, FIRST_EXTENDED_PATTERN, FIRST_GENERATOR, PLACED_UV_PROJECTION,
    TILE_GENERATOR_OPCODE,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Features {
    patterns: bool,
    extended_patterns: bool,
    tile_generator: bool,
    authored_images: bool,
    placed_uv: bool,
    curved_warp: bool,
    mask_preview: bool,
    extended_blend_modes: bool,
    measured_blend: bool,
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
}

impl Features {
    fn of(program: &CompositeProgram) -> Self {
        let mut features = Self {
            authored_images: !program.material_images.is_empty(),
            curved_warp: program.material_images.has_curved_warp(),
            mask_preview: program.is_mask_preview(),
            measured_blend: program.measured_blend,
            extended_blend_modes: program.measured_blend && program.groups.depth != 0,
            spatial: !program.filters.stages.is_empty(),
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
            features.placed_uv |= op.procedural[3] == PLACED_UV_PROJECTION;
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
                "fn authored_image_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec3<f32> { return layer.low.rgb; }\n                 fn decal_fill_value(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> { return vec4(layer.low.rgb, 1.0); }
                 fn authored_image_content(layer: Operation, uv: vec2<f32>, surface: Surface) -> vec4<f32> { return vec4(layer.low.rgb, 1.0); }
                 fn authored_image_placed(layer: Operation, uv: vec2<f32>, repeat: vec2<bool>) -> vec4<f32> { return vec4(0.5, 0.5, 0.5, 1.0); }
                 fn authored_image_triplanar(layer: Operation, surface: Surface, with_alpha: bool) -> vec4<f32> { return vec4(layer.low.rgb, 1.0); }
                 fn authored_image_warp(layer: Operation, surface: Surface, with_alpha: bool) -> vec4<f32> { return vec4(layer.low.rgb, 1.0); }",
            ),
            include(
                self.curved_warp,
                include_str!("shaders/material_warp.wgsl"),
                "fn warp_inverse(data: u32, position: vec2f) -> vec4f { return vec4f(0.0); }",
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
                 }
                 fn layer_content_from(layer: Operation, texel: LayerTexel, count: u32, previous: vec4<f32>, passthrough: bool) -> vec4<f32> {
                     return layer_content(layer, texel, count);
                 }",
            ),
            include(
                self.height_mask,
                include_str!("shaders/composite_height_mask.wgsl"),
                "fn mask_with_height(layer: Operation, texel: LayerTexel, mask: f32) -> f32 { return mask; }",
            ),
            include_str!("shaders/composite_groups.wgsl"),
            include(
                self.measured_blend,
                include_str!("shaders/composite_measured.wgsl"),
                "fn empty_channel(channel: u32) -> vec3<f32> { return vec3(0.0); }
                 fn layer_blended_scalar(mode: u32, a: f32, b: f32, unit: bool) -> f32 { return blend_scalar(mode, a, b); }
                 fn layer_blended_color(mode: u32, a: vec3<f32>, b: vec3<f32>, channel: u32) -> vec3<f32> { return blend_rgb(mode, a, b); }
                 fn measured_over(mode: u32, below: vec4<f32>, top: vec4<f32>, strength: f32, channel: u32) -> vec4<f32> { return below; }",
            ),
            include_str!("shaders/composite.wgsl"),
        ]
        .join("\n")
    }

    fn constants(self, grouped: bool) -> [(&'static str, f64); 10] {
        [
            ("enable_authored_images", f64::from(u8::from(self.authored_images))),
            ("enable_placed_uv", f64::from(u8::from(self.placed_uv))),
            ("enable_generators", f64::from(u8::from(self.generators))),
            ("enable_curvature_detail", f64::from(u8::from(self.curvature))),
            ("enable_spatial_fields", f64::from(u8::from(self.spatial))),
            ("enable_anchor_reads", f64::from(u8::from(self.anchors))),
            ("composite_groups", f64::from(u8::from(grouped))),
            ("composite_mask_preview", f64::from(u8::from(self.mask_preview))),
            ("enable_extended_blend_modes", f64::from(u8::from(self.extended_blend_modes))),
            ("measured_blend", f64::from(u8::from(self.measured_blend))),
        ]
    }
}

#[derive(Clone)]
struct Variants {
    module: wgpu::ShaderModule,
    render: [Option<wgpu::ComputePipeline>; 2],
    prepare: [Option<wgpu::ComputePipeline>; 2],
}

#[derive(Clone)]
pub(super) struct CompositePipelines {
    supports_curvature: bool,
    supports_id: bool,
    variants: Arc<Mutex<BTreeMap<Features, Variants>>>,
}

impl CompositePipelines {
    pub(super) fn new(supports_curvature: bool, supports_id: bool) -> Self {
        Self { supports_curvature, supports_id, variants: Arc::default() }
    }

    pub(super) fn get(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::PipelineLayout,
        program: &CompositeProgram,
    ) -> Result<(wgpu::ComputePipeline, Option<wgpu::ComputePipeline>), String> {
        let features = Features::of(program);
        let grouped = program.groups.depth != 0;
        let render_index = usize::from(grouped);
        // Only material anchor capture needs to walk the group structure while preparing a source.
        let prepare_index = usize::from(grouped && features.material_anchors);
        let existing = self.variants.lock().unwrap_or_else(PoisonError::into_inner).get(&features).cloned();
        if let Some(variants) = &existing
            && let Some(render) = &variants.render[render_index]
            && (!features.spatial || variants.prepare[prepare_index].is_some())
        {
            return Ok((render.clone(), variants.prepare[prepare_index].clone()));
        }
        // Driver compilation may take seconds. Keep the cache lock out of this block so the
        // viewport can use already prepared materials while another variant is built.
        let variants = crate::gpu_trace::scoped(device, "GPU material pipelines", || {
            let mut variants = existing.unwrap_or_else(|| Variants {
                module: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("tc_gpu_composite"),
                    source: wgpu::ShaderSource::Wgsl(features.source(self.supports_curvature, self.supports_id).into()),
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
            variants.render[render_index].get_or_insert_with(|| pipeline("compose", "tc_gpu_composite", grouped));
            if features.spatial {
                variants.prepare[prepare_index].get_or_insert_with(|| pipeline("prepare_filter", "tc_spatial_source", prepare_index != 0));
            }
            variants
        });
        let variants = variants?;
        let mut cache = self.variants.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = cache.entry(features).or_insert_with(|| variants.clone());
        for (old, new) in entry.render.iter_mut().zip(variants.render) {
            if old.is_none() {
                *old = new;
            }
        }
        for (old, new) in entry.prepare.iter_mut().zip(variants.prepare) {
            if old.is_none() {
                *old = new;
            }
        }
        let render = entry.render[render_index].clone().ok_or("The GPU material pipeline is missing.")?;
        Ok((render, entry.prepare[prepare_index].clone()))
    }
}
