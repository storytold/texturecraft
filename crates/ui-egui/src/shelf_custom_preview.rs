//! A picture the user picked to show on an asset's tile instead of its rendered thumbnail ("Set custom preview..." in the asset menu)

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::shelf_entry::ShelfEntry;

/// The custom preview's size in pixels: the largest tile's thumbnail on a high-density screen
pub(crate) const CUSTOM_PREVIEW_PX: u32 = 164;

/// The image file shown on the tile of the asset with [`ShelfEntry::identity`] `kind` and `id`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShelfCustomPreview {
    pub kind: String,
    pub id: String,
    pub file: PathBuf,
}

impl ShelfCustomPreview {
    fn is(&self, entry: &ShelfEntry) -> bool {
        let (kind, id) = entry.identity();
        self.kind == kind && self.id == id
    }
}

/// The image file `entry` shows instead of its thumbnail, if the user picked one
pub(crate) fn file_for<'a>(previews: &'a [ShelfCustomPreview], entry: &ShelfEntry) -> Option<&'a Path> {
    previews.iter().find(|preview| preview.is(entry)).map(|preview| preview.file.as_path())
}

/// Shows `file` on the tile of the asset with identity `kind` and `id`, replacing an earlier pick
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn set(previews: &mut Vec<ShelfCustomPreview>, kind: String, id: String, file: PathBuf) {
    previews.retain(|preview| preview.kind != kind || preview.id != id);
    previews.push(ShelfCustomPreview { kind, id, file });
}

/// Shows `entry`'s own thumbnail again
pub(crate) fn remove(previews: &mut Vec<ShelfCustomPreview>, entry: &ShelfEntry) {
    previews.retain(|preview| !preview.is(entry));
}

/// The largest centred square of `image`, scaled to [`CUSTOM_PREVIEW_PX`] (smaller images keep their size)
pub(crate) fn square(image: &image::DynamicImage) -> image::RgbaImage {
    use image::{DynamicImage, GenericImageView, imageops};

    let side = image.width().min(image.height());
    let left = (image.width() - side) / 2;
    let top = (image.height() - side) / 2;
    let size = side.min(CUSTOM_PREVIEW_PX);
    let filter = imageops::FilterType::Triangle;

    // Sample common thumbnail formats directly from the decoded pixels
    match image {
        DynamicImage::ImageRgba8(pixels) => imageops::resize(&*pixels.view(left, top, side, side), size, size, filter),
        DynamicImage::ImageRgb8(pixels) => DynamicImage::ImageRgb8(imageops::resize(&*pixels.view(left, top, side, side), size, size, filter)).into_rgba8(),
        other => other.crop_imm(left, top, side, side).resize_exact(size, size, filter).into_rgba8(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_picture_keeps_its_middle() {
        let red = image::Rgba([255, 0, 0, 255]);
        let blue = image::Rgba([0, 0, 255, 255]);
        let wide = image::RgbaImage::from_fn(6, 2, |x, _| if (2..4).contains(&x) { blue } else { red });

        let square = square(&image::DynamicImage::ImageRgba8(wide));

        assert_eq!(square.width(), square.height(), "the preview is square");
        assert!(square.pixels().all(|pixel| *pixel == blue), "only the middle of a wide picture shows");
    }
}
