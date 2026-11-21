//! Where a stroke in the 2D view is between pointer samples.

use texturecraft_geom::glam::Vec2;
use texturecraft_paint::{BrushSettings, UvStamp};

/// Shortest spacing, including stamps with zero pressure and radius.
const MIN_UV_STAMP_STEP: f32 = 1e-5;

/// Bound working memory during sampling, rasterization and GPU readback recovery.
pub(crate) const UV_STAMP_BATCH: usize = 4096;

/// Cover a UV diagonal at the minimum step without letting hostile coordinates stall painting.
const MAX_UV_STAMPS_PER_SAMPLE: usize = 150_000;

/// A 2D view stroke's progress: its last point, the distance carried to the next stamp and the stamps so far.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct UvStrokeState {
    /// The last sample's texture coordinates (None before the first sample).
    pub(crate) last: Option<Vec2>,
    /// Distance from the last stamp to the last sample, at most one stamp step.
    carry: f32,
    /// Stamps so far: the next stamp's seed, so noise alphas don't repeat from sample to sample.
    stamps: u32,
}

impl UvStrokeState {
    /// Turn one pressure sample into resolution-independent dabs. The consumer may add mirrored
    /// copies to each batch. Those copies do not affect spacing or the seed of the next dab.
    pub(crate) fn sample(&mut self, settings: &BrushSettings, uv: Vec2, pressure: f32, mut consume: impl FnMut(&mut Vec<UvStamp>)) {
        let radius = settings.uv_stamp_radius(pressure);
        let step = (settings.spacing * 2.0 * radius).max(MIN_UV_STAMP_STEP);
        let angle = match self.last {
            Some(previous) if settings.follow_path && uv != previous => settings.angle + (previous.y - uv.y).atan2(uv.x - previous.x).to_degrees() - 90.0,
            _ => settings.angle,
        };
        let (start, segment, mut distance) = match self.last.replace(uv) {
            None => (uv, Vec2::ZERO, 0.0),
            // Pressure can shrink the step below the carry from the preceding sample.
            Some(previous) => (previous, uv - previous, (step - self.carry).max(0.0)),
        };
        let length = segment.length();
        let mut batch = Vec::new();
        let mut stamped = 0;
        while distance <= length && stamped < MAX_UV_STAMPS_PER_SAMPLE {
            let batch_size = UV_STAMP_BATCH.min(MAX_UV_STAMPS_PER_SAMPLE - stamped);
            batch.clear();
            while distance <= length && batch.len() < batch_size {
                let center = start + segment * (distance / length.max(f32::MIN_POSITIVE));
                let seed = self.stamps.wrapping_add((stamped + batch.len()) as u32);
                batch.push(UvStamp { center, radius, angle, flow: settings.flow, hardness: settings.hardness, seed, source: None });
                distance += step;
            }
            stamped += batch.len();
            consume(&mut batch);
        }
        // A segment cut off at the stamp limit cannot send later dabs behind the next sample.
        self.carry = (length - (distance - step)).min(step);
        self.stamps = self.stamps.wrapping_add(stamped as u32);
    }
}
