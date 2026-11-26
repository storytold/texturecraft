use crate::{EngineError, Result};

/// An evaluated mask: `width × height` values in `0..=1`, row by row from the top
#[derive(Clone, Debug, PartialEq)]
pub struct MaskBitmap {
    /// UDIM number of the tile (None: the whole texture set)
    pub tile: Option<u32>,
    pub width: u32,
    pub height: u32,
    pub values: Vec<f32>,
}

impl MaskBitmap {
    /// The values as 16-bit gray levels
    pub fn gray16(&self) -> Vec<u16> {
        self.values.iter().map(|v| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).collect()
    }

    /// The values as 8-bit gray levels
    pub fn gray8(&self) -> Vec<u8> {
        self.values.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()
    }

    /// A 16-bit grayscale PNG
    pub fn to_png16(&self) -> Result<Vec<u8>> {
        texturecraft_formats::gray16_png(self.width, self.height, self.values.iter().copied()).map_err(|error| EngineError::Other(error.to_string()))
    }
}
