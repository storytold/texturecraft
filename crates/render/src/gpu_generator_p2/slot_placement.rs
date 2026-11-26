/// Where an image slot is read: its scale and offset across the UV square (or the mesh's
/// bounds), its triplanar contrast (None: in UV), and whether the X projection mirrors about the
/// centre (compose's `SlotPlacement`)
pub(super) struct SlotPlacement {
    pub(super) scale: f32,
    pub(super) offset: f32,
    pub(super) triplanar_contrast: Option<f32>,
    pub(super) mirrored_about_centre: bool,
}
