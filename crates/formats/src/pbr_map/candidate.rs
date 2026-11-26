use super::Detected;

/// One candidate file.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// The file's path as given.
    pub name: String,
    pub detected: Detected,
    /// File size in bytes (a tie-breaker; 0 when unknown).
    pub bytes: u64,
}
