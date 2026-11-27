use std::collections::HashMap;

use texturecraft_engine::core::{EffectKind, Layer, LayerId, Project};

/// Anchor reference held by a layer, including whether the anchor currently sits above the reader
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AnchorLink {
    pub(super) anchor: LayerId,
    pub(super) anchor_name: String,
    pub(super) owner: LayerId,
    pub(super) broken: bool,
}

/// Bottom-to-top walk index of every layer in `texture_set`
pub(super) fn stack_order(project: &Project, texture_set: usize) -> HashMap<LayerId, usize> {
    project.walk(texture_set).into_iter().enumerate().map(|(position, (id, _))| (id, position)).collect()
}

/// Anchors defined on `layer` (either as content effects or inside its mask stack)
pub(super) fn layer_anchors(layer: &Layer) -> impl Iterator<Item = (LayerId, &str)> {
    let content = layer.content_effects.iter();
    let mask = layer.mask.iter().flat_map(|mask| mask.effects.iter());
    content.chain(mask).filter(|effect| matches!(effect.kind, EffectKind::Anchor)).map(|effect| (effect.id, effect.name.as_str()))
}

/// Whether `layer` owns anchor point `anchor`
pub(super) fn layer_owns_anchor(layer: &Layer, anchor: LayerId) -> bool {
    layer_anchors(layer).any(|(id, _)| id == anchor)
}

/// Every anchor point read by `layer_id` in bottom-to-top stack order
pub(super) fn layer_anchor_links(project: &Project, layer_id: LayerId) -> Vec<AnchorLink> {
    let Some((texture_set, _, _)) = project.locate(layer_id) else {
        return Vec::new();
    };

    let order = stack_order(project, texture_set);
    let Some(&reader_pos) = order.get(&layer_id) else {
        return Vec::new();
    };

    let mut links = Vec::new();
    for (owner_id, _) in project.walk(texture_set) {
        let Some(owner) = project.layers.get(&owner_id) else {
            continue;
        };

        let owner_pos = order.get(&owner_id).copied().unwrap_or(0);
        for (anchor, anchor_name) in layer_anchors(owner) {
            if project.anchor_readers(anchor).iter().any(|(reader, _)| *reader == layer_id) {
                links.push(AnchorLink { anchor, anchor_name: anchor_name.to_owned(), owner: owner_id, broken: owner_pos >= reader_pos });
            }
        }
    }
    links
}
