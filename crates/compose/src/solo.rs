//! The view-only "solo" of one layer: the project as the viewport shows it while a layer is
//! isolated. The document itself is never changed; the compositor reads this copy instead.

use std::collections::BTreeSet;

use texturecraft_core::project::MAX_DEPTH;
use texturecraft_core::{LayerId, Mask, MaskBackground, Project};

/// `project` as shown while `layer` is soloed: every other layer of its texture set hidden, the
/// layer and the folders around it shown (even if their eyes are off), and with `ignore_mask`
/// the layer and its folders drawn at full coverage. Other texture sets stay as they are.
/// None when `layer` is in no stack.
pub fn solo_project(project: &Project, layer: LayerId, ignore_mask: bool) -> Option<Project> {
    let (ts, mut parent, _) = project.locate(layer)?;
    let set = project.texture_sets.get(ts)?;

    // The layer and the folders holding it
    let mut shown = vec![layer];
    for _ in 0..=MAX_DEPTH {
        let Some(folder) = parent else { break };
        shown.push(folder);
        parent = project.locate(folder).and_then(|located| located.1);
    }

    // What stays drawn: those, and a soloed folder's contents
    let mut kept: BTreeSet<LayerId> = shown.iter().copied().collect();
    contents(project, &[layer], &mut kept, 0);

    // A soloed instance draws its source's contents, which must stay visible too
    kept.extend(project.render_tree(&[layer]));
    let mut every = BTreeSet::new();
    contents(project, &set.stack, &mut every, 0);

    let mut view = project.clone();
    for id in every.difference(&kept) {
        if let Some(hidden) = view.layers.get_mut(id) {
            hidden.visible = false;
        }
    }
    for id in &shown {
        let Some(drawn) = view.layers.get_mut(id) else { continue };
        drawn.visible = true;
        if !ignore_mask {
            continue;
        }

        let instance = drawn.instance_source().is_some();
        match drawn.mask.as_mut() {
            Some(mask) => mask.enabled = false,

            // An instance without its own mask draws its source's: an own, disabled one overrides it
            None if instance => drawn.mask = Some(Mask { enabled: false, ..Mask::new(MaskBackground::White) }),
            None => {}
        }
    }
    Some(view)
}

/// `roots` and everything in the folders among them (instances' sources are not followed:
/// they may belong to another texture set).
fn contents(project: &Project, roots: &[LayerId], out: &mut BTreeSet<LayerId>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }

    for id in roots {
        out.insert(*id);
        if let Some(layer) = project.layers.get(id) {
            contents(project, layer.children(), out, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_core::{Channel, Layer, LayerKind, TextureSet};

    const CHANNELS: [Channel; 1] = [Channel::BaseColor];

    fn project() -> Project {
        let mut p = Project::new("t", "");
        p.texture_sets.push(TextureSet::new("a", 0, 64));
        p.texture_sets.push(TextureSet::new("b", 0, 64));
        p
    }

    fn add(p: &mut Project, ts: usize, kind: LayerKind, above: Option<LayerId>, inside: bool) -> LayerId {
        let id = p.alloc_id();
        p.insert_layer(ts, Layer::new(id, "layer", kind, &CHANNELS), above, inside).unwrap();
        id
    }

    fn masked(p: &mut Project, id: LayerId) {
        p.layer_mut(id).unwrap().mask = Some(Mask::new(MaskBackground::Black));
    }

    #[test]
    fn solo_hides_the_others_and_leaves_the_original_alone() {
        let mut p = project();
        let (below, rust, other_set) =
            (add(&mut p, 0, LayerKind::Paint, None, false), add(&mut p, 0, LayerKind::Paint, None, false), add(&mut p, 1, LayerKind::Paint, None, false));
        p.layer_mut(rust).unwrap().visible = false;
        let before = p.clone();
        let view = solo_project(&p, rust, false).unwrap();
        assert!(view.layers[&rust].visible, "the soloed layer shows even with its eye off");
        assert!(!view.layers[&below].visible, "the others are hidden");
        assert!(view.layers[&other_set].visible, "another texture set keeps its layers");
        assert_eq!(p, before, "the project itself is untouched");
    }

    #[test]
    fn solo_keeps_the_folders_around_a_layer_and_everything_inside_a_soloed_folder() {
        let mut p = project();
        let outside = add(&mut p, 0, LayerKind::Paint, None, false);
        let folder = add(&mut p, 0, LayerKind::Group { children: Vec::new(), collapsed: false }, None, false);
        let inner = add(&mut p, 0, LayerKind::Paint, Some(folder), true);
        let sibling = add(&mut p, 0, LayerKind::Paint, Some(inner), false);
        let view = solo_project(&p, inner, false).unwrap();
        assert!(view.layers[&folder].visible && view.layers[&inner].visible);
        assert!(!view.layers[&sibling].visible && !view.layers[&outside].visible);
        let whole = solo_project(&p, folder, false).unwrap();
        assert!(whole.layers[&inner].visible && whole.layers[&sibling].visible, "a soloed folder shows its contents");
        assert!(!whole.layers[&outside].visible);
    }

    #[test]
    fn ignoring_the_mask_disables_it_on_the_layer_and_its_folders_only() {
        let mut p = project();
        let folder = add(&mut p, 0, LayerKind::Group { children: Vec::new(), collapsed: false }, None, false);
        let inner = add(&mut p, 0, LayerKind::Paint, Some(folder), true);
        let other = add(&mut p, 0, LayerKind::Paint, None, false);
        for id in [folder, inner, other] {
            masked(&mut p, id);
        }
        let kept = solo_project(&p, inner, false).unwrap();
        assert!(kept.layers[&inner].mask.as_ref().unwrap().enabled, "without the flag the mask stays");
        let view = solo_project(&p, inner, true).unwrap();
        assert!(!view.layers[&inner].mask.as_ref().unwrap().enabled && !view.layers[&folder].mask.as_ref().unwrap().enabled);
        assert!(view.layers[&other].mask.as_ref().unwrap().enabled, "a hidden layer's mask is not touched");
        assert!(p.layers[&inner].mask.as_ref().unwrap().enabled, "the project keeps its mask");
    }

    #[test]
    fn an_instance_gets_its_own_disabled_mask_instead_of_changing_its_source() {
        let mut p = project();
        let source = add(&mut p, 0, LayerKind::Paint, None, false);
        masked(&mut p, source);
        let instance = add(&mut p, 1, LayerKind::Instance { source }, None, false);
        let view = solo_project(&p, instance, true).unwrap();
        assert!(!view.layers[&instance].mask.as_ref().unwrap().enabled);
        assert!(view.layers[&source].mask.as_ref().unwrap().enabled, "the source keeps its mask");
    }

    #[test]
    fn a_soloed_instance_of_a_folder_keeps_the_folders_contents_drawn() {
        let mut p = project();
        let folder = add(&mut p, 0, LayerKind::Group { children: Vec::new(), collapsed: false }, None, false);
        let inner = add(&mut p, 0, LayerKind::Paint, Some(folder), true);
        let instance = add(&mut p, 0, LayerKind::Instance { source: folder }, None, false);
        let view = solo_project(&p, instance, false).unwrap();
        assert!(view.layers[&instance].visible && view.layers[&folder].visible && view.layers[&inner].visible);
    }

    #[test]
    fn a_layer_in_no_stack_gives_nothing() {
        assert!(solo_project(&project(), LayerId(99), true).is_none());
    }
}
