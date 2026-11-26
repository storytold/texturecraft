use std::sync::{Arc, mpsc};

use texturecraft_engine::{Result, file_cmds::SaveOutput, formats::project_file::SaveProgress};

pub(crate) struct SaveJob {
    pub(super) progress: Arc<SaveProgress>,
    pub(super) output: mpsc::Receiver<Result<SaveOutput>>,
    pub(super) project_id: u64,
    pub(super) for_prompt: bool,
    pub(super) remember: bool,
    pub(super) quit_requested_by: Option<u64>,
}
