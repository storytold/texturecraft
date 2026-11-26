use std::path::{Path, PathBuf};

use texturecraft_automation::AutomationError;

/// Why a subcommand failed: printed as `error: {e}`, exit 1.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CliError {
    #[error(transparent)]
    Automation(#[from] AutomationError),
    /// The engine's commands take paths as JSON text, which holds only Unicode.
    #[error("{}: the path is not valid Unicode", .0.display())]
    NotUnicode(PathBuf),
    #[error("unknown built-in mesh `{0}`")]
    UnknownBuiltin(String),
    #[error("cannot read {}: {message}", path.display())]
    Read { path: PathBuf, message: String },
    #[error("cannot write {}: {source}", path.display())]
    Write { path: PathBuf, source: std::io::Error },
    #[error("{} is not JSON: {source}", path.display())]
    Script { path: PathBuf, source: serde_json::Error },
    #[error("the script must be a list of steps or {{\"steps\": [...]}}")]
    ScriptShape,
    #[error("the script's `{0}` field must be a string")]
    ScriptField(&'static str),
    #[error("step {index} (`{id}`) failed: {message}")]
    BatchStopped { index: String, id: String, message: String },
    #[error("{} is already a project", .0.display())]
    AlreadyProject(PathBuf),
    #[error("{0}")]
    Serve(std::io::Error),
}

/// A path as the text the engine's commands take.
pub(crate) fn unicode(path: &Path) -> Result<&str, CliError> {
    path.to_str().ok_or_else(|| CliError::NotUnicode(path.to_owned()))
}
