use super::FileFormat;

/// How a file is written.
#[derive(Clone, Copy)]
pub(super) struct Encoding {
    pub(super) format: FileFormat,
    pub(super) depth: u8,
    /// Ordered dithering before 8-bit quantization.
    pub(super) dither: bool,
}
