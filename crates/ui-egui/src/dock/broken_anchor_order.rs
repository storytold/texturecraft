use serde_json::{Value, json};
use texturecraft_engine::core::{LayerId, Project};

use super::anchor_link::{layer_anchors, stack_order};

/// An anchor point positioned at or above a layer that reads it, breaking the link
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BrokenAnchorOrder {
    pub(super) anchor_name: String,
    pub(super) owner: LayerId,
    pub(super) lowest_reader: LayerId,
    pub(super) lowest_reader_name: String,
}

/// Every anchor point in `texture_set` that sits at or above at least one reading layer
pub(super) fn broken_anchor_orders(project: &Project, texture_set: usize) -> Vec<BrokenAnchorOrder> {
    let order = stack_order(project, texture_set);
    let mut broken = Vec::new();
    for (owner_id, _) in project.walk(texture_set) {
        let Some(owner) = project.layers.get(&owner_id) else {
            continue;
        };

        let owner_pos = order.get(&owner_id).copied().unwrap_or(0);
        for (anchor, anchor_name) in layer_anchors(owner) {
            let lowest = project
                .anchor_readers(anchor)
                .into_iter()
                .filter_map(|(reader, _)| order.get(&reader).copied().filter(|&pos| owner_pos >= pos).map(|pos| (pos, reader)))
                .min_by_key(|(pos, _)| *pos);
            if let Some((_, lowest_reader)) = lowest
                && let Some(reader_layer) = project.layers.get(&lowest_reader)
            {
                broken.push(BrokenAnchorOrder {
                    anchor_name: anchor_name.to_owned(),
                    owner: owner_id,
                    lowest_reader,
                    lowest_reader_name: reader_layer.name.clone(),
                });
            }
        }
    }
    broken
}

/// Parameters for `layer.move` that place the anchor layer below its lowest reader again
pub(super) fn fix_anchor_order_command(project: &Project, broken: &BrokenAnchorOrder) -> Value {
    let Some((ts_owner, parent_owner, _)) = project.locate(broken.owner) else {
        return json!({"layer": broken.owner.0, "index": 0});
    };

    let Some((ts_reader, parent_reader, idx_reader)) = project.locate(broken.lowest_reader) else {
        return json!({"layer": broken.owner.0, "index": 0});
    };

    if ts_owner == ts_reader && parent_owner == parent_reader {
        return json!({"layer": broken.owner.0, "index": idx_reader});
    }

    if parent_owner.is_none() {
        let mut root = broken.lowest_reader;
        while let Some((_, Some(parent), _)) = project.locate(root) {
            root = parent;
        }
        if let Some((_, None, root_idx)) = project.locate(root) {
            return json!({"layer": broken.owner.0, "index": root_idx});
        }
    }

    json!({"layer": broken.lowest_reader.0, "above": broken.owner.0, "inside": false})
}
