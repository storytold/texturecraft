use serde_json::Value;
use std::collections::BTreeMap;

/// The open import dialog.
pub struct PbrDialog {
    /// `{path}` or `{files}`: where the maps come from.
    pub(super) source: Value,
    /// What the scan found.
    pub(super) scan: Value,
    pub(super) name: String,
    /// The file picked per map where several were found.
    pub(super) choose: BTreeMap<String, String>,
    /// 0: automatic (from the file name), 1: OpenGL, 2: DirectX.
    pub(super) normal: usize,
    pub(super) triplanar: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) library: bool,
}
