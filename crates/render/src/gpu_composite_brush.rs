//! P7, brush and live stroke (plan section 2.6), the compositor side. Owned by the P7 agent; no
//! other package edits this file or `shaders/composite_brush.wgsl`.
//!
//! On the GPU brush: Paint, the Eraser on layers and masks (the ordinary eraser, flagged with
//! `BRUSH_ERASE`), the Projection tool in both views, symmetry copies in both views, and one
//! device stroke feeding every texture set of its size that draws the layer. Pickup strokes
//! (Smudge, Clone, Paint with a material pattern) rasterise on the CPU and composite live on the
//! GPU from `display_stroke_pixels`. Still on the CPU: mask strokes painting a mask image (the
//! live gray needs the texel in `live_brush_paint`), and strokes with a brush alpha image or a
//! library tip.
//!
//! P7 also owns `gpu_brush.rs`, `shaders/brush.wgsl`, crates/engine/src/painting.rs,
//! crates/engine/src/brush_device.rs, `LiveBrush` and `content_paint` in gpu_composite.rs, and
//! the stroke part of `SetCompositor::plan` (the lines before `view_project`).
//!
//! How: a paint operation with a live stroke mode sets `op.ext[2] & BRUSH_OP_BITS`;
//! `painted()` in composite.wgsl then calls `live_brush_paint` (this WGSL file) instead of the
//! premultiplied over. Keep [`uses_shader`] true for every such program.
//!
//! Tables: [`LIBRARY_STILL_ON_CPU`] (no library filter is P7's). The brush cases live in
//! crates/engine and crates/ui-egui tests (see `gpu.rs` mask image stroke test); every port gets
//! a GPU-versus-CPU stroke test during the stroke and after release.

use super::CompositeProgram;
use super::packages::BRUSH_OP_BITS;

/// Library filter ids P7 still leaves on the CPU (see `gpu_composite_packages.rs`).
pub(crate) const LIBRARY_STILL_ON_CPU: &[&str] = &[];

/// The `ext[2]` bit (one of [`BRUSH_OP_BITS`]) of a live paint operation whose stroke erases;
/// `BRUSH_ERASE` in WGSL
const BRUSH_ERASE: u32 = 0x0100_0000;

/// The P7 bits of a live paint operation's `ext[2]`: zero leaves the premultiplied over of
/// `painted()` in composite.wgsl
pub(super) fn live_paint_bits(erase: bool) -> u32 {
    if erase { BRUSH_ERASE } else { 0 }
}

pub(super) const WGSL: &str = include_str!("shaders/composite_brush.wgsl");
/// The same functions without P7's code, for programs that do not use it.
pub(super) const WGSL_STUB: &str =
    "fn live_brush_paint(layer: Operation, stored: vec4<f32>, m: f32, projected: ProjectedBrush) -> vec4<f32> { return stored; }";

/// Whether a program needs the full `composite_brush.wgsl`.
pub(super) fn uses_shader(program: &CompositeProgram) -> bool {
    program.operations.iter().any(|op| op.ext[2] & BRUSH_OP_BITS != 0)
}

/// P7's vec4 of the band uniform (`params.packages[6]`).
pub(super) fn band_uniform(_program: &CompositeProgram) -> [u32; 4] {
    [0; 4]
}

#[cfg(test)]
mod tests {
    use super::super::oracle::headless;
    use super::super::*;
    use crate::{headless::Headless, readback::Readback};
    use texturecraft_compose::{Composite, Inputs, MaskGray, PaintTarget, StrokePreview, composite_tiles, tile_bytes};
    use texturecraft_core::{Layer, TextureSet};
    use texturecraft_geom::primitives;
    use texturecraft_texel::{to_u16, to_u16_signed};

    /// The live Eraser against the CPU's preview of the same erase, over the whole set and in a
    /// partial update: Base Color under a non-Normal blend and the signed Height of a paint
    /// layer (Height listed only), a content paint effect, and a mask paint effect.
    #[test]
    fn a_live_erase_matches_the_cpu_on_layers_and_masks() {
        let Some(mut gpu) = headless() else { return };
        const SIZE: u32 = TILE;
        let size = (SIZE, SIZE);
        let channels = [Channel::BaseColor, Channel::Height];
        let mesh = Arc::new(primitives::plane(1.0, 1));
        let gbuf = Arc::new(UvGBuffer::new(&mesh, 0, SIZE, SIZE));
        let mut project = Project::new("Live erase", "builtin:plane");
        project.texture_sets.push(TextureSet::new("Plane", 0, SIZE));
        let layer_id = project.alloc_id();
        let mut layer = Layer::new(layer_id, "Painted layer", LayerKind::Paint, &channels);
        let base_color = layer.channels.get_mut(&Channel::BaseColor).unwrap();
        base_color.mode = BlendMode::Screen;
        base_color.opacity = 0.7;
        let mut content_paint = Effect::new(project.alloc_id(), EffectKind::Paint);
        content_paint.opacity = 0.75;
        let content_id = content_paint.id;
        layer.content_effects.push(content_paint);
        let mut mask = Mask::new(MaskBackground::White);
        let mask_paint = Effect::new(project.alloc_id(), EffectKind::Paint);
        let mask_id = mask_paint.id;
        mask.effects.push(mask_paint);
        layer.mask = Some(mask);
        project.insert_layer(0, layer, None, false).unwrap();

        let mut pixels = PixelStore::default();
        let color = pixels.layer_mut(layer_id, Channel::BaseColor, size);
        for y in 10..50 {
            for x in 12..54 {
                color.set(x, y, &[to_u16(0.31), to_u16(0.12), to_u16(0.5), to_u16(0.82)]);
            }
        }
        let height = pixels.layer_mut(layer_id, Channel::Height, size);
        for y in 20..60 {
            for x in 6..40 {
                height.set(x, y, &[to_u16_signed(-0.27), to_u16(0.66)]);
            }
        }
        let content_color = pixels.layer_mut(content_id, Channel::BaseColor, size);
        for y in 4..36 {
            for x in 24..62 {
                content_color.set(x, y, &[to_u16(0.05), to_u16(0.4), to_u16(0.3), to_u16(0.6)]);
            }
        }
        let painted_mask = pixels.mask_mut(mask_id, size);
        for y in 30..64 {
            for x in 30..64 {
                painted_mask.set(x, y, &[to_u16(0.2), to_u16(0.9)]);
            }
        }
        let alpha = input_texture(&gpu.device, "test erase stroke", size, 1, wgpu::TextureFormat::R16Uint);
        let mut stroke_alpha = TileImage::new(SIZE, SIZE, 1);
        let mut dense_alpha = vec![0; (SIZE * SIZE) as usize];
        for y in 16..56 {
            for x in 18..58 {
                let value = to_u16(0.15 + 0.02 * (x - 18) as f32);
                stroke_alpha.set(x, y, &[value]);
                dense_alpha[(y * SIZE + x) as usize] = value;
            }
        }
        write_input(&gpu.queue, &alpha, size, (0, 0), &dense_alpha);
        gpu.renderer.set_texture_sets(&gpu.device, &gpu.queue, &[(size, 0)], 1);
        let baked = BakedMaps::default();
        let images = ImageStore::default();
        let partial = CompositeRegion { min: [16, 20], max: [40, 48] };
        let read = |gpu: &Headless, kind: DisplayKind| {
            let texture = gpu.renderer.display_textures()[0][kind as usize];
            Readback::blocking_level(&gpu.device, &gpu.queue, texture, 0).unwrap().2
        };

        // The layer erases its Height only; the content effect every channel it painted
        let height_only = BTreeMap::from([(Channel::Height, [0.0; 3])]);
        for (target, values) in [(PaintTarget::Layer(layer_id), height_only), (PaintTarget::Layer(content_id), BTreeMap::new()), (PaintTarget::Mask(mask_id), BTreeMap::new())] {
            let preview = StrokePreview {
                target,
                alpha: stroke_alpha.clone(),
                opacity: 0.85,
                erase: true,
                values: values.clone(),
                mask_gray: MaskGray::Value(1.0),
                stencil: None,
                carried: BTreeMap::new(),
                carried_mask: None,
            };
            let inputs = Inputs {
                project: &project,
                ts: 0,
                mesh: &mesh,
                gbuf: &gbuf,
                pixels: &pixels,
                baked: &baked,
                block_scales: project.texture_sets[0].block_scales(),
                preview: Some(&preview),
                images: &images,
                bounds: mesh.bounds(),
            };
            let mut cpu = Composite::new(&channels, SIZE);
            composite_tiles(&inputs, &mut cpu, &[(0, 0)]);

            let live_target = match target {
                PaintTarget::Layer(layer) => {
                    let erased = texturecraft_paint::commit::commit_channels(&pixels, layer, &values, true);
                    LiveBrushTarget::Layer { layer, values: erased.into_iter().map(|channel| (channel, [0.0; 3])).collect() }
                }
                PaintTarget::Mask(effect) => LiveBrushTarget::Mask { effect, value: 1.0 },
            };
            let live = LiveBrush { target: live_target, opacity: preview.opacity, erase: true, alpha: alpha.create_view(&Default::default()), stencil: None };
            let mut program = CompositeProgram::try_compile_with_brush(&project, &pixels, 0, None, Some(live)).unwrap();
            program.set_surface_inputs(&mesh, &gbuf, None).unwrap();
            program.set_tile_images(&images);
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();
            let full = Vec::from([DisplayKind::BaseColor, DisplayKind::Height].map(|kind| read(&gpu, kind)));
            program.region = Some(partial);
            gpu.renderer.composite_on_gpu(&gpu.device, &gpu.queue, 0, &program).unwrap();

            for (kind, actual) in [DisplayKind::BaseColor, DisplayKind::Height].into_iter().zip(&full) {
                let expected = tile_bytes(&cpu, &baked, kind, 0, 0);
                assert_eq!(actual.len(), expected.len());
                if kind == DisplayKind::Height {
                    for (actual, expected) in actual.as_chunks::<2>().0.iter().zip(expected.as_chunks::<2>().0) {
                        let actual = half::f16::from_le_bytes(*actual).to_f32();
                        let expected = half::f16::from_le_bytes(*expected).to_f32();
                        assert!((actual - expected).abs() <= 0.001, "{target:?}: GPU height {actual}, CPU {expected}");
                    }
                } else {
                    assert!(actual.iter().zip(&expected).all(|(a, b)| a.abs_diff(*b) <= 1), "{target:?}: GPU color differs from CPU");
                }
                assert_eq!(&read(&gpu, kind), actual, "{target:?} {kind:?}: the partial update keeps the full result");
            }
        }
    }
}
