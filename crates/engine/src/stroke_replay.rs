//! Ordered paint operations and the raster images they completely describe.

use std::collections::BTreeMap;
use std::sync::{
    Arc, OnceLock, Weak,
    atomic::{AtomicBool, Ordering::Relaxed},
};

use serde::{Deserialize, Serialize};
use texturecraft_bake::Progress;
use texturecraft_compose::{PaintTarget, PixelStore};
use texturecraft_core::{Channel, EffectKind, LayerId, LayerKind, Project};
use texturecraft_formats::{FormatError, StrokeReplayData};
use texturecraft_geom::{Bvh, Mesh, UvGBuffer, glam::Vec2};
use texturecraft_paint::{
    BrushSettings, CommitSource, EyeDepth, RasterCtx, Shape, Staging, Stamp, StrokeCanvas, UvStamp, commit_layer, commit_mask, rasterize, rasterize_uv,
    screen_point, triangle_scan_texels,
};
use texturecraft_texel::{TileIdentity, TileImage};

use crate::{
    EngineError, Result,
    painting::StrokeView,
    uv_stroke_state::{UvStrokeState, dab_upper_bound},
};

const MAX_SAMPLES_PER_STROKE: usize = 16_384;
const MAX_STROKES: usize = 4_096;
const MAX_SAMPLES: usize = 262_144;
const ARCHIVE_VERSION: u32 = 2;
const MAX_RECORDED_DABS: usize = 16_384;
const WORLD_SAMPLE_WEIGHT: usize = 8;
const MAX_VIEW_SIDE: u32 = 4096;
const MAX_HIDDEN_MATERIALS: usize = 256;
/// Cap work amplified by tiny spacing or very large stamp footprints when resizing from a file.
const MAX_REPLAY_DABS: u64 = 1 << 20;
const MAX_REPLAY_TEXEL_VISITS: u64 = 1 << 28;
const MAX_REPLAY_GEOMETRY_VISITS: u64 = 1 << 23;

type UvSample = [f32; 3];

pub(crate) fn record_sample(placement: &mut Option<PaintPlacement>, uv: Vec2, pressure: f32) {
    let Some(PaintPlacement::Uv { samples, .. }) = placement.as_mut() else { return };
    let sample = [uv.x, uv.y, pressure];
    if samples.len() < MAX_SAMPLES_PER_STROKE && valid_sample(sample) {
        samples.push(sample);
    } else {
        *placement = None;
    }
}

/// Freeze the inputs the rasterizer received, including the chosen angle and per-dab seed.
/// Keep pointer samples as a fallback for older readers and strokes over the dab limit.
pub(crate) fn record_uv_dabs(placement: &mut Option<PaintPlacement>, batch: &[UvStamp]) {
    let Some(PaintPlacement::Uv { dabs, .. }) = placement.as_mut() else { return };
    let Some(recorded) = dabs else { return };
    if batch.len() > MAX_RECORDED_DABS.saturating_sub(recorded.len()) {
        *dabs = None;
        return;
    }
    recorded.extend(batch.iter().map(|dab| RecordedUvDab([dab.center.x, dab.center.y, dab.radius, dab.angle, dab.flow, dab.hardness], dab.seed)));
}

fn valid_sample([u, v, pressure]: UvSample) -> bool {
    (-8.0..=9.0).contains(&u) && (-8.0..=9.0).contains(&v) && (0.0..=1.0).contains(&pressure)
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RecordedUvDab([f32; 6], u32);

impl RecordedUvDab {
    fn stamp(&self) -> UvStamp {
        let [u, v, radius, angle, flow, hardness] = self.0;
        UvStamp { center: Vec2::new(u, v), radius, angle, flow, hardness, seed: self.1, source: None }
    }

    fn valid(&self) -> bool {
        let [u, v, radius, angle, flow, hardness] = self.0;
        valid_sample([u, v, 1.0]) && angle.is_finite() && [radius, flow, hardness].into_iter().all(|value| (0.0..=1.0).contains(&value))
    }
}

/// The first archive version stored UV samples directly on the operation.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum PaintPlacement {
    Uv {
        samples: Vec<UvSample>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dabs: Option<Vec<RecordedUvDab>>,
    },
    World {
        world: WorldPaint,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct WorldPaint {
    pub(crate) view: StrokeView,
    pub(crate) dabs: Vec<Stamp>,
    pub(crate) material: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RecordedPaint {
    pub(crate) settings: BrushSettings,
    #[serde(flatten)]
    pub(crate) placement: PaintPlacement,
    pub(crate) values: BTreeMap<Channel, [f32; 3]>,
    pub(crate) mask_value: f32,
    pub(crate) erase: bool,
}

#[derive(Clone, Debug)]
struct RecordedTarget {
    operations: Arc<Vec<Arc<RecordedPaint>>>,
    samples: usize,
    world_material: Option<usize>,
    /// The complete raster result. A write by another tool breaks this identity even if that
    /// tool doesn't know about the journal.
    output: Vec<(Option<Channel>, TileIdentity)>,
}

#[derive(Clone, Debug, Default)]
struct Journal {
    targets: BTreeMap<PaintTarget, RecordedTarget>,
    strokes: usize,
    samples: usize,
    world_mesh: Option<RecordedMesh>,
}

/// Journal identity is cheap while painting. A fingerprint is needed only for the archive.
#[derive(Clone, Debug)]
struct RecordedMesh {
    mesh: Weak<Mesh>,
    fingerprint: Arc<OnceLock<u64>>,
}

impl RecordedMesh {
    fn new(mesh: &Arc<Mesh>) -> Self {
        Self { mesh: Arc::downgrade(mesh), fingerprint: Arc::default() }
    }

    fn matches(&self, mesh: &Arc<Mesh>) -> bool {
        std::ptr::eq(self.mesh.as_ptr(), Arc::as_ptr(mesh))
    }

    fn fingerprint(&self, mesh: &Mesh) -> u64 {
        *self.fingerprint.get_or_init(|| mesh_fingerprint(mesh))
    }
}

impl PaintPlacement {
    fn world_material(&self) -> Option<usize> {
        match self {
            Self::World { world } => Some(world.material),
            Self::Uv { .. } => None,
        }
    }

    /// Weight more expensive world-space coordinates in the archive's aggregate sample limit.
    fn sample_weight(&self) -> usize {
        match self {
            Self::Uv { samples, dabs } => samples.len().saturating_add(dabs.as_ref().map_or(0, Vec::len)),
            Self::World { world } => world.dabs.len().saturating_mul(WORLD_SAMPLE_WEIGHT).saturating_add(world.view.hidden_count()),
        }
    }

    fn valid(&self) -> bool {
        match self {
            Self::Uv { samples, dabs } => {
                !samples.is_empty()
                    && samples.len() <= MAX_SAMPLES_PER_STROKE
                    && samples.iter().copied().all(valid_sample)
                    && dabs.as_ref().is_none_or(|dabs| !dabs.is_empty() && dabs.len() <= MAX_RECORDED_DABS && dabs.iter().all(RecordedUvDab::valid))
            }
            Self::World { world } => world.valid(),
        }
    }
}

impl WorldPaint {
    fn valid(&self) -> bool {
        let (width, height) = self.view.size;
        let camera = &self.view.camera;
        (64..=MAX_VIEW_SIDE).contains(&width)
            && (64..=MAX_VIEW_SIDE).contains(&height)
            && camera.eye().is_finite()
            && camera.view_proj(width as f32 / height.max(1) as f32).is_finite()
            && self.view.hidden_count() <= MAX_HIDDEN_MATERIALS
            && !self.dabs.is_empty()
            && self.dabs.len() <= MAX_RECORDED_DABS
            && self.dabs.iter().all(|dab| {
                dab.source.is_none()
                    && dab.center.is_finite()
                    && dab.normal.is_finite()
                    && dab.tangent.is_finite()
                    && dab.bitangent.is_finite()
                    && dab.radius.is_finite()
                    && dab.radius >= 0.0
                    && (0.0..=1.0).contains(&dab.flow)
                    && (0.0..=1.0).contains(&dab.hardness)
                    && (0.05..=4.0).contains(&dab.depth)
                    && dab.world_extent().is_finite()
            })
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct StrokeReplays(Arc<Journal>);

fn images(target: PaintTarget, pixels: &PixelStore) -> impl Iterator<Item = (Option<Channel>, &TileImage)> {
    let (layer, mask) = match target {
        PaintTarget::Layer(id) => (Some(id), None),
        PaintTarget::Mask(id) => (None, pixels.masks.get(&id)),
    };
    Channel::ALL.into_iter().filter_map(move |channel| Some((Some(channel), pixels.layer(layer?, channel)?))).chain(mask.map(|image| (None, image)))
}

fn output_of(target: PaintTarget, pixels: &PixelStore) -> Vec<(Option<Channel>, TileIdentity)> {
    images(target, pixels).map(|(channel, img)| (channel, img.identity())).collect()
}

impl RecordedTarget {
    fn describes(&self, target: PaintTarget, pixels: &PixelStore) -> bool {
        let mut output = self.output.iter();
        images(target, pixels).all(|(channel, img)| output.next().is_some_and(|(expected, identity)| *expected == channel && img.matches_identity(identity)))
            && output.next().is_none()
    }
}

fn set_of(project: &Project, target: PaintTarget) -> Option<usize> {
    let owner = match target {
        PaintTarget::Layer(id) if project.layer(id).is_ok_and(|layer| matches!(layer.kind, LayerKind::Paint)) => id,
        PaintTarget::Layer(id) => match project.effect(id) {
            Some((owner, effect)) if matches!(effect.kind, EffectKind::Paint) && project.layer(owner).ok()?.content_effects.iter().any(|e| e.id == id) => owner,
            _ => return None,
        },
        PaintTarget::Mask(id) => {
            let (owner, effect) = project.effect(id)?;
            if !matches!(effect.kind, EffectKind::Paint) || !project.layer(owner).ok()?.mask.as_ref()?.effects.iter().any(|e| e.id == id) {
                return None;
            }
            owner
        }
    };
    project.locate(owner).map(|(ts, _, _)| ts)
}

fn ordinary_set(project: &Project, target: PaintTarget) -> Option<(usize, usize)> {
    let ts = set_of(project, target)?;
    let set = project.texture_sets.get(ts)?;
    set.uv_tiles.is_empty().then_some((ts, set.material))
}

impl StrokeReplays {
    pub(crate) fn can_record(project: &Project, target: PaintTarget, ts: usize) -> bool {
        ordinary_set(project, target).is_some_and(|(set, _)| set == ts)
    }

    /// Whether this commit invalidated an existing replay history for its target.
    pub(crate) fn commit(
        &mut self,
        target: PaintTarget,
        mut operation: Option<RecordedPaint>,
        before: &PixelStore,
        after: &PixelStore,
        mesh: &Arc<Mesh>,
        material: usize,
    ) -> bool {
        let prior = self.0.targets.get(&target);
        if let Some(operation) = &mut operation
            && self.0.samples.saturating_add(operation.placement.sample_weight()) > MAX_SAMPLES
            && let PaintPlacement::Uv { dabs, .. } = &mut operation.placement
        {
            *dabs = None;
        }
        let operation = operation.filter(|op| {
            self.0.strokes < MAX_STROKES
                && self.0.samples.saturating_add(op.placement.sample_weight()) <= MAX_SAMPLES
                && op.valid()
                && prior.map_or_else(
                    || images(target, before).all(|(_, img)| img.allocated_count() == 0),
                    |prior| prior.describes(target, before) && self.compatible_world(prior, mesh, material),
                )
                && op
                    .placement
                    .world_material()
                    .is_none_or(|recorded_material| recorded_material == material && self.0.world_mesh.as_ref().is_none_or(|recorded| recorded.matches(mesh)))
        });
        match operation {
            Some(operation) => {
                let journal = Arc::make_mut(&mut self.0);
                let weight = operation.placement.sample_weight();
                let world_material = operation.placement.world_material();
                if world_material.is_some() {
                    journal.world_mesh.get_or_insert_with(|| RecordedMesh::new(mesh));
                }
                let record = journal.targets.entry(target).or_insert_with(|| RecordedTarget {
                    operations: Arc::default(),
                    samples: 0,
                    world_material: None,
                    output: Vec::new(),
                });
                record.samples += weight;
                record.world_material = world_material.or(record.world_material);
                journal.samples += weight;
                journal.strokes += 1;
                Arc::make_mut(&mut record.operations).push(Arc::new(operation));
                record.output = output_of(target, after);
                false
            }
            None if prior.is_some() => {
                self.forget(target);
                true
            }
            None => false,
        }
    }

    fn forget(&mut self, target: PaintTarget) {
        let journal = Arc::make_mut(&mut self.0);
        if let Some(prior) = journal.targets.remove(&target) {
            journal.strokes -= prior.operations.len();
            journal.samples -= prior.samples;
            if prior.world_material.is_some() && !journal.targets.values().any(|target| target.world_material.is_some()) {
                journal.world_mesh = None;
            }
        }
    }

    /// Forget targets edited by other tools without losing any raster paint.
    pub(crate) fn discard_stale(&mut self, project: &Project, pixels: &PixelStore, mesh: &Arc<Mesh>) -> bool {
        let mut discarded = false;
        let stale: Vec<_> = self
            .0
            .targets
            .iter()
            .filter_map(|(&target, record)| {
                if ordinary_set(project, target).is_some_and(|(_, material)| record.describes(target, pixels) && self.compatible_world(record, mesh, material))
                {
                    return None;
                }
                Some((target, set_of(project, target).is_some()))
            })
            .collect();
        for (target, raster_remains) in stale {
            discarded |= raster_remains;
            self.forget(target);
        }
        discarded
    }

    fn compatible_world(&self, target: &RecordedTarget, mesh: &Arc<Mesh>, material: usize) -> bool {
        target.world_material.is_none_or(|expected| expected == material && self.0.world_mesh.as_ref().is_some_and(|recorded| recorded.matches(mesh)))
    }

    pub(crate) fn prepare_resize(
        &self,
        project: &Project,
        pixels: &PixelStore,
        ts: usize,
        size: (u32, u32),
        scene: &ReplayScene<'_>,
        progress: &Progress,
    ) -> ResizeReplays {
        let mut prepared = ResizeReplays::default();
        let mut budget = ReplayBudget {
            dabs: MAX_REPLAY_DABS,
            texel_visits: MAX_REPLAY_TEXEL_VISITS,
            geometry_visits: MAX_REPLAY_GEOMETRY_VISITS,
            view: None,
            cancel: &progress.cancel,
        };
        for (&target, record) in &self.0.targets {
            if progress.cancel.load(Relaxed) {
                break;
            }
            if !Self::can_record(project, target, ts) || !record.describes(target, pixels) || !self.compatible_world(record, scene.mesh, scene.material) {
                continue;
            }
            let mut remaining = budget;
            if record.operations.iter().all(|operation| remaining.reserve(operation, size, scene)) {
                budget = remaining;
                prepared.replay.push((target, record.operations.clone()));
            } else {
                prepared.deferred.push(target);
            }
        }
        prepared
    }

    fn finish_replay(&mut self, target: PaintTarget, pixels: &PixelStore) {
        if let Some(record) = Arc::make_mut(&mut self.0).targets.get_mut(&target) {
            record.output = output_of(target, pixels);
        }
    }
}

pub(crate) struct ReplayScene<'a> {
    pub(crate) mesh: &'a Arc<Mesh>,
    pub(crate) bvh: &'a Arc<Bvh>,
    pub(crate) material: usize,
}

struct ReplayVisibility {
    view: StrokeView,
    bvh: Arc<Bvh>,
    depth: EyeDepth,
}

#[derive(Default)]
pub(crate) struct ResizeReplays {
    replay: Vec<(PaintTarget, Arc<Vec<Arc<RecordedPaint>>>)>,
    deferred: Vec<PaintTarget>,
}

impl ResizeReplays {
    pub(crate) fn is_empty(&self) -> bool {
        self.replay.is_empty() && self.deferred.is_empty()
    }

    pub(crate) fn deferred_count(&self) -> usize {
        self.deferred.len()
    }

    pub(crate) fn remove_rasters(&self, pixels: &mut PixelStore) {
        for (target, _) in &self.replay {
            match *target {
                PaintTarget::Layer(id) => {
                    for channel in Channel::ALL {
                        pixels.layers.remove(&(id, channel));
                    }
                }
                PaintTarget::Mask(id) => {
                    pixels.masks.remove(&id);
                }
            }
        }
    }

    pub(crate) fn run(
        self,
        pixels: &mut PixelStore,
        replays: &mut StrokeReplays,
        gbuf: &UvGBuffer,
        scene: &ReplayScene<'_>,
        progress: &Progress,
    ) -> Result<usize> {
        // Bilinear pixels are an approximation at this size. Retain the original operations
        // so they can still replay at a size that fits the budget.
        for target in self.deferred {
            replays.finish_replay(target, pixels);
        }
        let total_operations: usize = self.replay.iter().map(|(_, operations)| operations.len()).sum();
        let mut completed = 0;
        let mut visibility = None;
        for (target, operations) in self.replay {
            for operation in operations.iter() {
                if progress.cancel.load(Relaxed) {
                    return Err(EngineError::Other("Texture set resize was cancelled".into()));
                }
                operation.replay(target, pixels, gbuf, scene, &mut visibility, progress)?;
                completed += 1;
                progress.done.store(3_000 + ((completed * 6_000) / total_operations) as u32, Relaxed);
            }
            replays.finish_replay(target, pixels);
        }
        Ok(completed)
    }
}

#[derive(Clone, Copy)]
struct ReplayBudget<'a> {
    dabs: u64,
    texel_visits: u64,
    geometry_visits: u64,
    view: Option<&'a StrokeView>,
    cancel: &'a AtomicBool,
}

impl<'a> ReplayBudget<'a> {
    fn reserve(&mut self, operation: &'a RecordedPaint, size: (u32, u32), scene: &ReplayScene<'_>) -> bool {
        match &operation.placement {
            PaintPlacement::Uv { dabs: Some(dabs), .. } => self.reserve_uv_dabs(dabs, size),
            PaintPlacement::Uv { samples, .. } => self.reserve_uv(&operation.settings, samples, size),
            PaintPlacement::World { world } => self.reserve_world(world, size, scene),
        }
    }

    fn pay(&mut self, dabs: u64, texels: u64, geometry: u64) -> bool {
        if self.cancel.load(Relaxed) || dabs > self.dabs || texels > self.texel_visits || geometry > self.geometry_visits {
            return false;
        }
        self.dabs -= dabs;
        self.texel_visits -= texels;
        self.geometry_visits -= geometry;
        true
    }

    fn reserve_uv(&mut self, settings: &BrushSettings, samples: &[UvSample], (width, height): (u32, u32)) -> bool {
        let size = Vec2::new(width as f32, height as f32);
        let mut last = None;
        for &[u, v, pressure] in samples {
            let uv = Vec2::new(u, v);
            let radius = settings.uv_stamp_radius(pressure);
            let dabs = dab_upper_bound(settings, last, uv, radius);
            let extent = Vec2::splat(radius * size.x * 1.42);
            let previous = last.unwrap_or(uv) * size;
            let point = uv * size;
            let low = previous.min(point) - extent;
            let high = previous.max(point) + extent;
            let clipped_span = high.min(size) - low.max(Vec2::ZERO);
            let per_dab = if radius > 0.0 && clipped_span.min_element() > 0.0 {
                // The rasterizer rounds the low and high edges independently.
                let footprint = ((extent * 2.0).min(clipped_span) + Vec2::splat(2.0)).min(size).ceil();
                footprint.x as u64 * footprint.y as u64
            } else {
                0
            };
            let texels = dabs.saturating_mul(per_dab);
            if !self.pay(dabs, texels, 0) {
                return false;
            }
            last = Some(uv);
        }
        true
    }

    fn reserve_uv_dabs(&mut self, dabs: &[RecordedUvDab], (width, height): (u32, u32)) -> bool {
        let size = Vec2::new(width as f32, height as f32);
        dabs.iter().all(|dab| {
            let stamp = dab.stamp();
            let center = stamp.center * size;
            let radius = stamp.radius * size.x * 1.42;
            let low = (center - radius).floor().clamp(Vec2::ZERO, size);
            let high = (center + radius).ceil().clamp(Vec2::ZERO, size);
            let span = (high - low).as_uvec2();
            self.pay(1, u64::from(span.x) * u64::from(span.y), 0)
        })
    }

    fn reserve_world(&mut self, world: &'a WorldPaint, size: (u32, u32), scene: &ReplayScene<'_>) -> bool {
        if !self.pay(world.dabs.len() as u64, 0, 0) || !self.reserve_view(&world.view, scene.mesh) {
            return false;
        }
        world.dabs.iter().filter(|dab| dab.radius > 0.0).all(|dab| {
            let extent = dab.world_extent();
            scene.bvh.query_box_until(dab.center - extent, dab.center + extent, |triangle| {
                // The live rasterizer also checks material ownership. Count all candidates to
                // bound this scan before building a filtered scene.
                self.pay(0, triangle_scan_texels(scene.mesh, triangle, size), 1 + scene.mesh.submeshes.len() as u64)
            })
        })
    }

    fn reserve_view(&mut self, view: &'a StrokeView, mesh: &Mesh) -> bool {
        if self.view == Some(view) {
            return true;
        }
        let (width, height) = view.size;
        let base_cost = mesh.vertex_count().saturating_add(mesh.triangle_count()) as u64;
        if !self.pay(0, u64::from(width) * u64::from(height), base_cost) {
            return false;
        }
        // Count screen-space triangle bounds as an upper bound on camera-depth rasterization.
        let size = Vec2::new(width as f32, height as f32);
        let view_proj = view.camera.view_proj(size.x / size.y);
        let screen: Vec<_> = mesh.positions.iter().map(|point| screen_point(view_proj, *point, size)).collect();
        for triangle in 0..mesh.triangle_count() {
            let Some([a, b, c]) = mesh.tri(triangle) else { continue };
            let (Some(Some(a)), Some(Some(b)), Some(Some(c))) = (screen.get(a), screen.get(b), screen.get(c)) else { continue };
            let low = a.min(*b).min(*c).floor().max(Vec2::ZERO);
            let high = a.max(*b).max(*c).ceil().min(size);
            let extent = (high - low).max(Vec2::ZERO).as_uvec2();
            if !self.pay(0, u64::from(extent.x) * u64::from(extent.y), 0) {
                return false;
            }
        }
        self.view = Some(view);
        true
    }
}

impl RecordedPaint {
    fn valid(&self) -> bool {
        self.placement.valid()
            && self.settings.alpha_image.is_none()
            && !self.settings.symmetry.any()
            && self.settings == self.settings.sanitized()
            && (0.0..=1.0).contains(&self.mask_value)
            && self.values.values().flatten().all(|component| component.is_finite())
    }

    fn replay(
        &self,
        target: PaintTarget,
        pixels: &mut PixelStore,
        gbuf: &UvGBuffer,
        scene: &ReplayScene<'_>,
        visibility: &mut Option<ReplayVisibility>,
        progress: &Progress,
    ) -> Result<()> {
        let size = (gbuf.width, gbuf.height);
        let mut canvas = StrokeCanvas::new(size.0, size.1).with_stamp_blending(self.settings.stamp_blending);
        let library = self.settings.library_alpha_image();
        let shape = Shape::new(self.settings.alpha, None, library.as_deref(), self.settings.alpha_params);
        match &self.placement {
            PaintPlacement::Uv { dabs: Some(dabs), .. } => {
                for dab in dabs {
                    if progress.cancel.load(Relaxed) {
                        break;
                    }
                    rasterize_uv(&mut canvas, gbuf, shape, &[dab.stamp()]);
                }
            }
            PaintPlacement::Uv { samples, .. } => {
                let mut stroke = UvStrokeState::default();
                for &[u, v, pressure] in samples {
                    if progress.cancel.load(Relaxed) {
                        break;
                    }
                    stroke.sample(&self.settings, Vec2::new(u, v), pressure, |dabs| {
                        for dab in dabs {
                            if progress.cancel.load(Relaxed) {
                                break;
                            }
                            rasterize_uv(&mut canvas, gbuf, shape, std::slice::from_ref(dab));
                        }
                    });
                }
            }
            PaintPlacement::World { world } => {
                if visibility.as_ref().is_none_or(|cached| cached.view != world.view) {
                    let previous = visibility.as_ref().map(|cached| (&cached.view, &cached.bvh));
                    let (bvh, depth) = world.view.visibility(scene.mesh, scene.bvh, previous);
                    *visibility = Some(ReplayVisibility { view: world.view.clone(), bvh, depth });
                }
                let cached = visibility.as_ref().ok_or_else(|| EngineError::Other("World paint visibility could not be recreated".into()))?;
                let context = RasterCtx {
                    mesh: scene.mesh,
                    bvh: &cached.bvh,
                    gbuf,
                    material: scene.material,
                    depth: Some(&cached.depth),
                    eye: world.view.camera.eye(),
                    backface_culling: self.settings.backface_culling,
                    angle_fade: self.settings.angle_fade,
                    shape,
                    facing: texturecraft_paint::Facing::Front,
                    reach: texturecraft_paint::Reach::Plane,
                };
                for dab in &world.dabs {
                    if progress.cancel.load(Relaxed) {
                        break;
                    }
                    rasterize(&mut canvas, &context, std::slice::from_ref(dab));
                }
            }
        }
        if progress.cancel.load(Relaxed) {
            return Err(EngineError::Other("Texture set resize was cancelled".into()));
        }
        if !canvas.dirty.is_empty() {
            let mut staging = Staging::default();
            let source = CommitSource {
                alpha: &canvas.alpha,
                tiles: &canvas.dirty,
                opacity: self.settings.opacity,
                erase: self.erase,
                per_texel: None,
                carried: None,
                carried_mask: None,
            };
            match target {
                PaintTarget::Layer(id) => commit_layer(pixels, id, size, &self.values, source, &mut staging),
                PaintTarget::Mask(id) => commit_mask(pixels, id, size, self.mask_value, source, &mut staging),
            }
        }
        Ok(())
    }
}

/// The paint target as an explicit archive key.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum TargetId {
    Layer(LayerId),
    Mask(LayerId),
}

impl From<TargetId> for PaintTarget {
    fn from(target: TargetId) -> Self {
        match target {
            TargetId::Layer(id) => Self::Layer(id),
            TargetId::Mask(id) => Self::Mask(id),
        }
    }
}

impl From<PaintTarget> for TargetId {
    fn from(target: PaintTarget) -> Self {
        match target {
            PaintTarget::Layer(id) => Self::Layer(id),
            PaintTarget::Mask(id) => Self::Mask(id),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ReplayArchive<Operation> {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    world_mesh: Option<u64>,
    targets: Vec<ArchivedTarget<Operation>>,
}

#[derive(Serialize, Deserialize)]
struct ArchivedTarget<Operation> {
    target: TargetId,
    raster: Vec<(Option<Channel>, u64)>,
    operations: Vec<Operation>,
}

/// Identifies the fallback raster on disk. If another reader edits raster pixels while
/// preserving unknown zip entries, its newer raster must win.
fn fingerprint(image: &TileImage) -> u64 {
    let mut hash = Fingerprint::new();
    hash.add(u64::from(image.width()));
    hash.add(u64::from(image.height()));
    hash.add(image.comps() as u64);
    let Ok(()) = image.for_each_tile::<std::convert::Infallible>(|x, y, tile| {
        hash.add(u64::from(x));
        hash.add(u64::from(y));
        for &value in tile {
            hash.add(u64::from(value));
        }
        Ok(())
    });
    hash.0
}

struct Fingerprint(u64);

impl Fingerprint {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn add(&mut self, word: u64) {
        self.0 = (self.0 ^ word).wrapping_mul(0x100_0000_01b3);
    }

    fn floats(&mut self, values: impl IntoIterator<Item = f32>) {
        for value in values {
            self.add(u64::from(value.to_bits()));
        }
    }
}

/// Position, shading normal, UV and material ownership determine same-mesh world replay.
fn mesh_fingerprint(mesh: &Mesh) -> u64 {
    let mut hash = Fingerprint::new();
    for count in [mesh.positions.len(), mesh.normals.len(), mesh.uvs.len(), mesh.indices.len(), mesh.submeshes.len()] {
        hash.add(count as u64);
    }
    hash.floats(mesh.positions.iter().flat_map(|vertex| vertex.to_array()));
    hash.floats(mesh.normals.iter().flat_map(|normal| normal.to_array()));
    hash.floats(mesh.uvs.iter().flat_map(|uv| uv.to_array()));
    for &index in &mesh.indices {
        hash.add(u64::from(index));
    }
    for range in &mesh.submeshes {
        hash.add(u64::from(range.start));
        hash.add(u64::from(range.count));
        hash.add(range.material as u64);
    }
    hash.0
}

impl StrokeReplays {
    pub(crate) fn archive(&self, project: &Project, pixels: &PixelStore, mesh: &Arc<Mesh>) -> Option<StrokeReplayData> {
        let targets: Vec<_> = self
            .0
            .targets
            .iter()
            .filter(|(target, record)| {
                ordinary_set(project, **target).is_some_and(|(_, material)| record.describes(**target, pixels) && self.compatible_world(record, mesh, material))
            })
            .map(|(&target, record)| {
                let checkpoint: Vec<_> = images(target, pixels).map(|(channel, image)| (channel, image.clone())).collect();
                (target, record.operations.clone(), checkpoint, record.world_material.is_some())
            })
            .collect();
        (!targets.is_empty()).then(|| {
            let world_mesh =
                targets.iter().any(|(_, _, _, has_world)| *has_world).then(|| self.0.world_mesh.as_ref().map(|state| (state.clone(), mesh.clone()))).flatten();
            StrokeReplayData::snapshot(move || {
                let world_mesh = world_mesh.as_ref().map(|(state, mesh)| state.fingerprint(mesh));
                let targets = targets
                    .iter()
                    .map(|(target, operations, checkpoint, _)| ArchivedTarget {
                        target: (*target).into(),
                        raster: checkpoint.iter().map(|(channel, image)| (*channel, fingerprint(image))).collect(),
                        operations: operations.iter().map(Arc::as_ref).collect(),
                    })
                    .collect();
                serde_json::to_vec(&ReplayArchive { version: ARCHIVE_VERSION, world_mesh, targets }).map_err(|error| FormatError::Invalid(error.to_string()))
            })
        })
    }

    pub(crate) fn load(data: &StrokeReplayData, project: &Project, pixels: &PixelStore, mesh: &Arc<Mesh>) -> (Self, bool) {
        let archive = data.bytes().ok().and_then(|bytes| serde_json::from_slice::<ReplayArchive<RecordedPaint>>(&bytes).ok());
        let Some(archive) = archive.filter(|archive| (1..=ARCHIVE_VERSION).contains(&archive.version) && archive.targets.len() <= MAX_STROKES) else {
            return (Self::default(), true);
        };
        let state = RecordedMesh::new(mesh);
        let matching_mesh = archive.world_mesh.is_some_and(|fingerprint| fingerprint == state.fingerprint(mesh));
        let mut journal = Journal::default();
        let mut ignored = false;
        for record in archive.targets {
            let target = record.target.into();
            let strokes = record.operations.len();
            let samples = record.operations.iter().map(|operation| operation.placement.sample_weight()).sum::<usize>();
            let world_material = record.operations.iter().find_map(|operation| operation.placement.world_material());
            let acceptable = strokes != 0
                && strokes <= MAX_STROKES - journal.strokes
                && samples <= MAX_SAMPLES - journal.samples
                && record.operations.iter().all(RecordedPaint::valid)
                && !journal.targets.contains_key(&target)
                && (world_material.is_none() || matching_mesh)
                && ordinary_set(project, target)
                    .is_some_and(|(_, expected)| record.operations.iter().all(|op| op.placement.world_material().is_none_or(|material| material == expected)));
            if !acceptable || images(target, pixels).map(|(channel, image)| (channel, fingerprint(image))).ne(record.raster) {
                ignored = true;
                continue;
            }
            journal.strokes += strokes;
            journal.samples += samples;
            if world_material.is_some() {
                journal.world_mesh.get_or_insert_with(|| state.clone());
            }
            journal.targets.insert(
                target,
                RecordedTarget {
                    operations: Arc::new(record.operations.into_iter().map(Arc::new).collect()),
                    samples,
                    world_material,
                    output: output_of(target, pixels),
                },
            );
        }
        (Self(Arc::new(journal)), ignored)
    }
}
