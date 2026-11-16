//! Selection cases for the tool availability tests

use texturecraft_engine::Tool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Selection {
    PaintContent,
    FillContent,
    PaintMask,
    FillMask,
    FolderMask,
    InstanceOwnMask,
    PaintSublayer,
    PaintSublayerInMask,
    PaintSublayerInFill,
    PathSublayer,
    FilterInPaint,
    FilterInFill,
    FilterInMask,
    FillEffectInPaint,
    GeneratorInFillMask,
    FolderContent,
    InstanceOfPaint,
    InstanceOfFill,
    Nothing,
    QuickMaskOverPaint,
    QuickMaskOverFill,
    QuickMaskOverNothing,
}

impl Selection {
    pub(super) const ALL: [Selection; 22] = [
        Selection::PaintContent,
        Selection::FillContent,
        Selection::PaintMask,
        Selection::FillMask,
        Selection::FolderMask,
        Selection::InstanceOwnMask,
        Selection::PaintSublayer,
        Selection::PaintSublayerInMask,
        Selection::PaintSublayerInFill,
        Selection::PathSublayer,
        Selection::FilterInPaint,
        Selection::FilterInFill,
        Selection::FilterInMask,
        Selection::FillEffectInPaint,
        Selection::GeneratorInFillMask,
        Selection::FolderContent,
        Selection::InstanceOfPaint,
        Selection::InstanceOfFill,
        Selection::Nothing,
        Selection::QuickMaskOverPaint,
        Selection::QuickMaskOverFill,
        Selection::QuickMaskOverNothing,
    ];

    /// Whether the shelf offers `tool` for this selection
    pub(super) fn offers(self, tool: Tool) -> bool {
        let reads_content = matches!(tool, Tool::Smudge | Tool::Clone);
        match tool {
            Tool::ColorPicker => true,
            Tool::GeometryMask => !matches!(self, Selection::Nothing | Selection::QuickMaskOverNothing),
            _ => match self {
                Selection::PaintContent | Selection::PaintSublayer => true,
                Selection::PaintSublayerInFill => tool != Tool::Path,
                Selection::PaintMask | Selection::FillMask | Selection::FolderMask | Selection::InstanceOwnMask | Selection::PaintSublayerInMask => {
                    !reads_content
                }
                Selection::PathSublayer => tool == Tool::Path,
                Selection::QuickMaskOverPaint | Selection::QuickMaskOverFill | Selection::QuickMaskOverNothing => tool != Tool::Path,
                _ => false,
            },
        }
    }

    /// Greyed in the shelf, but scripts still paint here: below the effect, or the instance's source
    pub(super) fn scripts_still_paint(self) -> bool {
        matches!(
            self,
            Selection::FilterInPaint | Selection::FillEffectInPaint | Selection::FilterInMask | Selection::GeneratorInFillMask | Selection::InstanceOfPaint
        )
    }
}
