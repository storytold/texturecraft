//! Shelf preview work submitted to [`crate::preview_worker::PreviewWorker`]

use std::path::Path;

use egui::ColorImage;

use crate::mask_preview_image::mask_preview_image;
use crate::material_preview_image::material_preview_image;
use crate::orb_shape::OrbShape;
use crate::shelf_preview::ShelfPreview;

/// Largest user smart material file previewed; a bigger one keeps its plain list entry
#[cfg(not(target_arch = "wasm32"))]
const MAX_MATERIAL_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Render `preview` at `size_px²`; the answer comes back under `key`
pub struct PreviewRequest {
    pub key: String,
    pub preview: ShelfPreview,
    pub size_px: usize,
    /// A hover preview: the worker renders the newest of these before anything else
    // The web renders each request at once, so nothing is reordered there
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub hover: bool,
}

impl PreviewRequest {
    /// Render the asset, baking the orb on first use for smart masks and smart materials
    /// Unknown assets and failed bakes return None
    pub fn render(&self, shape: &mut OrbShape) -> Option<ColorImage> {
        match &self.preview {
            ShelfPreview::SmartMask(id) => Some(mask_preview_image(self.size_px, &shape.get()?.mask(id, self.size_px)?)),
            ShelfPreview::SmartMaterial(id) => Some(material_preview_image(self.size_px, &shape.get()?.material(id, self.size_px)?)),
            ShelfPreview::UserMaterial(path) => {
                let bytes = material_file_bytes(path)?;
                Some(material_preview_image(self.size_px, &shape.get()?.material_from_bytes(&bytes, self.size_px)?))
            }
            preview => preview.image(self.size_px),
        }
    }
}

/// The bytes of the `.tcmat` file at `path`, or None for other files, unreadable ones and ones too large to preview
#[cfg(not(target_arch = "wasm32"))]
fn material_file_bytes(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;

    if path.extension()? != texturecraft_engine::material_lib::SMART_MATERIAL_EXT {
        return None;
    }

    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_MATERIAL_FILE_BYTES {
        return None;
    }

    // The limit holds even if the file grows while it is read
    let mut bytes = Vec::new();
    file.take(MAX_MATERIAL_FILE_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_MATERIAL_FILE_BYTES).then_some(bytes)
}

/// The web has no library folder, so no file to read
#[cfg(target_arch = "wasm32")]
fn material_file_bytes(_: &Path) -> Option<Vec<u8>> {
    None
}
