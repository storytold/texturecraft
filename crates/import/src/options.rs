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
    /// Puts a correction layer on top of stacks that do not reproduce the original: the original's stored result on the
    /// texels the layers don't reproduce ([`crate::correction`]), so the project looks as the original saved it while
    /// every layer can still be switched on and off underneath. Off by default: the project shows what its layer stack
    /// computes. Needs [`Self::measure`].
    pub original_correction: bool,
    /// Composites each texture set and compares it with the original's stored result (the safety net). On by default;
    /// without it every stack counts as not measured.
    pub measure: bool,
    /// Prepare graph outputs during import for consumers that save or inspect the result without compositing a view.
    /// Interactive consumers that immediately composite the project can evaluate graphs in that first pass instead.
    pub precompute_graphs: bool,
    /// Largest texture set resolution kept (larger sets are imported at this size, the original's result reduced);
    /// `None` keeps the file's resolution.
    pub max_resolution: Option<u32>,
    /// Replays brush strokes with the replayer the import is given ([`crate::Hooks::replay`]); on by default. Off, or
    /// without a replayer, strokes stay placeholders and their pixels only in the original's stored result.
    pub replay_strokes: bool,
    /// Runs the project's compiled graphs, decompiled ([`crate::GraphMode::Decompile`], the default), or keeps every use
    /// a locked placeholder.
    pub graphs: crate::GraphMode,
    /// Translates the layer stacks and nothing else: no stored result or baked maps read, no strokes replayed, no
    /// polygon fills laid, no graph cooked, no safety net. Strokes count as replayable (as with a replayer). For a
    /// census of the layers an import locks and the placeholders it leaves; the project holds no pixels. Off by default.
    pub translate_only: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            original_on_top: false,
            original_correction: false,
            native_filters: false,
            measure: true,
            precompute_graphs: true,
            max_resolution: None,
            replay_strokes: true,
            graphs: crate::GraphMode::Decompile,
            translate_only: false,
        }
    }
}
