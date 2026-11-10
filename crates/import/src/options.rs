//! [`ImportOptions`]: everything an import takes besides the file. The import reads nothing else (no environment, no
//! files next to the project), so the same file and options give the same project.

/// What the importer takes besides the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOptions {
    /// Shows the original's stored result on top of stacks whose layers don't reproduce it (as "Original result"). Off
    /// by default: the project shows what its layer stack computes, and the original's result stays as the hidden
    /// reference ("Original result (reference)").
    pub original_on_top: bool,
    /// Replaces the few compiled filter graphs that have a close built-in filter (invert, clamp, blur, sharpen, HSL)
    /// with it, approximated. Off by default: every filter graph is a placeholder until graphs are computed.
    pub native_filters: bool,
    /// Composites each texture set and compares it with the original's stored result (the safety net). On by default;
    /// without it every stack counts as not measured.
    pub measure: bool,
    /// Largest texture set resolution kept (larger sets are imported at this size, the original's result reduced);
    /// `None` keeps the file's resolution.
    pub max_resolution: Option<u32>,
    /// Replays brush strokes with the replayer the import is given ([`crate::Hooks::replay`]); on by default. Off, or
    /// without a replayer, strokes stay placeholders and their pixels only in the original's stored result.
    pub replay_strokes: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions { original_on_top: false, native_filters: false, measure: true, max_resolution: None, replay_strokes: true }
    }
}
