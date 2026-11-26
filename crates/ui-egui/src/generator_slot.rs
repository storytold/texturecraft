//! The generator picker a generator's slot card opens: picking a kind swaps the generator to it

use egui_phosphor::regular as ic;
use serde_json::json;
use texturecraft_engine::core::GeneratorKind;

use crate::TextureCraftApp;
use crate::i18n::tr;
use crate::picker_group::PickerGroup;
use crate::picker_tab::PickerTab;
use crate::resource_picker;
use crate::resource_picker_item::ResourcePickerItem;
use crate::tile_picture::TilePicture;

/// The picker of generator effect `effect` (a `current` generator) under its slot `pick`: a pick runs `effect.set {generator}`
pub(crate) fn picker(app: &mut TextureCraftApp, pick: &egui::Response, effect: u64, current: GeneratorKind) {
    let ctx = pick.ctx.clone();
    let popup = pick.id.with(("generator_picker", effect));
    if pick.clicked() {
        resource_picker::toggle(&ctx, popup);
    }

    let tabs = || {
        let mut items: Vec<ResourcePickerItem<GeneratorKind>> = GeneratorKind::ALL
            .into_iter()
            .filter(|kind| kind.in_picker())
            .map(|kind| ResourcePickerItem { id: kind, label: tr(kind.label()).to_owned(), caption: None, picture: TilePicture::Generator(kind) })
            .collect();

        // One kind of resource: one grid in name order, no group captions
        items.sort_by_cached_key(|item| item.label.to_lowercase());
        vec![PickerTab { icon: ic::SPARKLE, name: tr("Generators"), groups: vec![PickerGroup { header: None, items }] }]
    };
    let picked = resource_picker::show(pick, popup, &mut app.thumbs, Some(&current), true, tabs);
    if let Some(kind) = picked.filter(|kind| *kind != current) {
        app.run("effect.set", json!({"effect": effect, "generator": kind.id()}));
    }
}
