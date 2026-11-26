use super::{CursorImage, SwatchSide};

/// What a pointer picture shows: its cell in pixels and the colour disc's colour and side, if any
pub(super) type CursorLook = (u32, Option<([u8; 3], SwatchSide)>);

/// The eyedropper picture uploaded for one look, kept between frames
#[derive(Clone)]
pub(super) struct CursorTexture {
    pub(super) texture: egui::TextureHandle,
    look: CursorLook,
    pub(super) size: u32,
    pub(super) hotspot: [u32; 2],
}

impl CursorTexture {
    /// The eyedropper picture for `look`, uploaded again only when the look changes (the disc's colour, its side, the pixel size)
    pub(super) fn for_look(ctx: &egui::Context, look: CursorLook) -> Self {
        let id = egui::Id::unique("eyedropper_cursor_texture");
        if let Some(cursor) = ctx.data(|data| data.get_temp::<CursorTexture>(id)).filter(|cursor| cursor.look == look) {
            return cursor;
        }

        let image = match look {
            (cell, Some((color, side))) => CursorImage::eyedropper_with_swatch(cell, color, side),
            (cell, None) => CursorImage::eyedropper(cell),
        };
        let pixels = egui::ColorImage::from_rgba_unmultiplied([image.size as usize; 2], &image.rgba);
        let texture = ctx.load_texture("eyedropper_cursor", pixels, egui::TextureOptions::NEAREST);
        let cursor = CursorTexture { texture, look, size: image.size, hotspot: image.hotspot };
        ctx.data_mut(|data| data.insert_temp(id, cursor.clone()));
        cursor
    }
}
