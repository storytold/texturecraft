//! The sections of a paint tool's Properties, each with its jump tab

use egui_phosphor::regular as ic;

use crate::i18n::tr;

/// One section of a paint tool's Properties page
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaintSection {
    Brush,
    Alpha,
    Stencil,
    Material,
    /// The Material section's place while a mask is painted
    Grayscale,
}

impl PaintSection {
    /// The sections in page order: Stencil only for a tool that `uses_stencil`, Grayscale instead of Material unless `content` is painted
    pub(crate) fn page(uses_stencil: bool, content: bool) -> Vec<PaintSection> {
        let mut sections = vec![PaintSection::Brush, PaintSection::Alpha];
        if uses_stencil {
            sections.push(PaintSection::Stencil);
        }
        sections.push(if content { PaintSection::Material } else { PaintSection::Grayscale });
        sections
    }

    /// The section's caption and tab name
    pub(crate) fn title(self) -> &'static str {
        match self {
            PaintSection::Brush => tr("Brush"),
            PaintSection::Alpha => tr("Alpha"),
            PaintSection::Stencil => tr("Stencil"),
            PaintSection::Material => tr("Material"),
            PaintSection::Grayscale => tr("Grayscale"),
        }
    }

    /// The tab's icon
    pub(crate) fn icon(self) -> &'static str {
        match self {
            PaintSection::Brush => ic::PAINT_BRUSH,
            PaintSection::Alpha => ic::CIRCLE_HALF,
            PaintSection::Stencil => ic::FRAME_CORNERS,
            PaintSection::Material => ic::SPHERE,
            PaintSection::Grayscale => ic::SQUARE_HALF,
        }
    }
}
