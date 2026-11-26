use std::sync::Arc;

use texturecraft_core::{NormalFormat, Source};

use super::anchor_heights::AnchorHeights;
use super::block_uvs;
use super::slot_key::SlotKey;
use crate::eval::Inputs;
use crate::store::ImageRes;

/// What a micro details slot reads
#[derive(Clone)]
pub(super) enum Slot {
    Uniform([f32; 3]),
    Image {
        image: Arc<ImageRes>,
        flip_green: bool,
    },
    /// An anchor point's height, different in each UV tile block: read from the block at `origin` of a `layout` sized mesh map
    Anchor {
        heights: Arc<AnchorHeights>,
        origin: (u32, u32),
        layout: (u32, u32),
    },
}

impl Slot {
    /// The slot holding `source` (an empty slot, an image not loaded and an anchor not prepared yet read black; procedurals are not read)
    pub(super) fn of(inp: &Inputs, source: Option<&Source>, anchor: Option<Arc<AnchorHeights>>) -> Slot {
        match source {
            Some(Source::Uniform(value)) => Slot::Uniform(*value),
            Some(Source::Image { name, normal_format }) => match inp.images.get(name) {
                Some(image) => Slot::Image { image: image.clone(), flip_green: *normal_format == NormalFormat::DirectX },
                None => Slot::Uniform([0.0; 3]),
            },
            Some(Source::Anchor { .. }) => match anchor {
                Some(heights) => Slot::Anchor { heights, origin: (0, 0), layout: (0, 0) },
                None => Slot::Uniform([0.0; 3]),
            },
            _ => Slot::Uniform([0.0; 3]),
        }
    }

    /// Whether every UV tile block reads the same (an image or a value repeats in each)
    pub(super) fn same_in_every_block(&self) -> bool {
        !matches!(self, Slot::Anchor { .. })
    }

    /// The slot read in the block at `origin` of a `layout` sized mesh map
    pub(super) fn in_block(&self, origin: (u32, u32), layout: (u32, u32)) -> Slot {
        match self {
            Slot::Anchor { heights, .. } => Slot::Anchor { heights: heights.clone(), origin, layout },
            other => other.clone(),
        }
    }

    pub(super) fn key(&self) -> SlotKey {
        match self {
            Slot::Uniform(value) => SlotKey::Uniform(value.map(f32::to_bits)),
            Slot::Image { image, flip_green } => SlotKey::Image(Arc::downgrade(image), *flip_green),
            Slot::Anchor { heights, .. } => SlotKey::Anchor(Arc::downgrade(heights)),
        }
    }

    /// The slot's color at `uv`, in the file's encoding
    pub(super) fn color(&self, uv: [f32; 2]) -> [f32; 3] {
        match self {
            Slot::Uniform(value) => *value,
            Slot::Image { image, .. } => {
                let [red, green, blue, _] = image.sample(uv[0], uv[1]);
                [red, green, blue]
            }
            Slot::Anchor { .. } => [0.0; 3],
        }
    }

    /// The slot read as a height (its gray) over a `size` block
    pub(super) fn heights(&self, size: (u32, u32)) -> Vec<f32> {
        match self {
            Slot::Anchor { heights, origin, layout } => {
                (0..size.1).flat_map(|row| (0..size.0).map(move |column| (origin.0 + column, origin.1 + row))).map(|texel| heights.at(texel, *layout)).collect()
            }
            _ => block_uvs(size).map(|uv| self.color(uv).iter().sum::<f32>() / 3.0).collect(),
        }
    }

    /// The slot read as a normal over a `size` block: its X and Y, OpenGL convention
    pub(super) fn normals(&self, size: (u32, u32)) -> (Vec<f32>, Vec<f32>) {
        let flip_green = matches!(self, Slot::Image { flip_green: true, .. });
        block_uvs(size)
            .map(|uv| {
                let [red, green, _] = self.color(uv);
                let y = green * 2.0 - 1.0;
                (red * 2.0 - 1.0, if flip_green { -y } else { y })
            })
            .unzip()
    }
}
