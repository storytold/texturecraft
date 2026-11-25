use std::sync::Arc;

use texturecraft_compose::ImageRes;
use texturecraft_core::Channel;

use super::LinkMemoryBudget;
use crate::{EngineError, Result};

/// A validated bake ready to install in the document and decoded-image cache.
#[derive(Clone, Debug)]
pub struct CookedMap {
    pub channel: Channel,
    pub bytes: Arc<Vec<u8>>,
    pub image: Arc<ImageRes>,
}

impl CookedMap {
    pub(super) fn decoded_bytes(texels: u64) -> u64 {
        texels.saturating_mul(std::mem::size_of::<[f32; 4]>() as u64)
    }

    pub(super) fn decoder_peak_bytes(texels: u64) -> u64 {
        // Native image samples can coexist with the linear-sized float output during expansion
        Self::decoded_bytes(texels).saturating_mul(2)
    }

    pub fn decode(channel: Channel, bytes: Vec<u8>, size: [u32; 2], budget: &mut LinkMemoryBudget) -> Result<Self> {
        let texels = u64::from(size[0]) * u64::from(size[1]);
        if texels > texturecraft_formats::MAX_RESOURCE_TEXELS {
            return Err(EngineError::Other(format!("{} map exceeds the project resource limit", channel.id())));
        }

        let encoded_bytes = bytes.capacity() as u64;
        let decoder_peak_bytes = Self::decoder_peak_bytes(texels);
        budget.check(encoded_bytes.saturating_add(decoder_peak_bytes))?;
        #[cfg(not(target_arch = "wasm32"))]
        budget.check_available(decoder_peak_bytes)?;

        let decoder_bytes = budget.remaining().saturating_sub(encoded_bytes).saturating_sub(Self::decoded_bytes(texels));
        let image = texturecraft_formats::decode_cooked_image(&bytes, size, decoder_bytes)
            .map_err(|error| EngineError::Other(format!("{} map: {error}", channel.id())))?;
        if [image.width, image.height] != size {
            return Err(EngineError::Other(format!("{} map dimensions disagree with the export report", channel.id())));
        }

        budget.reserve(encoded_bytes.saturating_add(Self::decoded_bytes(image.rgba.capacity() as u64)))?;
        Ok(Self { channel, bytes: Arc::new(bytes), image: crate::resource_cmds::decoded(image) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_retained_map_reduces_the_next_decoders_budget() {
        let bytes = texturecraft_formats::gray16_png(4, 4, [0.5; 16].into_iter()).unwrap();
        let copy = bytes.clone();
        let size = [4, 4];
        let first_bytes = copy.capacity() as u64 + CookedMap::decoded_bytes(16);
        let second_peak = bytes.capacity() as u64 + CookedMap::decoder_peak_bytes(16);
        let mut budget = LinkMemoryBudget::new(first_bytes + second_peak - 1);

        let first = CookedMap::decode(Channel::Roughness, copy, size, &mut budget).unwrap();
        let second = CookedMap::decode(Channel::Metallic, bytes, size, &mut budget);

        assert_eq!(first.image.rgba.len(), 16);
        assert!(second.unwrap_err().to_string().contains("configured cap"));
    }
}
