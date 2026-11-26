//! The filter picker hanging from the filter card: a grid of the library's filters that swaps the effect's filter

use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::core::LayerId;
use texturecraft_engine::filter_lib::filter_effects;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::picker_group::PickerGroup;
use crate::picker_tab::PickerTab;
use crate::resource_picker;
use crate::resource_picker_item::ResourcePickerItem;
use crate::tile_picture::TilePicture;

/// The filter picker of filter effect `effect` (library filter `current` now), hanging from its card's row `pick`: a pick runs `effect.set {filter}`, masks offer only mask filters
pub(crate) fn picker(app: &mut TextureCraftApp, pick: &egui::Response, effect: u64, current: Option<&'static str>) {
    let ctx = pick.ctx.clone();
    let on_mask = is_on_mask(app, effect);
    let popup = pick.id.with(("filter_picker", effect));
    let open_requested = take_open_request(&ctx, effect);
    if pick.clicked() || (open_requested && !egui::Popup::is_id_open(&ctx, popup)) {
        resource_picker::toggle(&ctx, popup);
    }

    let tabs = || {
        let mut items: Vec<ResourcePickerItem<&'static str>> = filter_effects()
            .iter()
            .filter(|def| def.mask || !on_mask)
            .map(|def| ResourcePickerItem { id: def.id, label: tr(def.label).to_owned(), caption: None, picture: TilePicture::Filter(def.id) })
            .collect();

        // One kind of resource: one grid in name order, no group captions
        items.sort_by_cached_key(|item| item.label.to_lowercase());
        vec![PickerTab { icon: ic::FUNNEL, name: tr("Filters"), groups: vec![PickerGroup { header: None, items }] }]
    };
    let picked = resource_picker::show(pick, popup, &mut app.thumbs, current.as_ref(), current.is_some(), tabs);
    if let Some(id) = picked.filter(|id| Some(*id) != current) {
        app.run("effect.set", json!({"effect": effect, "filter": id}));
    }
}

/// Has the slot of filter effect `effect` open its picker if it shows this frame or the next, as Add filter does for the filter it adds
pub(crate) fn open_picker_when_shown(ctx: &egui::Context, effect: u64) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| data.insert_temp(open_request_id(), (effect, frame)));
}

/// Whether an open request for `effect`'s slot is waiting, removing it; a request for another effect stays for that effect's slot, one older than a frame lapses
fn take_open_request(ctx: &egui::Context, effect: u64) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| {
        let Some((requested_effect, requested_frame)) = data.get_temp::<(u64, u64)>(open_request_id()) else { return false };

        // Properties can draw before or after the menu that asked, so a request lives for this frame and the next
        let lapsed = frame > requested_frame.saturating_add(1);
        let for_this_slot = requested_effect == effect;
        if lapsed || for_this_slot {
            data.remove_temp::<(u64, u64)>(open_request_id());
        }
        for_this_slot && !lapsed
    })
}

/// Where the effect whose slot should open its picker waits
fn open_request_id() -> egui::Id {
    egui::Id::unique("filter_slot_open_request")
}

/// Whether filter effect `effect` sits in a mask rather than the layer content
pub(crate) fn is_on_mask(app: &TextureCraftApp, effect: u64) -> bool {
    let project = &app.session.doc.project;
    let owner = project.effect(LayerId(effect)).and_then(|(owner, _)| project.layers.get(&owner));
    owner.and_then(|layer| layer.mask.as_ref()).is_some_and(|mask| mask.effects.iter().any(|masked| masked.id.0 == effect))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two filter sheets drawn in one frame: the one for another effect leaves the request to the slot it is for
    #[test]
    fn an_open_request_waits_for_its_own_slot() {
        let ctx = egui::Context::default();
        open_picker_when_shown(&ctx, 2);

        assert!(!take_open_request(&ctx, 1), "not the requested slot");
        assert!(take_open_request(&ctx, 2), "the requested slot still finds it");
        assert!(!take_open_request(&ctx, 2), "taken once");
    }
}
