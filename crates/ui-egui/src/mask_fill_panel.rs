//! A mask fill effect's Properties: the Fill and Symmetry sections a fill layer has, then its Grayscale section with its one source slot

use serde_json::json;
use texturecraft_engine::core::{Fill, LayerId, Projection, Sampling, Source};

use crate::TextureCraftApp;
use crate::fill_material_panel::source_card;
use crate::i18n::tr;
use crate::property_sheet::PropertySheet;
use crate::source_slot::SourceSlot;

/// Every section of mask fill effect `effect` in layer `reader`'s mask
pub(crate) fn mask_fill_sheet(
    app: &mut TextureCraftApp,
    sheet: &mut PropertySheet<'_>,
    effect: LayerId,
    source: Source,
    projection: Projection,
    sampling: Sampling,
    reader: LayerId,
) {
    let base = json!({"effect": effect.0});
    let placement = Fill { projection, sampling, ..Fill::default() };
    crate::fill_panel::placement_sections(app, sheet, &placement, &base, reader);

    sheet.section(tr("Grayscale"), false);
    source_card(app, sheet, source, &base, SourceSlot::Grayscale, reader, false);
}
