/// Where a generator's image slot is read: `scale` times across the UV square (or the mesh's bounds, triplanar), plus `offset`
pub(crate) struct SlotPlacement {
    pub scale: f32,
    pub offset: f32,
    /// How sharply the three projections meet, or None to read in UV
    pub triplanar_contrast: Option<f32>,
    /// The X projection's mirrored U turns about the bounds' centre, which a centred scale keeps fixed (Use Texture); else about 0, as custom grunge was fitted
    pub mirrored_about_centre: bool,
}
