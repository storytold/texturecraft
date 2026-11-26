//! Mask export: a layer's evaluated mask as a grayscale bitmap at the texture set's resolution
//! (`mask.export`, and the layer menu's "Copy mask to clipboard").
//!
//! The bitmap has the orientation of the exported texture maps (row 0 is the top of the UV
//! square) and is black where the mesh does not cover the texture. A classic texture set gives one
//! bitmap; a UV-tiled set gives one per tile (what the texture export does) or, for the clipboard,
//! one of the whole tile atlas.

use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;
use texturecraft_core::LayerId;

use crate::commands::{CommandSpec, bad, has_layer};
#[cfg(not(target_arch = "wasm32"))]
use crate::commands::{layer_param, strp};
pub use crate::mask_bitmap::MaskBitmap;
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "mask.export",
        "Export mask to file...",
        [],
        None,
        "{layer?, path: .png file} → {files}. Writes the evaluated mask as a 16-bit grayscale PNG at the texture set's resolution, black outside the mesh and oriented like the exported textures. A UV-tiled set writes one file per tile with the tile number before the extension (mask.1001.png). A layer without an enabled mask is an error",
        has_layer,
        export
    )]
}

impl Session {
    /// Layer `id`'s evaluated mask: one bitmap per UV tile (`per_tile`) or one of the whole
    /// texture set. A tile smaller than its atlas block gives its own size. Errors without an
    /// enabled mask.
    pub fn mask_bitmaps(&mut self, id: LayerId, per_tile: bool) -> Result<Vec<MaskBitmap>> {
        let (ts, _, _) = self.doc.project.locate(id).ok_or_else(|| EngineError::Other(format!("no layer {}", id.0)))?;
        self.update();
        let atlas = self
            .with_inputs(ts, |inputs| texturecraft_compose::mask_texels(inputs, id).map(|values| (inputs.gbuf.width, inputs.gbuf.height, values)))
            .ok_or_else(|| EngineError::Other("the layer has no enabled mask".into()))?;
        let (width, height, values) = atlas;
        let set = self.doc.project.texture_set(ts).map_err(|e| EngineError::Other(e.to_string()))?;
        let Some(grid) = set.tile_grid().filter(|_| per_tile) else { return Ok(vec![MaskBitmap { tile: None, width, height, values }]) };
        let block = set.block_resolution();
        set.uv_tiles
            .iter()
            .map(|&tile| {
                let (block_x, block_y) = grid.block(tile).ok_or_else(|| EngineError::Other(format!("UV tile {tile} is not in the set's grid")))?;

                // A tile smaller than its block fills the block's top-left corner
                let side = ((block as f32 * set.tile_scale(tile)).round() as u32).clamp(1, block);
                let mut tile_values = Vec::with_capacity(side as usize * side as usize);
                for row in 0..side {
                    let start = ((block_y * block + row) as usize) * width as usize + (block_x * block) as usize;
                    tile_values.extend_from_slice(values.get(start..start + side as usize).unwrap_or(&[]));
                }
                Ok(MaskBitmap { tile: Some(tile), width: side, height: side, values: tile_values })
            })
            .collect()
    }
}

/// `path` as a `.png` path, with `tile` before the extension for a UV tile.
#[cfg(not(target_arch = "wasm32"))]
fn file_path(path: &str, tile: Option<u32>) -> std::path::PathBuf {
    let path = std::path::Path::new(path);
    let is_png = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"));
    let name = path.file_name().map_or_else(|| "mask".into(), |n| n.to_string_lossy().into_owned());
    let stem = if is_png { name[..name.len() - 4].to_string() } else { name };
    let named = match tile {
        Some(tile) => format!("{stem}.{tile}.png"),
        None => format!("{stem}.png"),
    };
    path.with_file_name(named)
}

#[cfg(not(target_arch = "wasm32"))]
fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let path = strp(p, "path").filter(|path| !path.trim().is_empty()).ok_or_else(|| bad("mask.export", "`path` is required"))?;
    let id = layer_param(s, "mask.export", p)?;
    let bitmaps = s.mask_bitmaps(id, true)?;
    let mut files = Vec::new();
    for bitmap in &bitmaps {
        let file = file_path(path, bitmap.tile);
        texturecraft_formats::write_atomic(&file, &bitmap.to_png16()?).map_err(|e| bad("mask.export", format!("cannot write {}: {e}", file.display())))?;
        files.push(file.display().to_string());
    }
    s.log.info("MapExporter", format!("Exported the mask to {}", files.join(", ")));
    Ok(json!({ "files": files }))
}

#[cfg(target_arch = "wasm32")]
fn export(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad("mask.export", "writing files is not available in the browser"))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::file_path;

    #[test]
    fn the_tile_number_goes_before_the_png_extension() {
        // Compared as paths: Windows writes the folder with backslashes
        let name = |path: &str, tile| {
            let file = file_path(path, tile);
            assert_eq!(file.parent(), std::path::Path::new(path).parent(), "the folder is kept");
            file.file_name().unwrap().to_string_lossy().into_owned()
        };
        assert_eq!(name("/a/mask.png", None), "mask.png");
        assert_eq!(name("/a/mask", None), "mask.png");
        assert_eq!(name("/a/mask.PNG", Some(1002)), "mask.1002.png");
        assert_eq!(name("/a/skin.v2", Some(1001)), "skin.v2.1001.png");
    }
}
