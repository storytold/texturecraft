//! Mask and effect commands (the layer context menu's mask section and "Add <effect>").

use serde_json::{Value, json};
use texturecraft_core::{
    AxisBlend, BakedMap, BlendMode, Channel, ChannelKind, ColorSelection, CompareOp, CompareSource, CurvatureDetail, CurvatureMode, Effect, EffectKind, Filter,
    Generator, GeneratorGrayscale, GeneratorKind, KindControls, LayerId, Levels, LightParameters, Mask, MaskBackground, Procedural, ProceduralKind, Project,
    Sampling, SectionParameters, Source, TextureBlend, WearParameters,
};

use crate::anchored_micro_edit::AnchoredMicroEdit;
use crate::commands::{CommandSpec, always, bad, blend_param, boolp, channel_param, color_param, f32p, has_layer, layer_param, strp, u64p};
use crate::distance_params::distance_from;
use crate::generator_input::GeneratorInput;
use crate::layer_cmds::{projection_from, projection_from_fields};
use crate::mask_owner::MaskOwner;
use crate::micro_params::{MICRO_HEIGHT_INPUT, MICRO_NORMAL_INPUT, micro_from, swapped_micro};
use crate::session::StackTarget;
use crate::{EngineError, Result, Session, cmd};

/// The `filter` id of a filter effect with no filter picked
pub const EMPTY_FILTER: &str = "none";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("mask.add.white", "Add white mask", ["Layers", "Mask"], None, "{layer?}", has_layer, |s, p| add_mask(s, p, MaskBackground::White)),
        cmd!("mask.add.black", "Add black mask", ["Layers", "Mask"], None, "{layer?}", has_layer, |s, p| add_mask(s, p, MaskBackground::Black)),
        cmd!(
            "mask.add.colorSelection",
            "Add mask with color selection",
            ["Layers", "Mask"],
            None,
            "{layer?, color?: a first ID color as [r,g,b] linear or \"#rrggbb\" sRGB (default: none, pick them with effect.colorSelection.pick), tolerance?} → {effect} (in the layer's mask, a new black one if it has none; selected)",
            has_layer,
            add_color_mask
        ),
        cmd!("mask.toggle", "Toggle mask", ["Layers", "Mask"], None, "{layer?, enabled?} — defaults to the selected layer", always, toggle),
        cmd!("mask.invertBackground", "Invert mask background", ["Layers", "Mask"], None, "{layer?}", has_layer, invert_background),
        cmd!("mask.invert", "Invert mask", ["Layers", "Mask"], None, "{layer?}", has_layer, invert),
        cmd!("mask.clear", "Clear mask", ["Layers", "Mask"], None, "{layer?}", has_layer, clear),
        cmd!("mask.remove", "Remove mask", ["Layers", "Mask"], None, "{layer?}", has_layer, remove),
        cmd!("effect.add.paint", "Add paint", ["Layers", "Effects"], None, "{layer?, target?: mask|content, name?}", has_layer, |s, p| push_effect(
            s,
            p,
            EffectKind::Paint,
            BlendMode::Normal
        )),
        cmd!(
            "effect.add.fill",
            "Add fill",
            ["Layers", "Effects"],
            None,
            "{layer?, target?: mask|content, name?, mask: value?: 0..1 | image?: resource name | procedural?: kind, scale?, seed?, contrast?, balance?, invert?, resolution?: auto|textureSet|uvTile|custom, customResolution?, resolutionFactor?: -3..3, tile?: {Tile Generator settings in camelCase, as project.inspect shows them}, tileDefaults?: true | anchor?: anchor effect id, anchorChannel?: channel (height); content: color? ([r,g,b] linear or \"#rrggbb\" sRGB), roughness?, metallic?, height?, opacity? (default: every layer channel); projection?; filtering?: bilinearHq|bilinearSharp|nearest; wrap?: repeat|repeatHorizontally|repeatVertically|none}",
            has_layer,
            add_fill
        ),
        cmd!(
            "effect.add.generator",
            "Add generator",
            ["Layers", "Effects"],
            None,
            "{layer?, name?, generator?: edgeWear|dirt|fiberGlassEdgeWear|thinAreas|curvature|ambientOcclusion|position|worldSpaceNormals|direction|3dDistance|3dLinearGradient|uvBorderDistance|grayscaleConversion|drippingRust|uvRandomColor|uvChecker, wearLevel?, wearContrast?, grungeAmount?, grungeScale?: 1..16, triplanar?, triplanarContrast?, edgesSmoothness?: 0..16, aoMasking?, curvatureWeight?, edgesMasking?, useCustomGrunge? (the image: effect.set input: grunge) (wear controls: edgeWear, fiberGlassEdgeWear and dirt), sectionInvert?, sectionBlur?: 0..16 (ambientOcclusion, worldSpaceNormals), sectionBalance? (not curvature), sectionContrast?, sectionBrightness?: -1..1 (not ambientOcclusion), rightLeft|topBottom|frontBack + Invert?|Blend?: normal|add|subtract|multiply|addSub|max|Weight? (position, worldSpaceNormals), useTexture?, textureOpacity?, textureInvert?, textureGrayscale?: desaturation|luma|average|max|min|red|green|blue, textureBlend?: normal|add|subtract|multiply|addSub|max|min|overlay|screen|softLight, textureScale?: 0.01..10, textureContrast?: -1..1, textureBrightness?: -1..1, textureTriplanar?, textureTriplanarContrast? (the texture itself: effect.set input: texture) (section controls: curvature, ambientOcclusion, position, worldSpaceNormals), useMicroDetails? (curvature, ambientOcclusion), microHeight?, microNormal? (the images: effect.set input: microHeight|microNormal, an image or a value; microHeight also an anchor point), microCurvatureType?: standard|sobel|smooth, microCurvatureIntensity?: 0..5 on curvature, 0..1 on the wear kinds, microHeightIntensity?: 0..10 (curvature and the wear kinds), microAoRadius?, microAoDepth? (ambientOcclusion and the wear kinds) (micro details), distanceX?, distanceY?, distanceZ? (the point across the mesh's bounds), distanceRadius?: 0..1.5, distanceOffset? (0 all black, 1 all white), distanceContrast? (3dDistance, which also takes invert), gradientStart?, gradientEnd?: [x, y, z] across the mesh's bounds, gradientBalance?, gradientContrast? (3dLinearGradient, which also takes invert), borderDistance? (1 = half a UV tile), borderSmoothness?, borderBalance?, borderContrast? (uvBorderDistance, which also takes invert), grayscaleType?: desaturation|luma|average|max|min|red|green|blue, grayscaleBalance?, grayscaleContrast?, grayscaleTile?: 1..16, grayscaleRotation?: 0..360, grayscaleSafeRotation? (grayscaleConversion, which also takes invert; its image: effect.set input: image), rustSpreading?, rustContrast?, spreadingSmoothness?, dripsIntensity?, dripsSmoothness?, dripsSamples?: 0..32, dripsAxis?: x|y|z (drippingRust, which also takes seed and invert), colorSourceMode?: random|customGradient, gradientColors?: [[r,g,b], ...], gradientPositions?: [0..1 per color], gradientQuantity?: 2..16 (uvRandomColor, which also takes seed), checkerColorMode?: uniform|random, checkerColor?, checkerGridTile?: 1..24, checkerGridOpacity?, uvBordersColor?, uvBordersWidth? (uvChecker, which also takes seed), lightHorizontal?: 0..360, lightVertical?: 0..360, highlightGlossiness?, highlightLevel?, lightAttenuation? (light controls: direction), level?, softness?, noise?, noiseScale?, offset?: -1..1 added to the result, direction? (instead keep edgeWear, dirt, curvature, ambientOcclusion, position and direction on the earlier model), invert?, blur?: 0..1, balance?: 0..1 (0.5 unchanged), contrast?: 0..1, resolution?: auto|textureSet|uvTile|custom, customResolution?, resolutionFactor?: -3..3 (1/8..x8), mode?: edges|cavities|both|raw, sizes?: [7 weights 0..1, finest first] (curvature and edgeWear), microAnchor?: anchor effect id below (its height as the Micro Height), microAmount?: 0..1 (the Micro Height's intensity, 0.5 = 1)}",
            has_layer,
            add_generator
        ),
        cmd!(
            "effect.add.levels",
            "Add levels",
            ["Layers", "Effects"],
            None,
            "{layer?, target?: mask|content, name?, inLow?, inHigh?, gamma?, outLow?, outHigh?, affects?: the one content channel it remaps (default baseColor; not normal maps, not on a mask), clamp?: values past the input handles stop at the output ends (default false)}",
            has_layer,
            add_levels
        ),
        cmd!(
            "effect.add.filter",
            "Add filter",
            ["Layers", "Effects"],
            None,
            "{layer?, target?: mask|content, name?, filter?: any id or label from filter.list (warp with warpMode: warp|directional|multiDirectional, blurSlope with quality?) or \"none\" for an empty filter that changes nothing, level?, radius?, amount?, hue?: turns -1..1, saturation?, lightness?, contrast?, luminosity?, shadows?|midtones?|highlights?: [r,g,b] -1..1, colors?: [[r,g,b] linear or \"#rrggbb\" sRGB, ...], positions?: [0..1, ...], quantity?: 2..10, grayscaleMode?: desaturation|luma|average|max|min, blur?, balance?, invert?, linear?, debug?, preset?: round|pinch|stairs|easeOut|ease|easeIn|peakOut|peak|peakIn|bump|doubleBump|triangular, orientation?: horizontal|vertical, color?, levels?, min?, max?, position?, range?, offset?, intensity?, scale?, seed?, angle?} — material finishes go on the content",
            has_layer,
            add_filter
        ),
        cmd!(
            "effect.add.compare",
            "Add compare mask",
            ["Layers", "Effects"],
            None,
            "{layer?, name?, left?: layersBelow|thisLayer|thisMask|constant (default thisLayer), op?: lesser|within|greater (default greater), right? (default layersBelow), compareChannel? (default height), value?: the constant, tolerance?, hardness? (default 0.9)}",
            has_layer,
            add_compare
        ),
        cmd!(
            "effect.add.anchor",
            "Add anchor point",
            ["Layers", "Effects"],
            None,
            "{layer?, target?: content|mask (default content), name? (default the layer's name, \"<layer> mask\" in a mask)} (on the content it records the stack up to this layer, in a mask the mask value at that point: every channel reads that gray; fills, masks and generators above read it: effect.add.fill {anchor}, effect.add.generator {microAnchor})",
            has_layer,
            add_anchor
        ),
        cmd!(
            "effect.set",
            "Edit effect",
            [],
            None,
            "{effect, visible?, blend?, opacity?, channel?: the content channel blend and opacity set (default: every channel), name?, filter?: id from filter.list (a filter effect only: swaps its type, keeping position, blend and opacity; its current id edits it in place; \"none\" empties it), channels?: {channelId: bool} a content effect's channels on or off, divider?: 1|10|100|1000 (warp, slope blur: divides the intensity), generator?: kind (a generator effect only: swaps its kind in place, keeping the shared settings and the inputs it reads; its own controls start from that kind's defaults), wear, section and light controls (see effect.add.generator), level?, softness?, noise?, noiseScale?, offset?, invert?, blur?, balance?, contrast?, resolution?, customResolution?, resolutionFactor?, input?: mesh map id, texture, grunge, microHeight, microNormal or image + one of inputImage?, inputValue?, inputAnchor?, inputBaked?: true (a generator), mode?, sizes?, microAnchor?: id|null (the Micro Height reads that anchor point; null empties it), microAmount?, value?, uniform?: 0..1 (a mask fill reads that value, whatever its source), procedural?: kind (a mask fill) with scale?, seed?, contrast?, balance?, invert?, resolution?, tile?, tileDefaults? (alone, they edit its pattern), image?: resource name (a fill's source), projection?: uv|triplanar|planar|spherical|cylindrical, scale?, rotation?, offset?, axis?, sharpness? (a fill effect's projection; while it reads a pattern, scale is the pattern's), anchor?, anchorChannel?, inLow?, inHigh?, gamma?, outLow?, outHigh?, affects?, clamp? (levels), tolerance?, left?, op?, right?, compareChannel?, hardness? (compare; value is its constant), colors?: [[r,g,b] linear, ...] (a color selection's ID colors, may be empty), output?, background?: transparent | black | white | gray, idMap?: an image resource name read as the ID map, null for the texture set's ID mesh map (a color selection; tolerance and hardness too), filter parameters (see effect.add.filter)}",
            has_layer,
            set
        ),
        cmd!("effect.remove", "Remove effect", [], None, "{effect?: the selected effect}", has_layer, remove_effect),
        cmd!(
            "effect.duplicate",
            "Duplicate effect",
            [],
            None,
            "{effect?: the selected effect} → {effect} (the copy, just above it, pixels included)",
            has_layer,
            duplicate_effect
        ),
        cmd!("effect.move", "Move effect", [], None, "{effect, index: position in its stack, 0 = bottom (applied first)}", has_layer, move_effect),
        cmd!(
            "effect.histogram",
            "Effect input histogram",
            [],
            None,
            "{effect, channel?: for content effects (default: the channel a levels effect remaps, else the layer's first channel), bins?: 2..256 (64)} → {target: mask|content, channel?, bins: [count per bucket over 0..1], luminance?, components?: [red, green, blue] (a color channel), compare?: {left, right} (a compare mask: what each side reads)}",
            always,
            histogram
        ),
    ]
}

fn with_mask<T>(s: &mut Session, p: &Value, cmd: &str, label: &str, f: impl FnOnce(&mut Session, LayerId) -> Result<T>) -> Result<T> {
    let id = layer_param(s, cmd, p)?;

    s.edit(label, |s| f(s, id))
}

fn add_mask(s: &mut Session, p: &Value, bg: MaskBackground) -> Result<Value> {
    let label = if bg == MaskBackground::White { "Add white mask" } else { "Add black mask" };
    with_mask(s, p, "mask.add", label, |s, id| {
        let l = s.doc.project.layer_mut(id)?;
        if l.mask.is_some() {
            return Err(EngineError::Other(format!("\"{}\" already has a mask", l.name)));
        }
        l.mask = Some(Mask::new(bg));
        s.target = StackTarget::Mask;
        Ok(())
    })?;
    Ok(Value::Null)
}

fn add_color_mask(s: &mut Session, p: &Value) -> Result<Value> {
    push_own_mask_effect(s, p, color_selection("mask.add.colorSelection", p)?, "Add mask with color selection", |_| Ok(()))
}

/// A colour selection with the `color` and `tolerance` params of command `cmd`
pub(crate) fn color_selection(cmd: &str, p: &Value) -> Result<EffectKind> {
    let colors: Vec<[f32; 3]> = color_param(cmd, p, "color")?.into_iter().collect();
    let tolerance = f32p(p, "tolerance").unwrap_or(0.1).clamp(0.0, 1.0);
    Ok(EffectKind::ColorSelection(ColorSelection::new(colors, tolerance)))
}

/// `p` aimed at the layer's mask, whatever `target` it names
pub(crate) fn mask_target(p: &Value) -> Value {
    let mut params = p.as_object().cloned().unwrap_or_default();
    params.insert("target".into(), json!("mask"));
    Value::Object(params)
}

/// The mask layer `id` draws with (an instance's source's unless it has its own).
fn mask_of(s: &mut Session, id: LayerId) -> Result<&mut Mask> {
    let l = s.doc.project.layer_mut(s.doc.project.mask_owner(id))?;
    let name = l.name.clone();
    l.mask.as_mut().ok_or_else(|| EngineError::Other(format!("\"{name}\" has no mask")))
}

fn toggle(session: &mut Session, params: &Value) -> Result<Value> {
    let enabled = with_mask(session, params, "mask.toggle", "Toggle mask", |session, layer| {
        let mask = mask_of(session, layer)?;
        mask.enabled = boolp(params, "enabled").unwrap_or(!mask.enabled);
        Ok(mask.enabled)
    })?;

    Ok(json!({ "enabled": enabled }))
}

fn invert_background(s: &mut Session, p: &Value) -> Result<Value> {
    with_mask(s, p, "mask.invertBackground", "Invert mask background", |s, id| {
        let m = mask_of(s, id)?;
        m.background = if m.background == MaskBackground::White { MaskBackground::Black } else { MaskBackground::White };
        Ok(())
    })?;
    Ok(Value::Null)
}

fn invert(s: &mut Session, p: &Value) -> Result<Value> {
    with_mask(s, p, "mask.invert", "Invert mask", |s, id| {
        let eid = s.doc.project.alloc_id();
        let m = mask_of(s, id)?;
        // Inverting twice cancels out.
        if m.effects.last().is_some_and(|e| matches!(e.kind, EffectKind::Filter(Filter::Invert))) {
            m.effects.pop();
        } else {
            m.effects.push(Effect::new(eid, EffectKind::Filter(Filter::Invert)));
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn clear(s: &mut Session, p: &Value) -> Result<Value> {
    with_mask(s, p, "mask.clear", "Clear mask", |s, id| {
        let m = mask_of(s, id)?;
        let ids: Vec<LayerId> = m.effects.drain(..).map(|e| e.id).collect();
        for e in ids {
            s.doc.pixels.masks.remove(&e);
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    with_mask(s, p, "mask.remove", "Remove mask", |s, id| {
        let m = s.doc.project.layer_mut(s.doc.project.mask_owner(id))?.mask.take().ok_or_else(|| EngineError::Other("the layer has no mask".into()))?;
        for e in m.effects {
            s.doc.pixels.masks.remove(&e.id);
        }
        s.target = StackTarget::Content;
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Whether `effect.add.*` targets the layer content (`target: "content"`) or its mask (the
/// default).
pub(crate) fn content_target(p: &Value, cmd: &str) -> Result<bool> {
    match strp(p, "target") {
        None | Some("mask") => Ok(false),
        Some("content") => Ok(true),
        Some(other) => Err(bad(cmd, format!("unknown target `{other}`: use content or mask"))),
    }
}

const MAX_EFFECT_NAME_CHARS: usize = 256;

/// A valid `name` param.
fn name_param(p: &Value) -> Option<&str> {
    strp(p, "name").filter(|n| !n.trim().is_empty() && n.chars().count() <= MAX_EFFECT_NAME_CHARS)
}

/// Add an effect to the selected layer's mask (creating a mask if there is none) or, with `target: "content"` or a content-only effect, to its content stack
fn push_effect(s: &mut Session, p: &Value, kind: EffectKind, blend: BlendMode) -> Result<Value> {
    let label = format!("Add {}", kind.name().to_ascii_lowercase());
    push_effect_as(s, p, kind, blend, &label)
}

/// [`push_effect`] as the undo step `label`
pub(crate) fn push_effect_as(s: &mut Session, p: &Value, kind: EffectKind, blend: BlendMode, label: &str) -> Result<Value> {
    push_effect_in(s, p, kind, blend, label, |_| Ok(()), MaskOwner::Shared)
}

/// An Add mask item's effect, `first` made in the same undo step: it goes in the layer's own mask, so an instance without one gets its own instead of changing its source's
pub(crate) fn push_own_mask_effect(s: &mut Session, p: &Value, kind: EffectKind, label: &str, first: impl FnOnce(&mut Session) -> Result<()>) -> Result<Value> {
    push_effect_in(s, &mask_target(p), kind, BlendMode::Normal, label, first, MaskOwner::Own)
}

/// [`push_effect_as`] into `owner`'s mask, making the change `first` in the same undo step before adding the effect
fn push_effect_in(
    s: &mut Session,
    p: &Value,
    mut kind: EffectKind,
    blend: BlendMode,
    label: &str,
    first: impl FnOnce(&mut Session) -> Result<()>,
    owner: MaskOwner,
) -> Result<Value> {
    let content = kind.content_only() == Some(true) || content_target(p, "effect.add")?;
    if kind.content_only().is_some_and(|content_only| content_only != content) {
        return Err(bad("effect.add", format!("{} effects go on masks, not on layer content", kind.name())));
    }

    if let EffectKind::Levels(levels) = &mut kind {
        levels_target(levels, content)?;
    }
    let new_effect = with_mask(s, p, "effect.add", label, |s, id| {
        first(s)?;
        let new_effect = s.doc.project.alloc_id();

        // An instance's content and (unless it has its own) mask belong to its source
        let id = match (content, owner) {
            (true, _) => s.doc.project.content_layer(id).map_or(id, |source| source.id),
            (false, MaskOwner::Own) => id,
            (false, MaskOwner::Shared) => s.doc.project.mask_owner(id),
        };
        let layer = s.doc.project.layer_mut(id)?;
        let background = if matches!(kind, EffectKind::Generator(_) | EffectKind::Fill { .. } | EffectKind::ColorSelection(_)) {
            MaskBackground::Black
        } else {
            MaskBackground::White
        };
        let mut effect = Effect::new(new_effect, kind);
        effect.blend = blend;
        if let Some(name) = name_param(p) {
            effect.name = name.to_string();
        }

        // A filter that changes only some channels (lighting, validation) starts with the other channels' pills off
        if content
            && let EffectKind::Filter(filter) = &effect.kind
            && let Some(on) = filter.starts_on()
        {
            effect.off_channels = Channel::ALL.into_iter().filter(|channel| !on.contains(channel)).collect();
        }
        if !content {
            layer.mask.get_or_insert_with(|| Mask::new(background)).effects.push(effect);
        } else if layer.is_group() {
            return Err(EngineError::Other(format!("\"{}\" is a folder: content effects go on paint and fill layers", layer.name)));
        } else {
            layer.content_effects.push(effect);
        }
        s.target = StackTarget::Effect(new_effect);
        Ok(new_effect)
    })?;
    Ok(json!({ "effect": new_effect.0 }))
}

/// A source reading anchor point `anchor` for layer `reader`: the anchor must sit on a layer
/// below the reader in the same texture set (the compositor walks bottom → top).
pub(crate) fn anchor_source(s: &Session, cmd: &str, reader: LayerId, anchor: LayerId, channel: Channel) -> Result<Source> {
    let owner = match s.doc.project.effect(anchor) {
        Some((owner, e)) if matches!(e.kind, EffectKind::Anchor) => owner,
        _ => return Err(bad(cmd, format!("{} is not an anchor point", anchor.0))),
    };

    let (ts, _, _) = s.doc.project.locate(reader).ok_or_else(|| bad(cmd, "layer not in a stack"))?;
    let order: Vec<LayerId> = s.doc.project.walk(ts).into_iter().map(|(id, _)| id).collect();
    let pos = |id: LayerId| order.iter().position(|x| *x == id);
    match (pos(owner), pos(reader)) {
        (Some(o), Some(r)) if o < r => Ok(Source::Anchor { anchor, channel, ramp: Vec::new() }),
        _ => Err(bad(cmd, "the anchor point must be on a layer below this one in the same texture set")),
    }
}

/// A source reading image resource `name`, which must be imported and decoded: the compositor
/// would otherwise fill with a flat gray without saying why
pub(crate) fn image_source(session: &Session, command: &str, name: &str) -> Result<Source> {
    if !session.images.contains_key(name) {
        return Err(bad(command, format!("no image resource `{name}` (import it first)")));
    }

    Ok(Source::image(name))
}

fn add_fill(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "effect.add.fill";
    let sampling = crate::layer_cmds::sampling_from(params, CMD, Sampling::default())?;
    let blend = blend_param(CMD, params, "blend")?.unwrap_or(BlendMode::Normal);
    let layer = layer_param(session, CMD, params)?;

    if content_target(params, CMD)? {
        let mut fill = crate::layer_cmds::fill_from(CMD, params)?;
        fill.sampling = sampling;
        if fill.sources.is_empty() {
            fill.sources = session.doc.project.layer(layer)?.channels.keys().map(|channel| (*channel, Source::uniform_of(*channel))).collect();
        }

        fill.projection = projection_from(params, CMD, None, || crate::warp_cmds::fitted_frame(&session.mesh))?;
        return push_effect(session, params, EffectKind::Material(fill), blend);
    }

    let source = if let Some(anchor) = u64p(params, "anchor") {
        let channel = channel_param(CMD, params, "anchorChannel")?.unwrap_or(Channel::Height);
        let reader = session.doc.project.mask_owner(layer);
        anchor_source(session, CMD, reader, LayerId(anchor), channel)?
    } else if let Some(name) = strp(params, "image") {
        image_source(session, CMD, name)?
    } else if let Some(name) = strp(params, "procedural") {
        let kind = ProceduralKind::parse(name).ok_or_else(|| bad(CMD, format!("unknown procedural `{name}`")))?;
        let mut pattern = Procedural::new(kind);
        crate::procedural_params::pattern_basics_from(params, &mut pattern);
        crate::procedural_params::pattern_settings_from(CMD, params, &mut pattern)?;

        Source::Procedural { pattern, low: [0.0; 3], high: [1.0; 3], ramp: Vec::new() }
    } else {
        Source::Uniform([f32p(params, "value").unwrap_or(1.0).clamp(0.0, 1.0); 3])
    };

    // A scalar controls pattern frequency, while a two-axis scale places the new mask
    let pattern_scale = matches!(source, Source::Procedural { .. }) && params.get("scale").is_some_and(Value::is_number);
    let projection = projection_from_fields(params, CMD, None, !pattern_scale, || crate::warp_cmds::fitted_frame(&session.mesh))?;

    push_effect(session, params, EffectKind::Fill { source, projection, sampling }, blend)
}

/// The parameters that drive the wear generators' earlier model
const EARLIER_WEAR_KEYS: [&str; 7] = ["level", "softness", "noise", "noiseScale", "offset", "mode", "sizes"];

/// The global adjustments, which the wear kinds don't have
const GLOBAL_KEYS: [&str; 3] = ["blur", "balance", "contrast"];

/// The wear controls `effect.add.generator` and `effect.set` take
const WEAR_KEYS: [&str; 11] = [
    "wearLevel",
    "wearContrast",
    "grungeAmount",
    "grungeScale",
    "triplanar",
    "triplanarContrast",
    "edgesSmoothness",
    "aoMasking",
    "curvatureWeight",
    "edgesMasking",
    "useCustomGrunge",
];

/// The input name of a wear generator's custom grunge image, beside the mesh map ids
const GRUNGE_INPUT: &str = "grunge";

/// The wear controls in `params` (a grunge input counts as one): they move a wear generator onto them, while level, softness, noise or offset keep it on the earlier model
fn wear_from(command: &str, params: &Value, generator: &mut Generator) -> Result<()> {
    let given = |keys: &[&str]| keys.iter().any(|key| params.get(*key).is_some());
    let wear_given = given(&WEAR_KEYS) || strp(params, "input") == Some(GRUNGE_INPUT);
    let Some(defaults) = WearParameters::defaults(generator.kind) else {
        return if wear_given { Err(bad(command, format!("{} has no wear controls", generator.kind.label()))) } else { Ok(()) };
    };

    let earlier_model = given(&EARLIER_WEAR_KEYS) || (generator.wear.is_none() && !wear_given && generator.kind != GeneratorKind::FiberGlassEdgeWear);
    if given(&GLOBAL_KEYS) && !earlier_model {
        return Err(bad(command, format!("{} has no global blur, balance or contrast on its wear controls", generator.kind.label())));
    }

    if let Some(unused) = WEAR_KEYS.iter().find(|key| params.get(**key).is_some() && !wear_key_applies(generator.kind, key)) {
        return Err(bad(command, format!("{} has no `{unused}`", generator.kind.label())));
    }

    if given(&EARLIER_WEAR_KEYS) {
        if wear_given {
            return Err(bad(command, "level, softness, noise and offset drive the earlier wear model: give them or the wear controls, not both"));
        }

        if generator.kind == GeneratorKind::FiberGlassEdgeWear {
            return Err(bad(command, format!("{} requires wear controls such as wearLevel or wearContrast", generator.kind.label())));
        }
        generator.wear = None;
        return Ok(());
    }

    if !wear_given {
        return Ok(());
    }

    // The offset belongs to the earlier model: kept, it would shift the wear controls' result unseen
    generator.offset = 0.0;
    let wear = generator.wear.get_or_insert(defaults);
    for (key, value) in [
        ("wearLevel", &mut wear.level),
        ("wearContrast", &mut wear.contrast),
        ("grungeAmount", &mut wear.grunge_amount),
        ("grungeScale", &mut wear.grunge_scale),
        ("triplanarContrast", &mut wear.triplanar_contrast),
        ("edgesSmoothness", &mut wear.edges_smoothness),
        ("aoMasking", &mut wear.ao_masking),
        ("curvatureWeight", &mut wear.curvature_weight),
        ("edgesMasking", &mut wear.edges_masking),
    ] {
        if let Some(given) = f32p(params, key) {
            *value = given;
        }
    }
    if let Some(triplanar) = boolp(params, "triplanar") {
        wear.triplanar = triplanar;
    }
    if let Some(custom_grunge) = boolp(params, "useCustomGrunge") {
        wear.custom_grunge = custom_grunge;
    }
    *wear = wear.clamped(generator.kind);
    Ok(())
}

/// Whether wear control `key` changes a generator of `kind`
pub fn wear_key_applies(kind: GeneratorKind, key: &str) -> bool {
    match key {
        "edgesMasking" => kind == GeneratorKind::Dirt,
        "edgesSmoothness" | "aoMasking" | "curvatureWeight" => kind != GeneratorKind::Dirt,
        "grungeScale" => kind != GeneratorKind::FiberGlassEdgeWear,
        _ => true,
    }
}

/// The parameters that drive the earlier model of the kinds with section or light controls
const EARLIER_SECTION_KEYS: [&str; 6] = ["level", "softness", "noise", "noiseScale", "offset", "direction"];

/// The section controls `effect.add.generator` and `effect.set` take, beside the axis keys made of [`AXIS_NAMES`] and [`AXIS_SUFFIXES`]
const SECTION_KEYS: [&str; 5] = ["sectionInvert", "sectionBlur", "sectionBalance", "sectionContrast", "sectionBrightness"];

/// The name of each axis in its keys, in the order of [`SectionParameters::axes`], and what follows it
const AXIS_NAMES: [&str; 3] = ["rightLeft", "topBottom", "frontBack"];
const AXIS_SUFFIXES: [&str; 3] = ["Invert", "Blend", "Weight"];

/// The Light generator's controls
const LIGHT_KEYS: [&str; 5] = ["lightHorizontal", "lightVertical", "highlightGlossiness", "highlightLevel", "lightAttenuation"];

/// The texture controls (Use Texture); the texture itself is the `texture` input
const TEXTURE_KEYS: [&str; 10] = [
    "useTexture",
    "textureOpacity",
    "textureInvert",
    "textureGrayscale",
    "textureBlend",
    "textureScale",
    "textureContrast",
    "textureBrightness",
    "textureTriplanar",
    "textureTriplanarContrast",
];

/// The input name of Grayscale Conversion's image, beside the mesh map ids
const GRAYSCALE_IMAGE_INPUT: &str = "image";

/// The input name of a generator's texture, beside the mesh map ids
const TEXTURE_INPUT: &str = "texture";

/// The section controls, the axis keys, the texture controls and the light controls `params` gives (a texture input counts as `texture`)
fn given_new_model_keys(params: &Value) -> Vec<String> {
    let axis_keys = AXIS_NAMES.iter().flat_map(|axis| AXIS_SUFFIXES.iter().map(move |suffix| format!("{axis}{suffix}")));
    let mut keys: Vec<String> = SECTION_KEYS
        .iter()
        .chain(TEXTURE_KEYS.iter())
        .chain(LIGHT_KEYS.iter())
        .map(|key| key.to_string())
        .chain(axis_keys)
        .filter(|key| params.get(key).is_some())
        .collect();
    if strp(params, "input") == Some(TEXTURE_INPUT) {
        keys.push(TEXTURE_INPUT.to_string());
    }
    keys
}

/// The section and light controls in `params`: they move a generator onto them, while level, softness, noise, offset or direction keep it on the earlier model
fn section_from(command: &str, params: &Value, generator: &mut Generator) -> Result<()> {
    let kind = generator.kind;
    let new_keys = given_new_model_keys(params);
    let earlier = EARLIER_SECTION_KEYS.iter().any(|key| params.get(*key).is_some());
    let section_defaults = SectionParameters::defaults(kind);
    let light_defaults = LightParameters::defaults(kind);
    if section_defaults.is_none() && light_defaults.is_none() {
        return match new_keys.first() {
            Some(key) => Err(bad(
                command,
                format!("{} has no `{key}`: only Curvature, Ambient Occlusion, Position, World Space Normals and Light have them", kind.label()),
            )),
            None => Ok(()),
        };
    }

    if let Some(unused) = new_keys.iter().find(|key| !section_key_applies(kind, key)) {
        return Err(bad(command, format!("{} has no `{unused}`", kind.label())));
    }

    let reads_direction = matches!(kind, GeneratorKind::Position | GeneratorKind::Direction);
    if params.get("direction").is_some() && !reads_direction {
        return Err(bad(command, format!("{} has no `direction`: only Position and Light point one way", kind.label())));
    }

    if earlier {
        if !new_keys.is_empty() {
            return Err(bad(
                command,
                "level, softness, noise, offset and direction drive the earlier model: give them or the section and light controls, not both",
            ));
        }
        if kind == GeneratorKind::WorldSpaceNormals {
            return Err(bad(
                command,
                "World Space Normals has section controls only (sectionBalance, topBottomWeight…), no level, softness, noise, offset or direction",
            ));
        }
        generator.section = None;
        generator.light = None;
        return Ok(());
    }

    let on_light = generator.light.is_some() || (light_defaults.is_some() && !new_keys.is_empty());
    if on_light && GLOBAL_KEYS.iter().any(|key| params.get(*key).is_some()) {
        return Err(bad(command, "Light has no global blur, balance or contrast on its light controls"));
    }

    if new_keys.is_empty() {
        return Ok(());
    }

    // Moving onto the reference's controls starts from the kind's defaults: the earlier model's values no longer apply
    if generator.on_earlier_model() {
        let defaults = Generator::new(kind);
        (generator.level, generator.softness, generator.noise_amount, generator.noise_scale) =
            (defaults.level, defaults.softness, defaults.noise_amount, defaults.noise_scale);
        (generator.offset, generator.direction) = (defaults.offset, defaults.direction);
        if light_defaults.is_some() {
            generator.global = defaults.global;
        }
    }

    if let Some(defaults) = light_defaults {
        let light = generator.light.get_or_insert(defaults);
        for (key, value) in [
            ("lightHorizontal", &mut light.horizontal),
            ("lightVertical", &mut light.vertical),
            ("highlightGlossiness", &mut light.glossiness),
            ("highlightLevel", &mut light.highlight_level),
            ("lightAttenuation", &mut light.attenuation),
        ] {
            if let Some(given) = f32p(params, key) {
                *value = given;
            }
        }
        *light = light.clamped();
        return Ok(());
    }

    let Some(defaults) = section_defaults else { return Ok(()) };
    let mut section = generator.section_controls().unwrap_or(defaults);
    for (key, value) in [
        ("sectionBlur", &mut section.blur),
        ("sectionBalance", &mut section.balance),
        ("sectionContrast", &mut section.contrast),
        ("sectionBrightness", &mut section.brightness),
    ] {
        if let Some(given) = f32p(params, key) {
            *value = given;
        }
    }

    if let Some(invert) = boolp(params, "sectionInvert") {
        section.invert = invert;
    }

    for (axis, name) in section.axes.iter_mut().zip(AXIS_NAMES) {
        if let Some(invert) = boolp(params, &format!("{name}Invert")) {
            axis.invert = invert;
        }
        if let Some(weight) = f32p(params, &format!("{name}Weight")) {
            axis.weight = weight;
        }
        let blend_key = format!("{name}Blend");
        if let Some(blend) = params.get(&blend_key) {
            let blend_name = blend.as_str().ok_or_else(|| bad(command, format!("`{blend_key}` must be a string")))?;
            axis.blend = AxisBlend::parse(blend_name).ok_or_else(|| bad(command, format!("unknown blending mode `{blend_name}`")))?;
        }
    }

    generator.section = Some(section.clamped(kind));
    texture_from(command, params, generator)
}

/// The texture controls in `params` (the generator is on its section controls already)
fn texture_from(command: &str, params: &Value, generator: &mut Generator) -> Result<()> {
    if !TEXTURE_KEYS.iter().any(|key| params.get(*key).is_some()) {
        return Ok(());
    }

    let mut texture = generator.texture.as_deref().cloned().unwrap_or_default();
    if let Some(enabled) = boolp(params, "useTexture") {
        texture.enabled = enabled;
    }
    for (key, value) in [
        ("textureOpacity", &mut texture.opacity),
        ("textureScale", &mut texture.scale),
        ("textureContrast", &mut texture.contrast),
        ("textureBrightness", &mut texture.brightness),
        ("textureTriplanarContrast", &mut texture.triplanar_contrast),
    ] {
        if let Some(given) = f32p(params, key) {
            *value = given;
        }
    }
    if let Some(invert) = boolp(params, "textureInvert") {
        texture.invert = invert;
    }
    if let Some(triplanar) = boolp(params, "textureTriplanar") {
        texture.triplanar = triplanar;
    }
    if let Some(value) = params.get("textureGrayscale") {
        let name = value.as_str().ok_or_else(|| bad(command, "`textureGrayscale` must be a string"))?;
        texture.grayscale = GeneratorGrayscale::parse(name).ok_or_else(|| bad(command, format!("unknown grayscale conversion `{name}`")))?;
    }
    if let Some(value) = params.get("textureBlend") {
        let name = value.as_str().ok_or_else(|| bad(command, "`textureBlend` must be a string"))?;
        texture.blend = TextureBlend::parse(name).ok_or_else(|| bad(command, format!("unknown blending mode `{name}`")))?;
    }
    generator.texture = Some(Box::new(texture.clamped()));
    Ok(())
}

/// Whether section or light control `key` changes a generator of `kind`
pub fn section_key_applies(kind: GeneratorKind, key: &str) -> bool {
    let axis_key = AXIS_NAMES.iter().any(|axis| key.strip_prefix(axis).is_some_and(|suffix| AXIS_SUFFIXES.contains(&suffix)));
    match key {
        "sectionInvert" | "sectionContrast" | TEXTURE_INPUT => SectionParameters::defaults(kind).is_some(),
        _ if TEXTURE_KEYS.contains(&key) => SectionParameters::defaults(kind).is_some(),
        "sectionBlur" => SectionParameters::has_blur(kind),
        "sectionBalance" => SectionParameters::has_balance(kind),
        "sectionBrightness" => SectionParameters::has_brightness(kind),
        _ if axis_key => SectionParameters::has_axes(kind),
        _ => LIGHT_KEYS.contains(&key) && LightParameters::defaults(kind).is_some(),
    }
}

/// Apply `params` to `generator`, using `command` in errors
fn generator_from(command: &str, params: &Value, generator: &mut Generator) -> Result<()> {
    wear_from(command, params, generator)?;
    section_from(command, params, generator)?;
    let earlier_keys: &[&str] = if WearParameters::defaults(generator.kind).is_some() { &EARLIER_WEAR_KEYS } else { &EARLIER_SECTION_KEYS };
    micro_from(command, params, generator, earlier_keys)?;
    distance_from(command, params, generator)?;
    if let Some(level) = f32p(params, "level") {
        generator.level = level.clamp(0.0, 1.0);
    }
    if let Some(softness) = f32p(params, "softness") {
        generator.softness = softness.clamp(0.0, 1.0);
    }
    if let Some(noise) = f32p(params, "noise") {
        generator.noise_amount = noise.clamp(0.0, Generator::MAX_NOISE_AMOUNT);
    }
    if let Some(offset) = f32p(params, "offset") {
        generator.offset = offset.clamp(-1.0, 1.0);
    }
    if let Some(noise_scale) = f32p(params, "noiseScale") {
        generator.noise_scale = noise_scale.clamp(Generator::MIN_NOISE_SCALE, Generator::MAX_NOISE_SCALE);
    }
    if let Some(seed) = u64p(params, "seed") {
        generator.seed = seed as u32;
    }
    if let Some(invert) = boolp(params, "invert") {
        generator.invert = invert;
    }
    if let Some(direction) = color_param(command, params, "direction")? {
        generator.direction = direction;
    }
    if let Some(blur) = f32p(params, "blur") {
        generator.global.blur = blur.clamp(0.0, 1.0);
    }
    if let Some(balance) = f32p(params, "balance") {
        generator.global.balance = balance.clamp(0.0, 1.0);
    }
    if let Some(contrast) = f32p(params, "contrast") {
        generator.global.contrast = contrast.clamp(0.0, 1.0);
    }
    crate::procedural_params::resolution_from(command, params, &mut generator.resolution)?;

    // A mode or sizes also give a curvature generator saved before detail sizes its detail
    let mode = params
        .get("mode")
        .map(|value| {
            let name = value.as_str().ok_or_else(|| bad(command, "`mode` must be a string"))?;
            CurvatureMode::parse(name).ok_or_else(|| bad(command, format!("unknown curvature mode `{name}`")))
        })
        .transpose()?;
    let weights = params.get("sizes").map(|value| size_weights(command, value)).transpose()?;
    if mode.is_none() && weights.is_none() {
        return Ok(());
    }

    if !matches!(generator.kind, GeneratorKind::Curvature | GeneratorKind::EdgeWear) {
        return Err(bad(command, "only curvature and edge wear generators have a mode and detail sizes"));
    }

    let detail = generator.detail.get_or_insert_with(CurvatureDetail::default);
    if let Some(mode) = mode {
        detail.mode = mode;
    }
    if let Some(weights) = weights {
        detail.weights = weights;
    }
    Ok(())
}

/// Read one weight (0..1) per size of [`CurvatureDetail::SIZES`], finest first
fn size_weights(command: &str, value: &Value) -> Result<[f32; CurvatureDetail::SIZES.len()]> {
    let invalid = || bad(command, format!("`sizes` takes {} numbers, finest first", CurvatureDetail::SIZES.len()));
    let items = value.as_array().filter(|items| items.len() == CurvatureDetail::SIZES.len()).ok_or_else(invalid)?;
    let mut weights = [0.0; CurvatureDetail::SIZES.len()];
    for (weight, item) in weights.iter_mut().zip(items) {
        *weight = item.as_f64().ok_or_else(invalid)?.clamp(0.0, 1.0) as f32;
    }
    Ok(weights)
}

/// An anchor point on the layer content by default, preserving previous placement, or in its mask with `target: "mask"`; named after the layer ("<layer> mask" in a mask) unless given a name
fn add_anchor(session: &mut Session, params: &Value) -> Result<Value> {
    // Params may be null or an object without a target: both default to the content
    let content = strp(params, "target").is_none() || content_target(params, "effect.add.anchor")?;
    let mut fields = params.as_object().cloned().unwrap_or_default();
    fields.insert("target".into(), json!(if content { "content" } else { "mask" }));
    if name_param(params).is_none() {
        let layer = session.doc.project.layer(layer_param(session, "effect.add.anchor", params)?)?;
        let suffix = if content { "" } else { " mask" };
        let mut name: String = layer.name.chars().take(MAX_EFFECT_NAME_CHARS - suffix.len()).collect();
        name.push_str(suffix);
        fields.insert("name".into(), json!(name));
    }
    push_effect(session, &Value::Object(fields), EffectKind::Anchor, BlendMode::Normal)
}

/// The generator input named by `input` and what replaces it (None: its mesh map again, or an empty texture or grunge slot), for a generator on layer `owner`
fn generator_input_param(session: &Session, owner: LayerId, params: &Value) -> Result<Option<(GeneratorInput, Option<Source>)>> {
    const CMD: &str = "effect.set";
    let Some(name) = params.get("input") else { return Ok(None) };
    let name = name.as_str().ok_or_else(|| bad(CMD, "`input` must be a mesh map id, `texture`, `grunge`, `microHeight`, `microNormal` or `image`"))?;
    let input = match name {
        TEXTURE_INPUT => GeneratorInput::Texture,
        GRUNGE_INPUT => GeneratorInput::Grunge,
        MICRO_HEIGHT_INPUT => GeneratorInput::MicroHeight,
        MICRO_NORMAL_INPUT => GeneratorInput::MicroNormal,
        GRAYSCALE_IMAGE_INPUT => GeneratorInput::Image,
        _ => GeneratorInput::Map(BakedMap::parse(name).ok_or_else(|| bad(CMD, format!("unknown input `{name}`")))?),
    };

    let image = strp(params, "inputImage");
    let value = f32p(params, "inputValue");
    let anchor = u64p(params, "inputAnchor");
    let baked = boolp(params, "inputBaked") == Some(true);
    let source = match (image, value, anchor, baked) {
        (Some(image), None, None, false) => {
            if !session.images.contains_key(image) {
                return Err(bad(CMD, format!("no image resource `{image}`")));
            }

            Some(Source::Image { name: image.to_string(), normal_format: Default::default() })
        }
        (None, Some(value), None, false) => Some(Source::Uniform([value.clamp(0.0, 1.0); 3])),
        (None, None, Some(_), false) if matches!(input, GeneratorInput::MicroNormal) => {
            return Err(bad(CMD, "the micro normal reads an image or a value, not an anchor point"));
        }
        (None, None, Some(anchor), false) => {
            let channel = channel_param(CMD, params, "anchorChannel")?.unwrap_or(Channel::Height);
            Some(anchor_source(session, CMD, owner, LayerId(anchor), channel)?)
        }
        (None, None, None, true) => None,
        _ => return Err(bad(CMD, "`input` takes exactly one of inputImage, inputValue, inputAnchor or inputBaked")),
    };
    Ok(Some((input, source)))
}

fn add_generator(session: &mut Session, params: &Value) -> Result<Value> {
    let kind = match strp(params, "generator") {
        Some(name) => GeneratorKind::parse(name).ok_or_else(|| bad("effect.add.generator", format!("unknown generator `{name}`")))?,
        None => GeneratorKind::EdgeWear,
    };
    let mut generator = Generator::new(kind);
    generator_from("effect.add.generator", params, &mut generator)?;
    let selected_layer = layer_param(session, "effect.add.generator", params)?;
    let reader = session.doc.project.mask_owner(selected_layer);
    if let Some(edit) = AnchoredMicroEdit::parse(session, "effect.add.generator", reader, params)? {
        edit.apply("effect.add.generator", &mut generator)?;
    }
    let blend = blend_param("effect.add.generator", params, "blend")?.unwrap_or(BlendMode::Normal);
    push_effect(session, params, EffectKind::Generator(generator), blend)
}

pub(crate) fn levels_from(cmd: &str, p: &Value, levels: &mut Levels) -> Result<()> {
    let slots = [
        ("inLow", &mut levels.in_low),
        ("inHigh", &mut levels.in_high),
        ("gamma", &mut levels.gamma),
        ("outLow", &mut levels.out_low),
        ("outHigh", &mut levels.out_high),
    ];
    for (key, slot) in slots {
        if let Some(value) = f32p(p, key) {
            let range = if key == "gamma" { Levels::GAMMA_RANGE } else { 0.0..=1.0 };
            *slot = value.clamp(*range.start(), *range.end());
        }
    }
    if let Some(clamp) = boolp(p, "clamp") {
        levels.clamp = clamp;
    }
    match channel_param(cmd, p, "affects")? {
        // Levels pass normal maps through unchanged (`per_unit`)
        Some(channel) if channel.kind() == ChannelKind::NormalMap => Err(bad(cmd, format!("levels cannot remap `{}`, a normal map", channel.id()))),
        Some(channel) => {
            levels.affects = Some(channel);
            Ok(())
        }
        None => Ok(()),
    }
}

/// A levels effect added to a mask remaps its one value; on the content it remaps Base color unless told otherwise
fn levels_target(levels: &mut Levels, content: bool) -> Result<()> {
    if content {
        levels.affects.get_or_insert(Channel::BaseColor);
    } else if levels.affects.is_some() {
        return Err(bad("effect.add", "a mask's levels remap its one value: leave out `affects`"));
    }

    Ok(())
}

fn add_levels(s: &mut Session, p: &Value) -> Result<Value> {
    let mut levels = Levels::default();
    levels_from("effect.add.levels", p, &mut levels)?;
    push_effect(s, p, EffectKind::Levels(levels), BlendMode::Normal)
}

fn add_filter(s: &mut Session, p: &Value) -> Result<Value> {
    let id = strp(p, "filter").unwrap_or("invert");

    // An empty filter changes nothing until one is picked into it
    let kind = if id == EMPTY_FILTER { EffectKind::Filter(Filter::Empty) } else { crate::filter_lib::build(id, p)? };

    // Library filters are named after their shelf entry (material finishes are fill effects)
    let mut p = p.clone();
    if name_param(&p).is_none()
        && let (Some(d), Some(o)) = (crate::filter_lib::find(id), p.as_object_mut())
    {
        o.insert("name".into(), json!(d.label));
    }
    push_effect(s, &p, kind, BlendMode::Normal)
}

fn add_compare(s: &mut Session, p: &Value) -> Result<Value> {
    let mut kind = new_compare();
    compare_from("effect.add.compare", p, &mut kind)?;
    push_effect(s, p, kind, BlendMode::Normal)
}

/// A new compare mask: this layer's height over the layers below it, hardness 0.9
pub(crate) fn new_compare() -> EffectKind {
    EffectKind::Compare {
        op: CompareOp::GreaterThan,
        value: 0.5,
        tolerance: 0.1,
        left: CompareSource::ThisLayer,
        right: CompareSource::LayersBelow,
        channel: Channel::Height,
        hardness: 0.9,
    }
}

/// Apply the compare parameters of `p` to `kind` (a compare effect)
fn compare_from(cmd: &str, p: &Value, kind: &mut EffectKind) -> Result<()> {
    let EffectKind::Compare { op, value, tolerance, left, right, channel, hardness } = kind else { return Ok(()) };
    let named_source = |key: &str| -> Result<Option<CompareSource>> {
        match strp(p, key) {
            None => Ok(None),
            Some(id) => CompareSource::from_id(id)
                .map(Some)
                .ok_or_else(|| bad(cmd, format!("unknown {key} source `{id}` (layersBelow, thisLayer, thisMask, constant)"))),
        }
    };
    if let Some(id) = strp(p, "op") {
        *op = CompareOp::from_id(id).ok_or_else(|| bad(cmd, format!("unknown op `{id}`")))?;
    }
    if let Some(source) = named_source("left")? {
        *left = source;
    }
    if let Some(source) = named_source("right")? {
        *right = source;
    }
    if let Some(compared) = channel_param(cmd, p, "compareChannel")? {
        *channel = compared;
    }
    for (key, slot) in [("value", value), ("tolerance", tolerance), ("hardness", hardness)] {
        if let Some(number) = f32p(p, key) {
            *slot = number.clamp(0.0, 1.0);
        }
    }
    Ok(())
}

/// The `effect` param, or the selected effect
pub(crate) fn effect_id(s: &Session, p: &Value, cmd: &str) -> Result<LayerId> {
    match (u64p(p, "effect"), s.target) {
        (Some(e), _) => Ok(LayerId(e)),
        (None, StackTarget::Effect(e)) => Ok(e),
        (None, _) => Err(bad(cmd, "`effect` is required (no effect is selected)")),
    }
}

/// Histogram bins when the call names none
const DEFAULT_HISTOGRAM_BINS: u64 = 64;

/// The fewest and most histogram bins a call takes
const HISTOGRAM_BINS_RANGE: (u64, u64) = (2, 256);

/// The distribution of the values entering an effect (what a levels effect remaps)
fn histogram(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effect.histogram";
    let effect = effect_id(s, p, CMD)?;
    let (owner, found) = s.doc.project.effect(effect).ok_or_else(|| bad(CMD, format!("no effect {}", effect.0)))?;
    let remapped = match &found.kind {
        EffectKind::Levels(levels) => levels.affects,
        _ => None,
    };
    let layer = s.doc.project.layer(owner)?;
    let content = layer.content_effects.iter().any(|candidate| candidate.id == effect);
    let channel = match channel_param(CMD, p, "channel")? {
        Some(channel) => channel,
        None => remapped.or_else(|| layer.channels.keys().next().copied()).unwrap_or(Channel::BaseColor),
    };
    let (fewest_bins, most_bins) = HISTOGRAM_BINS_RANGE;
    let bins = u64p(p, "bins").unwrap_or(DEFAULT_HISTOGRAM_BINS).clamp(fewest_bins, most_bins) as usize;
    let counts = s.effect_histogram(effect, channel, bins).ok_or_else(|| bad(CMD, "the texture set is not ready"))?;
    if !content {
        let mut out = json!({"target": "mask", "bins": counts.all});
        if let Some([left, right]) = s.compare_histograms(effect, bins) {
            out["compare"] = json!({"left": left, "right": right});
        }
        return Ok(out);
    }

    let mut out = json!({"target": "content", "channel": channel.id(), "bins": counts.all, "luminance": counts.luminance});
    if !counts.components.is_empty() {
        out["components"] = json!(counts.components);
    }
    Ok(out)
}

/// Edit visibility, blending, name, effect parameters, or a filter's type
pub(crate) fn set(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "effect.set";
    let effect = effect_id(session, params, CMD)?;
    let blend = blend_param(CMD, params, "blend")?;
    let generator_kind = params
        .get("generator")
        .map(|value| {
            let name = value.as_str().ok_or_else(|| bad(CMD, "`generator` must be a string"))?;
            GeneratorKind::parse(name).ok_or_else(|| bad(CMD, format!("unknown generator `{name}`")))
        })
        .transpose()?;
    let channel = channel_param(CMD, params, "channel")?;
    let on_mask = crate::effect_blending_cmds::on_mask(&session.doc.project, effect);
    if channel.is_some() && on_mask {
        return Err(bad(CMD, "a mask effect blends one channel: leave out `channel`"));
    }

    if on_mask && params.get("affects").is_some() {
        return Err(bad(CMD, "a mask's levels remap its one value: leave out `affects`"));
    }

    let switched = switched_channels(params)?;
    if !switched.is_empty() && on_mask {
        return Err(bad(CMD, "a mask effect has one value: leave out `channels`"));
    }

    // `anchor` picks a fill's anchor point, `anchorChannel` alone re-targets the current one
    let anchor = match session.doc.project.effect(effect) {
        Some((owner, Effect { kind: EffectKind::Fill { source, .. }, .. })) => {
            let (current, channel) = match source {
                Source::Anchor { anchor, channel, .. } => (Some(*anchor), *channel),
                _ => (None, Channel::Height),
            };
            let channel_param = channel_param(CMD, params, "anchorChannel")?;
            match u64p(params, "anchor").map(LayerId).or(current.filter(|_| channel_param.is_some())) {
                Some(anchor) => Some(anchor_source(session, CMD, owner, anchor, channel_param.unwrap_or(channel))?),
                None => None,
            }
        }
        _ => None,
    };

    let micro = match session.doc.project.effect(effect) {
        Some((owner, Effect { kind: EffectKind::Generator(_), .. })) => AnchoredMicroEdit::parse(session, CMD, owner, params)?,
        _ if params.get("microAnchor").is_some() || params.get("microAmount").is_some() => {
            return Err(bad(CMD, "`microAnchor` and `microAmount` edit generator effects only"));
        }

        _ => None,
    };

    // The input key identifies the generator slot to replace or restore
    let input = match session.doc.project.effect(effect) {
        Some((owner, Effect { kind: EffectKind::Generator(_), .. })) => generator_input_param(session, owner, params)?,
        _ => None,
    };

    // `image` points a fill at an imported image, checked before anything is edited
    let image = match (strp(params, "image"), session.doc.project.effect(effect)) {
        (Some(name), Some((_, Effect { kind: EffectKind::Fill { .. }, .. }))) => Some(image_source(session, CMD, name)?),
        _ => None,
    };

    let swapped = match strp(params, "filter") {
        Some(EMPTY_FILTER) => Some(emptied_filter(session, effect)?),
        Some(filter) if crate::filter_lib::find(filter).is_some_and(|def| current_filter_id(session, effect) == Some(def.id)) => None,
        Some(filter) => Some(swapped_filter(session, effect, filter, params)?),
        None => None,
    };
    let swaps_filter = swapped.is_some();
    let id_map = crate::color_selection_cmds::id_map_param(session, params)?;
    let swapped_generator = match strp(params, "generator") {
        Some(kind) => swapped_generator_kind(session, effect, kind)?,
        None => None,
    };

    let mesh = session.mesh.clone();
    session.edit("Edit effect", |session| {
        let edited = session.doc.project.effect_mut(effect).ok_or_else(|| bad(CMD, format!("no effect {}", effect.0)))?;
        if let Some(generator) = swapped_generator {
            edited.kind = EffectKind::Generator(generator);
        }
        if let Some((kind, label)) = swapped {
            if let EffectKind::Filter(old) = &edited.kind
                && (edited.name == old.label() || Some(edited.name.as_str()) == old.former_label())
            {
                edited.name = label.to_string();
            }
            edited.kind = kind;
        }

        if let Some(visible) = boolp(params, "visible") {
            edited.visible = visible;
        }
        for (channel, on) in &switched {
            if *on {
                edited.off_channels.remove(channel);
            } else {
                edited.off_channels.insert(*channel);
            }
        }
        crate::effect_blending_cmds::set_blending(edited, channel, blend, f32p(params, "opacity"));
        if let Some(name) = name_param(params) {
            edited.name = name.to_string();
        }

        match &mut edited.kind {
            EffectKind::Generator(generator) => {
                if let Some(kind) = generator_kind
                    && kind != generator.kind
                {
                    generator.kind = kind;
                    generator.detail = Generator::new(kind).detail;
                    generator.inputs.retain(|map, _| kind.inputs().contains(map));
                }
                generator_from(CMD, params, generator)?;
                match input {
                    Some((GeneratorInput::Map(map), source)) => {
                        if !generator.inputs_in_use().contains(&map) {
                            return Err(bad(CMD, format!("this generator does not read a `{}` input", map.id())));
                        }
                        match source {
                            Some(source) => generator.inputs.insert(map, source),
                            None => generator.inputs.remove(&map),
                        };
                    }

                    // The generator parser has validated texture support and migrated the section controls
                    Some((GeneratorInput::Texture, source)) => generator.texture.get_or_insert_default().source = source,
                    // generator_from refused the grunge on kinds without wear controls and moved the generator onto them
                    Some((GeneratorInput::Grunge, source)) => generator.grunge = source,
                    // generator_from refused micro details on kinds without them and created them
                    Some((GeneratorInput::MicroHeight, source)) => {
                        if let Some(micro) = generator.micro.as_mut() {
                            micro.height_source = source;
                        }
                    }
                    Some((GeneratorInput::Image, source)) => {
                        if generator.kind != GeneratorKind::GrayscaleConversion {
                            return Err(bad(CMD, "only Grayscale Conversion reads an `image` input"));
                        }

                        let mut grayscale = generator.grayscale_controls().unwrap_or_default();
                        grayscale.source = source;
                        generator.controls = Some(Box::new(KindControls::Grayscale(grayscale)));
                    }
                    Some((GeneratorInput::MicroNormal, source)) => {
                        if let Some(micro) = generator.micro.as_mut() {
                            micro.normal_source = source;
                        }
                    }
                    None => {}
                }
                if let Some(edit) = micro {
                    edit.apply(CMD, generator)?;
                }
            }
            _ if generator_kind.is_some() => return Err(bad(CMD, "`generator` requires a generator effect")),
            EffectKind::Levels(levels) => levels_from(CMD, params, levels)?,

            // The swapped filter was built with the parameters already
            EffectKind::Filter(_) if swaps_filter => {}
            EffectKind::Filter(filter) => crate::filter_lib::apply(params, filter, CMD)?,
            EffectKind::Fill { source, projection, .. } => {
                // `uniform` picks the source, so it outranks settings that would edit the current pattern
                let uniform = f32p(params, "uniform").filter(|_| strp(params, "procedural").is_none());
                if let Some(anchor) = anchor {
                    *source = anchor;
                } else if let Some(image) = image {
                    *source = image;
                } else if let Some(value) = uniform {
                    *source = Source::Uniform([value.clamp(0.0, 1.0); 3]);
                } else if let Some(pattern) = fill_pattern(CMD, params, source)? {
                    *source = pattern;
                } else if let Some(value) = f32p(params, "value")
                    && matches!(source, Source::Uniform(_) | Source::Anchor { .. } | Source::Image { .. })
                {
                    *source = Source::Uniform([value.clamp(0.0, 1.0); 3]);
                }

                // A scalar changes pattern frequency while an array places the projection
                let pattern_scale = matches!(source, Source::Procedural { .. }) && params.get("scale").is_some_and(Value::is_number);
                *projection = projection_from_fields(params, CMD, Some(&*projection), !pattern_scale, || crate::warp_cmds::fitted_frame(&mesh))?;
            }
            EffectKind::ColorSelection(selection) => crate::color_selection_cmds::apply_params(params, selection, id_map)?,
            kind @ EffectKind::Compare { .. } => compare_from(CMD, params, kind)?,
            _ => {}
        }

        Ok(())
    })?;

    Ok(Value::Null)
}

/// The pattern a mask fill reads after `params`: `procedural` picks one (a pattern of the kind it already reads keeps its settings), pattern settings alone edit the current one
fn fill_pattern(command: &str, params: &Value, current: &Source) -> Result<Option<Source>> {
    let (current, range) = match current {
        Source::Procedural { pattern, low, high, .. } => (Some(pattern), (*low, *high)),
        _ => (None, ([0.0; 3], [1.0; 3])),
    };
    let mut pattern = match strp(params, "procedural") {
        Some(name) => {
            let kind = ProceduralKind::parse(name).ok_or_else(|| bad(command, format!("unknown procedural `{name}`")))?;
            current.filter(|pattern| pattern.kind == kind).cloned().unwrap_or_else(|| Procedural::new(kind))
        }
        None => {
            let has_settings = crate::procedural_params::has_pattern_settings(params);
            let edits_pattern = has_settings
                || params.get("scale").is_some_and(Value::is_number)
                || ["seed", "contrast", "balance", "invert"].iter().any(|key| params.get(key).is_some());
            match current {
                Some(pattern) if edits_pattern => pattern.clone(),
                None if has_settings => {
                    return Err(bad(command, "the mask fill reads no pattern: give `procedural` with its settings"));
                }
                _ => return Ok(None),
            }
        }
    };

    crate::procedural_params::pattern_basics_from(params, &mut pattern);
    crate::procedural_params::pattern_settings_from(command, params, &mut pattern)?;

    Ok(Some(Source::Procedural { pattern, low: range.0, high: range.1, ramp: Vec::new() }))
}

/// The content channels `channels` switches on (true) or off for an effect, e.g. {"roughness": false}
fn switched_channels(params: &Value) -> Result<Vec<(Channel, bool)>> {
    const CMD: &str = "effect.set";
    let Some(given) = params.get("channels") else { return Ok(Vec::new()) };
    let given = given.as_object().ok_or_else(|| bad(CMD, "`channels` must be an object of channel ids to true (on) or false (off)"))?;
    given
        .iter()
        .map(|(id, on)| {
            let channel = Channel::parse(id).ok_or_else(|| bad(CMD, format!("unknown channel `{id}`")))?;
            let on = on.as_bool().ok_or_else(|| bad(CMD, format!("`channels.{id}` must be true or false")))?;
            Ok((channel, on))
        })
        .collect()
}

/// An empty filter and its label, to replace filter effect `effect`'s
fn emptied_filter(s: &Session, effect: LayerId) -> Result<(EffectKind, &'static str)> {
    match s.doc.project.effect(effect) {
        Some((_, Effect { kind: EffectKind::Filter(_), .. })) => Ok((EffectKind::Filter(Filter::Empty), Filter::Empty.label())),
        Some((_, other)) => Err(bad("effect.set", format!("`filter` changes filter effects only, not {}", other.kind.name().to_ascii_lowercase()))),
        None => Err(bad("effect.set", format!("no effect {}", effect.0))),
    }
}

/// The library id of the filter effect `effect` is, if it is one
fn current_filter_id(s: &Session, effect: LayerId) -> Option<&'static str> {
    match &s.doc.project.effect(effect)?.1.kind {
        EffectKind::Filter(filter) => crate::filter_lib::definition(filter).map(|def| def.id),
        _ => None,
    }
}

/// A generator of kind `name` at its defaults to replace generator effect `effect`, or None when it is that kind already
fn swapped_generator_kind(s: &Session, effect: LayerId, name: &str) -> Result<Option<Generator>> {
    const CMD: &str = "effect.set";
    let kind = GeneratorKind::parse(name).ok_or_else(|| bad(CMD, format!("unknown generator `{name}`")))?;
    match &s.doc.project.effect(effect).ok_or_else(|| bad(CMD, format!("no effect {}", effect.0)))?.1.kind {
        EffectKind::Generator(current) if current.kind == kind => Ok(None),
        EffectKind::Generator(current) => Ok(Some(swapped_kind(current, kind))),
        other => Err(bad(CMD, format!("`generator` changes generator effects only, not {}", other.name().to_ascii_lowercase()))),
    }
}

/// `current` as a `kind` generator: the settings every kind shares and the inputs `kind` reads stay, its own controls start from `kind`'s defaults
fn swapped_kind(current: &Generator, kind: GeneratorKind) -> Generator {
    let mut inputs = current.inputs.clone();
    inputs.retain(|map, _| kind.inputs().contains(map));
    Generator {
        level: current.level,
        softness: current.softness,
        noise_amount: current.noise_amount,
        noise_scale: current.noise_scale,
        seed: current.seed,
        // The offset belongs to the earlier model, which these kinds lack
        offset: if kind.has_no_earlier_model() { 0.0 } else { current.offset },
        direction: current.direction,
        invert: current.invert,
        global: current.global,
        resolution: current.resolution,
        inputs,
        grunge: current.grunge.clone().filter(|_| WearParameters::defaults(kind).is_some()),
        texture: current.texture.clone().filter(|_| SectionParameters::defaults(kind).is_some()),
        micro: swapped_micro(current.micro.as_deref(), kind),
        ..Generator::new(kind)
    }
}

/// Filter `id` built with `p` and its label, to replace filter effect `effect`
fn swapped_filter(s: &Session, effect: LayerId, id: &str, p: &Value) -> Result<(EffectKind, &'static str)> {
    const CMD: &str = "effect.set";
    let (owner, current) = s.doc.project.effect(effect).ok_or_else(|| bad(CMD, format!("no effect {}", effect.0)))?;
    if !matches!(current.kind, EffectKind::Filter(_)) {
        return Err(bad(CMD, format!("`filter` changes filter effects only, not {}", current.kind.name().to_ascii_lowercase())));
    }

    let def = crate::filter_lib::find(id).ok_or_else(|| bad(CMD, format!("unknown filter `{id}` (see filter.list)")))?;
    if !def.is_filter_effect() {
        return Err(bad(CMD, format!("{} is its own effect type: add it with effect.add.filter", def.label)));
    }

    let mask = s.doc.project.layer(owner)?.mask.as_ref();
    let on_mask = mask.is_some_and(|mask| mask.effects.iter().any(|masked| masked.id == effect));
    if on_mask && !def.mask {
        return Err(bad(CMD, format!("{} goes on layer content, not on masks", def.label)));
    }

    let kind = crate::filter_lib::build_for(CMD, id, p)?;
    Ok((kind, def.label))
}

/// The stack (a mask's or a layer's content effects) holding effect `eid`, and its position there.
fn stack_of(project: &mut Project, eid: LayerId) -> Option<(&mut Vec<Effect>, usize)> {
    project.layers.values_mut().find_map(|l| {
        l.mask.iter_mut().map(|m| &mut m.effects).chain(std::iter::once(&mut l.content_effects)).find_map(|list| {
            let i = list.iter().position(|e| e.id == eid)?;
            Some((list, i))
        })
    })
}

fn duplicate_effect(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effect.duplicate";
    let eid = effect_id(s, p, CMD)?;
    let new = s.edit("Duplicate effect", |s| {
        let new = s.doc.project.alloc_id();
        let (list, i) = stack_of(&mut s.doc.project, eid).ok_or_else(|| bad(CMD, format!("no effect {}", eid.0)))?;
        let mut e = list.get(i).cloned().ok_or_else(|| bad(CMD, format!("no effect {}", eid.0)))?;
        e.id = new;
        e.name = format!("{} copy", e.name);
        list.insert(i + 1, e);
        // Painted effects keep their pixels (copy-on-write tiles: cheap).
        let px = &mut s.doc.pixels;
        if let Some(img) = px.masks.get(&eid).cloned() {
            px.masks.insert(new, img);
        }
        for c in Channel::ALL {
            if let Some(img) = px.layer(eid, c).cloned() {
                px.layers.insert((new, c), img);
            }
        }
        Ok(new)
    })?;
    if s.target == StackTarget::Effect(eid) {
        s.target = StackTarget::Effect(new);
    }
    Ok(json!({ "effect": new.0 }))
}

fn move_effect(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "effect.move";
    let eid = effect_id(s, p, CMD)?;
    let index = u64p(p, "index").ok_or_else(|| bad(CMD, "`index` is required"))?;
    s.edit("Move effect", |s| {
        let (list, i) = stack_of(&mut s.doc.project, eid).ok_or_else(|| bad(CMD, format!("no effect {}", eid.0)))?;
        let e = list.remove(i);
        let to = usize::try_from(index).unwrap_or(usize::MAX).min(list.len());
        list.insert(to, e);
        Ok(())
    })?;
    Ok(Value::Null)
}

fn remove_effect(s: &mut Session, p: &Value) -> Result<Value> {
    let eid = effect_id(s, p, "effect.remove")?;
    s.edit("Remove effect", |s| {
        let content = match s.doc.project.effect(eid) {
            Some((owner, _)) => s.doc.project.layer(owner)?.content_effects.iter().any(|e| e.id == eid),
            None => return Err(bad("effect.remove", format!("no effect {}", eid.0))),
        };
        for l in s.doc.project.layers.values_mut() {
            for list in l.mask.iter_mut().map(|m| &mut m.effects).chain(std::iter::once(&mut l.content_effects)) {
                list.retain(|e| e.id != eid);
            }
        }
        s.doc.pixels.masks.remove(&eid);
        s.doc.pixels.layers.retain(|(id, _), _| *id != eid);
        if s.target == StackTarget::Effect(eid) {
            s.target = if content { StackTarget::Content } else { StackTarget::Mask };
        }
        Ok(())
    })?;
    Ok(Value::Null)
}
