//! The layer toolbar's Add mask items that add a mask with an effect in it (a bitmap, a height combination) and Add effect's colour selection

use serde_json::Value;
use texturecraft_core::{BlendMode, EffectKind, Sampling, Source};
use texturecraft_paint::alphas;

use crate::commands::{CommandSpec, bad, has_layer, strp};
use crate::mask_cmds::{color_selection, mask_target, new_compare, push_effect_as, push_own_mask_effect};
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "mask.add.bitmap",
            "Add bitmap mask",
            ["Layers", "Mask"],
            None,
            "{layer?, image: image resource name | alpha: library alpha id or name (its picture becomes the image resource alpha_<id>.png), projection? (see effect.add.fill)} → {effect} — a mask fill reading the image's gray, in the layer's mask (a new black one if it has none); selected",
            has_layer,
            add_bitmap
        ),
        cmd!(
            "mask.add.heightCombination",
            "Add mask with height combination",
            ["Layers", "Mask"],
            None,
            "{layer?} → {effect} — a compare mask (this layer's height greater than the layers below it, hardness 0.9) in the layer's mask (a new one if it has none); selected",
            has_layer,
            |s, p| push_own_mask_effect(s, p, new_compare(), "Add mask with height combination", |_| Ok(()))
        ),
        cmd!(
            "effect.add.colorSelection",
            "Add color selection",
            ["Layers", "Effects"],
            None,
            "{layer?, color?: a first ID color as [r,g,b] linear or \"#rrggbb\" sRGB (default: none), tolerance?} → {effect} — in the layer's mask (a new black one if it has none; colour selections go on masks only); selected",
            has_layer,
            |s, p| push_effect_as(s, &mask_target(p), color_selection("effect.add.colorSelection", p)?, BlendMode::Normal, "Add color selection")
        ),
    ]
}

/// A mask fill reading an image resource or a library alpha, in one undo step with the alpha's resource
fn add_bitmap(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "mask.add.bitmap";
    let (name, library) = match (strp(p, "image"), strp(p, "alpha")) {
        (Some(_), Some(_)) => return Err(bad(CMD, "give `image` or `alpha`, not both")),
        (Some(image), None) => {
            if !s.images.contains_key(image) {
                return Err(bad(CMD, format!("no image resource `{image}` (import it first)")));
            }

            (image.to_owned(), None)
        }
        (None, Some(alpha)) => {
            let alpha = alphas::find(alpha).ok_or_else(|| bad(CMD, format!("unknown alpha `{alpha}`")))?;
            (crate::fill_alpha::resource_name(alpha), Some(alpha))
        }
        (None, None) => return Err(bad(CMD, "`image` or `alpha` is required")),
    };

    let projection = crate::layer_cmds::projection_from(p, CMD, None, || crate::warp_cmds::fitted_frame(&s.mesh))?;
    let kind = EffectKind::Fill { source: Source::Image { name, normal_format: Default::default() }, projection, sampling: Sampling::default() };
    let add_library = |s: &mut Session| library.map_or(Ok(()), |alpha| s.add_library_alpha(alpha).map(|_| ()));

    push_own_mask_effect(s, p, kind, "Add bitmap mask", add_library)
}
