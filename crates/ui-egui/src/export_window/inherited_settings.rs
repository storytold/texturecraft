use texturecraft_engine::formats::FileFormat;

use crate::TextureCraftApp;
use crate::state::{FileType, SIZE_OF_LARGEST_SET};

use super::{file_type_name, largest_set};

/// Starting values for a texture set override.
pub(super) struct InheritedSettings {
    pub(super) preset: String,
    pub(super) file_type: FileType,
    pub(super) size: u32,
    pub(super) padding: (String, u32),
}

impl InheritedSettings {
    pub(super) fn for_texture_set(app: &TextureCraftApp, native_size: u32) -> Self {
        let e = &app.ui.export;
        let format = if e.format.is_empty() { FileFormat::Png.name().to_string() } else { file_type_name(&e.format) };
        let (bit_depth, dithering) = match FileFormat::from_extension(&format) {
            Some(f) if e.bit_depth > 0 => (f.depth(e.bit_depth), e.dithering),
            Some(f) => (f.default_depth(), false),
            None => (8, false),
        };
        let size = match e.size {
            0 => native_size,
            SIZE_OF_LARGEST_SET => largest_set(app),
            n => n,
        };
        Self { preset: e.preset.clone(), file_type: FileType { format, bit_depth, dithering }, size, padding: (e.padding.clone(), e.dilation) }
    }
}
