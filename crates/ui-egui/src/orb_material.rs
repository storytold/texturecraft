//! What a material orb shows: every channel the orb lights with, read from the fill's own sources through its own projection

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Weak};

use texturecraft_engine::compose::ImageRes;
use texturecraft_engine::compose::pattern::{Coords, image_samples, procedural};
use texturecraft_engine::core::blend::lum;
use texturecraft_engine::core::{Channel, ChannelKind, Fill, NormalFormat, Projection, Sampling, Source};
use texturecraft_engine::geom::glam::{Vec2, Vec3};
use texturecraft_engine::texel::srgb_to_linear;

mod hash_writer;
mod orb_sample;

use hash_writer::HashWriter;
pub(crate) use orb_sample::OrbSample;

/// The channels the orb reads; the others do not change how it looks
pub(crate) const ORB_CHANNELS: [Channel; 7] =
    [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Normal, Channel::Height, Channel::Emissive, Channel::Opacity];

/// Fewest and most image repeats the orb's filtering assumes, so a tiny or huge projection scale neither blurs nor aliases without bound
const MIN_IMAGE_SCALE: f32 = 0.01;
const MAX_IMAGE_SCALE: f32 = 64.0;

/// A material as the orb shows it
#[derive(Clone, Debug)]
pub(crate) struct OrbMaterial {
    sources: BTreeMap<Channel, Source>,
    projection: Projection,
    sampling: Sampling,
    /// The images the sources read, shared with the session
    images: BTreeMap<String, Arc<ImageRes>>,
    /// The one channel shown on its own, unlit, as the viewport's channel view draws it; `None` = the whole material, lit
    alone: Option<Channel>,
}

impl OrbMaterial {
    /// One value per channel: a plain material's base colour, metallic and roughness
    #[cfg(test)]
    pub(crate) fn uniform(base: [f32; 3], metallic: f32, roughness: f32) -> OrbMaterial {
        let sources = BTreeMap::from([
            (Channel::BaseColor, Source::Uniform(base)),
            (Channel::Metallic, Source::Uniform([metallic; 3])),
            (Channel::Roughness, Source::Uniform([roughness; 3])),
        ]);
        OrbMaterial { sources, projection: Projection::default(), sampling: Sampling::default(), images: BTreeMap::new(), alone: None }
    }

    /// `fill` with the channels in `on`; channels that are off show their defaults, images come from `images`
    pub(crate) fn of_fill(fill: &Fill, on: &[Channel], images: &std::collections::HashMap<String, Arc<ImageRes>>) -> OrbMaterial {
        OrbMaterial::with_channels(fill, ORB_CHANNELS.into_iter().filter(|channel| on.contains(channel)), images, None)
    }

    /// `fill`'s `channel` alone (its default when the fill does not write it, `on` being the channels it writes)
    pub(crate) fn of_channel(fill: &Fill, channel: Channel, on: &[Channel], images: &std::collections::HashMap<String, Arc<ImageRes>>) -> OrbMaterial {
        OrbMaterial::with_channels(fill, on.contains(&channel).then_some(channel), images, Some(channel))
    }

    fn with_channels(
        fill: &Fill,
        channels: impl IntoIterator<Item = Channel>,
        images: &std::collections::HashMap<String, Arc<ImageRes>>,
        alone: Option<Channel>,
    ) -> OrbMaterial {
        let sources: BTreeMap<Channel, Source> = channels.into_iter().map(|channel| (channel, fill.source(channel))).collect();
        let images = sources
            .values()
            .filter_map(|source| match source {
                Source::Image { name, .. } => images.get(name).map(|image| (name.clone(), image.clone())),
                _ => None,
            })
            .collect();
        OrbMaterial { sources, projection: fill.projection.clone(), sampling: fill.sampling, images, alone }
    }

    /// The channel the orb shows on its own, if any
    pub(crate) fn alone(&self) -> Option<Channel> {
        self.alone
    }

    /// The images it reads, held weakly: while a cached picture holds them, a new image can't take a freed one's address, which [`OrbMaterial::key`] hashes
    pub(crate) fn image_handles(&self) -> Vec<Weak<ImageRes>> {
        self.images.values().map(Arc::downgrade).collect()
    }

    /// A key that changes whenever the orb would look different
    pub(crate) fn key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();

        // Writing to a hasher cannot fail
        let _ = write!(HashWriter(&mut hasher), "{:?}{:?}{:?}{:?}", self.sources, self.projection, self.sampling, self.alone);
        for (name, image) in &self.images {
            // The same name re-imported is a new image
            (name, Arc::as_ptr(image) as usize, image.width, image.height).hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Whether any channel varies over the surface (a uniform material needs no per-pixel reads)
    pub(crate) fn varies(&self) -> bool {
        self.sources.values().any(|source| !matches!(source, Source::Uniform(_)))
    }

    /// The material at `coords`; `footprint` = how far one orb pixel reaches in UVs
    pub(crate) fn sample(&self, coords: &Coords, footprint: f32) -> OrbSample {
        let value = |channel: Channel| self.value(channel, coords, footprint);
        let [metallic, roughness, opacity] = [Channel::Metallic, Channel::Roughness, Channel::Opacity].map(|channel| value(channel)[0]);
        OrbSample { base: value(Channel::BaseColor), metallic, roughness, normal: value(Channel::Normal), emissive: value(Channel::Emissive), opacity }
    }

    /// The height at `coords` (0 without a height channel)
    pub(crate) fn height(&self, coords: &Coords, footprint: f32) -> f32 {
        self.value(Channel::Height, coords, footprint)[0]
    }

    /// Whether the height channel varies (only then does it bump the surface)
    pub(crate) fn has_relief(&self) -> bool {
        self.sources.get(&Channel::Height).is_some_and(|source| !matches!(source, Source::Uniform(_)))
    }

    /// `channel`'s value at `coords` (linear; normals 0..1 per component, height -1..1)
    pub(crate) fn value(&self, channel: Channel, coords: &Coords, footprint: f32) -> [f32; 3] {
        let Some(source) = self.sources.get(&channel) else { return channel.default_value() };
        match source {
            Source::Uniform(value) => *value,
            Source::Procedural { pattern, low, high, ramp } => {
                let amount = procedural(pattern, &self.projection, coords);

                // A colour channel's gradient map colours the pattern as the layer stack does
                if channel.kind() == ChannelKind::Color && !ramp.is_empty() {
                    texturecraft_engine::core::gradient(ramp, amount).unwrap_or_default()
                } else {
                    [0, 1, 2].map(|k| low[k] + (high[k] - low[k]) * amount)
                }
            }
            Source::Image { name, normal_format } => {
                let Some(image) = self.images.get(name) else { return channel.default_value() };
                self.image_value(image, channel.kind(), *normal_format, coords, footprint)
            }

            // An anchor reads another layer's result, which only the layer stack knows
            Source::Anchor { .. } => channel.default_value(),
        }
    }

    fn image_value(&self, image: &ImageRes, kind: ChannelKind, normal_format: NormalFormat, coords: &Coords, footprint: f32) -> [f32; 3] {
        let mut sum = [0.0f32; 3];
        for (uv, weight) in image_samples(&self.projection, coords).into_iter().filter(|(_, weight)| *weight > 0.0) {
            let texel = image.sample_filtered(uv.x, uv.y, self.sampling, footprint * image.width as f32 * image_scale(&self.projection));
            for (total, value) in sum.iter_mut().zip(texel) {
                *total += value * weight;
            }
        }
        match kind {
            ChannelKind::Color if image.srgb => sum.map(srgb_to_linear),
            // Normal channels are OpenGL (+Y up the image): a DirectX image's green flips
            ChannelKind::NormalMap if normal_format == NormalFormat::DirectX => [sum[0], 1.0 - sum[1], sum[2]],
            ChannelKind::Color | ChannelKind::NormalMap => sum,
            ChannelKind::Scalar => [lum(sum); 3],
            ChannelKind::Signed => [lum(sum) * 2.0 - 1.0; 3],
        }
    }
}

/// How many image repeats the projection lays across the orb's width, roughly (sharper filtering for fewer)
fn image_scale(projection: &Projection) -> f32 {
    let scale = match projection {
        Projection::Triplanar { scale, .. } | Projection::Planar { scale, .. } => *scale,
        Projection::Spherical { scale, .. } | Projection::Cylindrical { scale, .. } => scale.max(1.0),
        Projection::Uv { scale, .. } => scale[0].abs().max(scale[1].abs()),
        Projection::UvTile | Projection::Warp { .. } => 1.0,
    };
    if scale.is_finite() { scale.abs().clamp(MIN_IMAGE_SCALE, MAX_IMAGE_SCALE) } else { 1.0 }
}

/// Where a point of the unit orb lies for a fill's projection: its position and normal (both the unit direction `direction`), and UVs wrapping once around the equator (two across) and once from pole to pole
pub(crate) fn orb_coords(direction: Vec3) -> Coords {
    let u = direction.z.atan2(direction.x) / std::f32::consts::TAU + 0.5;
    let v = direction.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    Coords { uv: Vec2::new(u * 2.0, v), pos: direction, nrm: direction }
}
