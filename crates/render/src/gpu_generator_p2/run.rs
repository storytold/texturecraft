use bytemuck::Zeroable;
use texturecraft_core::MicroCurvatureType;

use super::block_group::BlockGroup;
use super::capture_layout::CaptureLayout;
use super::dummies::Dummies;
use super::fields::Fields;
use super::micro_gpu::MicroGpu;
use super::micro_request::MicroRequest;
use super::params::Params;
use super::slot_data::SlotData;
use super::{
    BOX_BLUR_MIN_SIGMA, FIRST, HEIGHT_NORMAL_STRENGTH, HEIGHTS, KERNEL_TAPS, LINE_WORKGROUP, READS_HEIGHT, REFERENCE_TILE_TEXELS, SECOND, SMOOTH_SIGMAS,
    WORKGROUP, dispatch, gaussian_kernel, occlusion_taps, type_constants,
};

/// The scratch fields of each block size, shared by every run of one build: the recorded passes
/// run in order and each first writes what it reads, so one set per size serves them all, and the
/// scratch stays that of the distinct block sizes instead of growing with every run
pub(super) type Scratch = std::cell::RefCell<std::collections::BTreeMap<[u32; 2], [wgpu::Buffer; 6]>>;

/// The passes of one request's map over one block size
pub(super) struct Run<'a> {
    pub(super) gpu: &'a MicroGpu,
    pub(super) device: &'a wgpu::Device,
    pub(super) slots: &'a SlotData,
    pub(super) dummy: &'a Dummies,
    pub(super) target: &'a wgpu::TextureView,
    pub(super) group: &'a BlockGroup,
    pub(super) layer: u32,
    /// The anchor heights the height slot reads, where they lie, and the map's size
    pub(super) capture: Option<(&'a wgpu::TextureView, &'a CaptureLayout, [u32; 2])>,
    pub(super) scratch: &'a Scratch,
}

impl Run<'_> {
    /// Whether the block's fields fit the storage-buffer limit
    pub(super) fn fits(&self, limit: u64) -> Result<(), String> {
        let texels = u64::from(self.group.size[0]) * u64::from(self.group.size[1]);
        if texels * 8 > limit {
            return Err("A Micro Details block exceeds the GPU storage-buffer limit.".into());
        }
        Ok(())
    }

    pub(super) fn buffers(&self) -> Fields<'_> {
        let texels = u64::from(self.group.size[0]) * u64::from(self.group.size[1]);
        let buffer = |bytes: u64| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tc_micro_field"),
                size: bytes.max(16),
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let fields = self
            .scratch
            .borrow_mut()
            .entry(self.group.size)
            .or_insert_with(|| [buffer(texels * 4), buffer(texels * 4), buffer(texels * 4), buffer(texels * 4), buffer(texels * 8), buffer(texels * 8)])
            .clone();
        Fields { slots: &self.slots.buffer, fields }
    }

    /// The parameters every pass of this block size shares
    pub(super) fn params(&self) -> Params {
        let group = self.group;
        Params {
            block: [group.size[0], group.size[1], self.slots.normal_start, 0],
            slots: self.slots.sizes,
            uniforms: self.slots.uniforms,
            origins: [group.origin[0], group.origin[1], group.step[0], group.step[1]],
            place: [group.count[0], group.count[1], self.layer, 0],
            anchor: self.capture.map_or([0; 4], |(_, layout, map)| [map[0], map[1], layout.working[0], layout.working[1]]),
            anchor_block: self.capture.map_or([0.0; 4], |(_, layout, map)| [layout.block as f32, layout.scale_at(group.origin, map), 0.0, 0.0]),
            ..Params::zeroed()
        }
    }

    pub(super) fn pass(&self, encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline, params: &Params, fields: &Fields, lines: Option<u32>) {
        let capture = self.capture.map_or(&self.dummy.capture, |(heights, _, _)| heights);
        let bindings = self.gpu.bindings(self.device, params, fields, (self.target, &self.dummy.bake, &self.dummy.maps_in, &self.dummy.bake_out, capture));
        let groups = match lines {
            Some(count) => [count.div_ceil(LINE_WORKGROUP), 1],
            None => [self.group.size[0].div_ceil(WORKGROUP), self.group.size[1].div_ceil(WORKGROUP)],
        };
        dispatch(encoder, pipeline, &bindings, groups);
    }

    /// The curvature the micro details add over this block size (compose's `curvature_term`)
    pub(super) fn curvature(&self, encoder: &mut wgpu::CommandEncoder, request: &MicroRequest) {
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
    pub(super) fn blur(&self, encoder: &mut wgpu::CommandEncoder, shared: &Params, fields: &Fields, sigma: f32) {
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
    pub(super) fn occlusion(&self, encoder: &mut wgpu::CommandEncoder, request: &MicroRequest) {
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
