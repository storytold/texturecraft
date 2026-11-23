use super::ExportSetPlan;

/// A running texture export: the texture sets still to write, one per frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportJob {
    /// Each texture set still to write with its `file.exportTextures` parameters (output folder
    /// included), taken when Export was clicked: later edits of the settings do not reach them.
    pub pending: Vec<(usize, serde_json::Value)>,
    pub total: usize,
    /// The original selection and the file names shown while the job runs.
    pub preview: Vec<ExportSetPlan>,
    /// Destination shown by the preview and Open output directory.
    pub directory: String,
    /// File names written so far.
    pub written: Vec<String>,
    /// The frame that wrote the last texture set (a frame's second pass writes none).
    pub frame: u64,
}
