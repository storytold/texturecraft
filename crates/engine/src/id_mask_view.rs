//! The view a color selection picks from while its ID Mask is an image chosen by hand: that image laid on its texture set as the mask reads it

use std::sync::Arc;

use texturecraft_compose::ImageRes;
use texturecraft_core::{EffectKind, LayerId};

use crate::Session;

impl Session {
    /// The texture set of color selection `effect` and the image it reads as its ID mask (None for another effect, the ID mesh map or a missing image)
    pub fn id_mask_image(&self, effect: LayerId) -> Option<(usize, &Arc<ImageRes>)> {
        let (owner, found) = self.doc.project.effect(effect)?;
        let EffectKind::ColorSelection(selection) = &found.kind else { return None };
        let image = self.images.get(selection.id_map.as_deref()?)?;
        let (texture_set, _, _) = self.doc.project.locate(owner)?;
        Some((texture_set, image))
    }

    /// RGBA8 of [`Session::id_mask_image`] on its texture set at `size²`, the stored values unconverted like the ID mesh map's view
    pub fn id_mask_view_rgba(&self, effect: LayerId, size: u32) -> Option<Vec<u8>> {
        let (texture_set, image) = self.id_mask_image(effect)?;
        let (width, height) = self.doc.project.texture_sets.get(texture_set)?.size();
        let size = size.max(1);

        // Each view pixel shows the color its texture set texel reads, sampled at that texel's centre as the mask and a pick do
        let texel_centre = |pixel: u32, extent: u32| {
            let extent = extent.max(1);
            let texel = ((pixel as f32 + 0.5) / size as f32 * extent as f32).floor().min((extent - 1) as f32);
            (texel + 0.5) / extent as f32
        };
        let side = size as usize;
        let byte_count = side.checked_mul(side)?.checked_mul(4)?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(byte_count).ok()?;

        for y in 0..size {
            let v = texel_centre(y, height);
            for x in 0..size {
                let color = image.sample_nearest(texel_centre(x, width), v);
                rgba.extend(color[..3].iter().map(|value| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
                rgba.push(u8::MAX);
            }
        }
        Some(rgba)
    }
}
