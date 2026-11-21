//! Ordered 2D paint operations and the raster images they completely describe.

use std::collections::BTreeMap;
use std::sync::{Arc, atomic::Ordering::Relaxed};

use serde::{Deserialize, Serialize};
use texturecraft_bake::Progress;
use texturecraft_compose::{PaintTarget, PixelStore};
use texturecraft_core::{Channel, EffectKind, LayerId, LayerKind, Project};
use texturecraft_formats::{FormatError, StrokeReplayData};
use texturecraft_geom::{UvGBuffer, glam::Vec2};
use texturecraft_paint::{BrushSettings, CommitSource, Shape, Staging, StrokeCanvas, commit_layer, commit_mask, rasterize_uv};
use texturecraft_texel::{TileIdentity, TileImage};

use crate::{EngineError, Result, uv_stroke_state::UvStrokeState};

const MAX_SAMPLES_PER_STROKE: usize = 16_384;
const MAX_STROKES: usize = 4_096;
const MAX_SAMPLES: usize = 262_144;
const ARCHIVE_VERSION: u32 = 1;

pub(crate) type UvSample = [f32; 3];

pub(crate) fn record_sample(samples: &mut Option<Vec<UvSample>>, uv: Vec2, pressure: f32) {
    let Some(recorded) = samples.as_mut() else { return };
    let sample = [uv.x, uv.y, pressure];
    if recorded.len() < MAX_SAMPLES_PER_STROKE && valid_sample(sample) {
        recorded.push(sample);
    } else {
        *samples = None;
    }
}

fn valid_sample([u, v, pressure]: UvSample) -> bool {
    (-8.0..=9.0).contains(&u) && (-8.0..=9.0).contains(&v) && (0.0..=1.0).contains(&pressure)
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct UvPaint {
    pub(crate) settings: BrushSettings,
    pub(crate) samples: Vec<UvSample>,
    pub(crate) values: BTreeMap<Channel, [f32; 3]>,
    pub(crate) mask_value: f32,
    pub(crate) erase: bool,
}

#[derive(Clone, Debug)]
struct RecordedTarget {
    operations: Arc<Vec<Arc<UvPaint>>>,
    samples: usize,
    /// The complete raster result. A write by another tool breaks this identity even if that
    /// tool doesn't know about the journal.
    output: Vec<(Option<Channel>, TileIdentity)>,
}

#[derive(Clone, Debug, Default)]
struct Journal {
    targets: BTreeMap<PaintTarget, RecordedTarget>,
    strokes: usize,
    samples: usize,
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

impl StrokeReplays {
    pub(crate) fn can_record(project: &Project, target: PaintTarget, ts: usize) -> bool {
        project.texture_sets.get(ts).is_some_and(|set| set.uv_tiles.is_empty()) && set_of(project, target) == Some(ts)
    }

    /// Whether this commit invalidated an existing replay history for its target.
    pub(crate) fn commit(&mut self, target: PaintTarget, operation: Option<UvPaint>, before: &PixelStore, after: &PixelStore) -> bool {
        let prior = self.0.targets.get(&target);
        let operation = operation.filter(|op| {
            self.0.strokes < MAX_STROKES
                && self.0.samples.saturating_add(op.samples.len()) <= MAX_SAMPLES
                && op.valid()
                && prior.map_or_else(|| images(target, before).all(|(_, img)| img.allocated_count() == 0), |prior| prior.describes(target, before))
        });
        match operation {
            Some(operation) => {
                let journal = Arc::make_mut(&mut self.0);
                let record = journal.targets.entry(target).or_insert_with(|| RecordedTarget { operations: Arc::default(), samples: 0, output: Vec::new() });
                record.samples += operation.samples.len();
                journal.samples += operation.samples.len();
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
        }
    }

    /// Forget targets edited by other tools without losing any raster paint.
    pub(crate) fn discard_stale(&mut self, project: &Project, pixels: &PixelStore) -> bool {
        let mut discarded = false;
        let stale: Vec<_> = self
            .0
            .targets
            .iter()
            .filter_map(|(&target, record)| match set_of(project, target) {
                Some(ts) if Self::can_record(project, target, ts) && record.describes(target, pixels) => None,
                Some(_) => Some((target, true)),
                None => Some((target, false)),
            })
            .collect();
        for (target, raster_remains) in stale {
            discarded |= raster_remains;
            self.forget(target);
        }
        discarded
    }

    pub(crate) fn prepare_resize(&self, project: &Project, pixels: &PixelStore, ts: usize) -> ResizeReplays {
        let replays = self
            .0
            .targets
            .iter()
            .filter(|(target, record)| record.describes(**target, pixels) && Self::can_record(project, **target, ts))
            .map(|(&target, record)| (target, record.operations.clone()))
            .collect();
        ResizeReplays(replays)
    }

    fn finish_replay(&mut self, target: PaintTarget, pixels: &PixelStore) {
        if let Some(record) = Arc::make_mut(&mut self.0).targets.get_mut(&target) {
            record.output = output_of(target, pixels);
        }
    }
}

#[derive(Default)]
pub(crate) struct ResizeReplays(Vec<(PaintTarget, Arc<Vec<Arc<UvPaint>>>)>);

impl ResizeReplays {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn remove_rasters(&self, pixels: &mut PixelStore) {
        for (target, _) in &self.0 {
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

    pub(crate) fn run(self, pixels: &mut PixelStore, replays: &mut StrokeReplays, gbuf: &UvGBuffer, progress: &Progress) -> Result<usize> {
        let total_operations: usize = self.0.iter().map(|(_, operations)| operations.len()).sum();
        let mut completed = 0;
        for (target, operations) in self.0 {
            for operation in operations.iter() {
                if progress.cancel.load(Relaxed) {
                    return Err(EngineError::Other("Texture set resize was cancelled".into()));
                }
                operation.replay(target, pixels, gbuf, progress)?;
                completed += 1;
                progress.done.store(3_000 + ((completed * 6_000) / total_operations) as u32, Relaxed);
            }
            replays.finish_replay(target, pixels);
        }
        Ok(completed)
    }
}

impl UvPaint {
    fn valid(&self) -> bool {
        !self.samples.is_empty()
            && self.samples.len() <= MAX_SAMPLES_PER_STROKE
            && self.samples.iter().copied().all(valid_sample)
            && self.settings.alpha_image.is_none()
            && !self.settings.symmetry.any()
            && self.settings == self.settings.sanitized()
            && (0.0..=1.0).contains(&self.mask_value)
            && self.values.values().flatten().all(|component| component.is_finite())
    }

    fn replay(&self, target: PaintTarget, pixels: &mut PixelStore, gbuf: &UvGBuffer, progress: &Progress) -> Result<()> {
        let size = (gbuf.width, gbuf.height);
        let mut canvas = StrokeCanvas::new(size.0, size.1).with_stamp_blending(self.settings.stamp_blending);
        let mut stroke = UvStrokeState::default();
        let library = self.settings.library_alpha_image();
        let shape = Shape::new(self.settings.alpha, None, library.as_deref(), self.settings.alpha_params);
        for &[u, v, pressure] in &self.samples {
            if progress.cancel.load(Relaxed) {
                return Err(EngineError::Other("Texture set resize was cancelled".into()));
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
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut add = |word: u64| hash = (hash ^ word).wrapping_mul(0x100_0000_01b3);
    add(u64::from(image.width()));
    add(u64::from(image.height()));
    add(image.comps() as u64);
    for (x, y) in image.allocated() {
        add(u64::from(x));
        add(u64::from(y));
        if let Some(tile) = image.tile(x, y) {
            for &value in tile {
                add(u64::from(value));
            }
        }
    }
    hash
}

impl StrokeReplays {
    pub(crate) fn archive(&self, project: &Project, pixels: &PixelStore) -> Option<StrokeReplayData> {
        let targets: Vec<_> = self
            .0
            .targets
            .iter()
            .filter(|(target, record)| {
                set_of(project, **target).is_some_and(|ts| Self::can_record(project, **target, ts)) && record.describes(**target, pixels)
            })
            .map(|(&target, record)| {
                let checkpoint: Vec<_> = images(target, pixels).map(|(channel, image)| (channel, image.clone())).collect();
                (target, record.operations.clone(), checkpoint)
            })
            .collect();
        (!targets.is_empty()).then(|| {
            StrokeReplayData::snapshot(move || {
                let targets = targets
                    .iter()
                    .map(|(target, operations, checkpoint)| ArchivedTarget {
                        target: (*target).into(),
                        raster: checkpoint.iter().map(|(channel, image)| (*channel, fingerprint(image))).collect(),
                        operations: operations.iter().map(Arc::as_ref).collect(),
                    })
                    .collect();
                serde_json::to_vec(&ReplayArchive { version: ARCHIVE_VERSION, targets }).map_err(|error| FormatError::Invalid(error.to_string()))
            })
        })
    }

    pub(crate) fn load(data: &StrokeReplayData, project: &Project, pixels: &PixelStore) -> (Self, bool) {
        let archive = data.bytes().ok().and_then(|bytes| serde_json::from_slice::<ReplayArchive<UvPaint>>(&bytes).ok());
        let Some(archive) = archive.filter(|archive| archive.version == ARCHIVE_VERSION && archive.targets.len() <= MAX_STROKES) else {
            return (Self::default(), true);
        };
        let mut journal = Journal::default();
        let mut ignored = false;
        for record in archive.targets {
            let target = record.target.into();
            let strokes = record.operations.len();
            let samples = record.operations.iter().map(|operation| operation.samples.len()).sum::<usize>();
            let acceptable = strokes != 0
                && strokes <= MAX_STROKES - journal.strokes
                && samples <= MAX_SAMPLES - journal.samples
                && record.operations.iter().all(UvPaint::valid)
                && !journal.targets.contains_key(&target)
                && set_of(project, target).is_some_and(|ts| Self::can_record(project, target, ts));
            if !acceptable || images(target, pixels).map(|(channel, image)| (channel, fingerprint(image))).ne(record.raster) {
                ignored = true;
                continue;
            }
            journal.strokes += strokes;
            journal.samples += samples;
            journal.targets.insert(
                target,
                RecordedTarget { operations: Arc::new(record.operations.into_iter().map(Arc::new).collect()), samples, output: output_of(target, pixels) },
            );
        }
        (Self(Arc::new(journal)), ignored)
    }
}
