use texturecraft_engine::formats::FileFormat;

/// A file an output map writes for the current texture set, as `export.plan` lists it
#[derive(Clone)]
pub(super) struct PlannedOutput {
    pub(super) file: String,
    /// The picture shows the full atlas while export writes individual tiles
    pub(super) tiled: bool,
    /// Planned file width and height in pixels
    pub(super) dimensions: (u64, u64),
    pub(super) format: Option<FileFormat>,
    pub(super) bit_depth: u8,
    pub(super) dithering: bool,
}
