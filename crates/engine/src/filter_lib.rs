//! The filter library (the Filters shelf): adjustment, blur and distortion filters for
//! layer content and masks, and material finishes (content only). `effect.add.filter` adds one
//! by id; `filter.list` lists them with their default parameters.
//!
//! The public [Warp] and [Blur Slope] guides describe the controls and inputs.
//! TextureCraft defines its own command IDs, units, and sampling.
//!
//! [Warp]: https://experienceleague.adobe.com/en/docs/substance-3d-painter/using/effects/filters/standard-filters/warp
//! [Blur Slope]: https://experienceleague.adobe.com/en/docs/substance-3d-painter/using/effects/filters/standard-filters/blur-slope

use serde_json::{Value, json};
use texturecraft_core::{
    AnisotropicKuwahara, Bevel, BevelBlur, BevelCorner, BevelDirection, BevelSmooth, Channel, ColorMatch, ConversionMode, CurvePreset, DirectionalDistance,
    Effect, EffectKind, Fill, Filter, GradientInput, GrayscaleConversion, GrayscaleMode, HeightToNormal, HighpassSpace, LayerId, Levels, MaskOutline, Mirror,
    MirrorCorner, MirrorMode, OutlinePosition, POSTERIZE_LEVELS_MAX, POSTERIZE_LEVELS_MIN, PbrValidate, Procedural, ProceduralKind, Projection, SlopeBlending,
    Source, SourceColorMode, SourceSlopeBlur, SourceWarp, StylizedLighting, TargetColorMode, ThresholdMode, TileTransform, ValidationMode, WarpBlending,
    WarpMode, WarpSource, WarpSourceMode,
};

use crate::commands::{CommandSpec, always, bad, boolp, color_of, color_param, f32p, has_layer, strp, u64p};
use crate::filter_about::FilterAbout;
use crate::layer_cmds::FULL_TURN_DEGREES;
use crate::{Result, Session, cmd};

/// A filter of the library.
#[derive(Clone, Copy, Debug)]
pub struct FilterDef {
    pub id: &'static str,
    pub label: &'static str,
    /// Shelf group.
    pub group: &'static str,
    /// Usable on masks too (material finishes only fill layer content).
    pub mask: bool,
}

impl FilterDef {
    /// What it does and the words it is found by (its Attributes)
    pub fn about(&self) -> Option<FilterAbout> {
        crate::filter_about::of(self.id)
    }

    /// Whether it builds a filter effect (levels and material finishes are effect types of their own)
    pub fn is_filter_effect(&self) -> bool {
        matches!(build(self.id, &Value::Null), Ok(EffectKind::Filter(_)))
    }
}

/// The library entries that build filter effects, in shelf order, worked out once
pub fn filter_effects() -> &'static [&'static FilterDef] {
    static FILTER_EFFECTS: std::sync::LazyLock<Vec<&'static FilterDef>> =
        std::sync::LazyLock::new(|| FILTERS.iter().filter(|def| def.is_filter_effect()).collect());
    &FILTER_EFFECTS
}

const fn def(id: &'static str, label: &'static str, group: &'static str, mask: bool) -> FilterDef {
    FilterDef { id, label, group, mask }
}

pub const FILTERS: &[FilterDef] = &[
    def("hsl", "HSL Perceptive", "Adjustments", true),
    def("levels", "Levels", "Adjustments", true),
    def("contrastLuminosity", "Contrast Luminosity", "Adjustments", true),
    def("colorBalance", "Color Balance", "Adjustments", true),
    def("colorCorrect", "Color Correct", "Adjustments", true),
    def("gradientMap", "Gradient", "Adjustments", true),
    def("gradientCurve", "Gradient Curve", "Adjustments", true),
    def("gradientDynamic", "Gradient Dynamic", "Adjustments", true),
    def("posterize", "Posterize", "Adjustments", true),
    def("clamp", "Clamp", "Adjustments", true),
    def("smoothstep", "Smoothstep", "Adjustments", true),
    def("heightAdjust", "Height Adjust", "Adjustments", true),
    def("histogramScan", "Histogram Scan", "Adjustments", true),
    def("histogramRange", "Histogram Range", "Adjustments", true),
    def("histogramShift", "Histogram Shift", "Adjustments", true),
    def("invert", "Invert", "Adjustments", true),
    def("threshold", "Threshold", "Adjustments", true),
    def("grayscale", "Grayscale Conversion", "Adjustments", true),
    def("edgeDetect", "Edge Detect", "Adjustments", true),
    def("normalInvert", "Normal Invert", "Adjustments", false),
    def("bakedLighting", "Baked Lighting Stylized", "Adjustments", false),
    def("bakedLightingEnvironment", "Baked Lighting Environment", "Adjustments", false),
    def("replaceColor", "Replace Color", "Adjustments", false),
    def("pbrValidate", "PBR Validate (Metallic Roughness)", "Adjustments", false),
    def("heightToNormal", "Height To Normal", "Adjustments", false),
    def("colorMatch", "Color Match", "Adjustments", false),
    def("blur", "Blur", "Blur and sharpen", true),
    def("blurDirectional", "Blur Directional", "Blur and sharpen", true),
    def("blurSlope", "Blur Slope", "Blur and sharpen", true),
    def("sharpen", "Sharpen", "Blur and sharpen", true),
    def("highPass", "Highpass", "Blur and sharpen", true),
    def("fxaa", "FXAA (Anti-aliasing)", "Blur and sharpen", true),
    def("anisotropicKuwahara", "Anisotropic Kuwahara", "Blur and sharpen", true),
    def("bevel", "Bevel", "Blur and sharpen", true),
    def("bevelSmooth", "Bevel Smooth", "Blur and sharpen", true),
    def("directionalDistance", "Directional Distance", "Blur and sharpen", true),
    def("glow", "Glow", "Blur and sharpen", true),
    def("dropShadow", "Drop Shadow", "Blur and sharpen", true),
    def("maskOutline", "Mask Outline", "Blur and sharpen", true),
    def("warp", "Warp", "Distort", true),
    def("mosaic", "Pixelate", "Distort", true),
    def("mirror", "Mirror", "Distort", true),
    def("transform", "Transform", "Distort", true),
    def("matFinishGrind", "MatFinish Grind", "Material finish", false),
    def("matFinishRaw", "MatFinish Raw", "Material finish", false),
    def("matFinishSanded", "MatFinish Sanded", "Material finish", false),
    def("matFinishHammered", "MatFinish Hammered", "Material finish", false),
];

fn norm(s: &str) -> String {
    s.to_ascii_lowercase().replace([' ', '_', '-', '/'], "")
}

/// The library entry for an ID or label, ignoring case and separators.
pub fn find(id: &str) -> Option<&'static FilterDef> {
    let wanted = norm(id);
    FILTERS.iter().find(|def| norm(def.id) == wanted || norm(def.label) == wanted)
}

/// The catalog entry for an installed filter. Saved Warp and Blur Slope variants
/// from before the new modes have no catalog entry, even when their labels match.
pub fn definition(filter: &Filter) -> Option<&'static FilterDef> {
    match filter {
        Filter::Warp { .. } | Filter::DirectionalWarp { .. } | Filter::SlopeBlur { .. } => None,
        _ => find(filter.label()),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "effect.filter.restoreDefaults",
            "Restore defaults",
            [],
            None,
            "{effect?: filter effect id, default the selected effect} — restore its catalog defaults in one undo step, keeping name, channels and blending. Filters without a catalog entry and filters already at defaults return {changed: false}",
            has_layer,
            restore_defaults
        ),
        cmd!(
            "filter.list",
            "List filters",
            [],
            None,
            "{} — the filter library: id, label, group, whether masks take it, default parameters, what it does (description) and its tags",
            always,
            |_, _| Ok(Value::Array(FILTERS.iter().map(listed).collect()))
        ),
    ]
}

/// `filter.list`'s entry for `def`
fn listed(def: &FilterDef) -> Value {
    let about = def.about();
    json!({
        "id": def.id,
        "label": def.label,
        "group": def.group,
        "mask": def.mask,
        "defaults": build(def.id, &Value::Null).ok(),
        "description": about.map(|about| about.description),
        "tags": about.map(|about| about.tags),
    })
}

/// The selected effect ID and its filter's defaults, when that filter has a catalog entry.
fn filter_defaults(s: &Session, p: &Value) -> Result<(LayerId, Option<Filter>)> {
    const CMD: &str = "effect.filter.restoreDefaults";
    let effect = crate::mask_cmds::effect_id(s, p, CMD)?;
    let filter = match s.doc.project.effect(effect) {
        Some((_, Effect { kind: EffectKind::Filter(filter), .. })) => filter,
        Some(_) => return Err(bad(CMD, format!("effect {} is not a filter", effect.0))),
        None => return Err(bad(CMD, format!("no effect {}", effect.0))),
    };
    let defaults = definition(filter).and_then(|def| match build(def.id, &Value::Null) {
        Ok(EffectKind::Filter(defaults)) => Some(defaults),
        _ => None,
    });
    Ok((effect, defaults))
}

/// Puts a filter effect's parameters back to their library defaults in one undo step
fn restore_defaults(s: &mut Session, p: &Value) -> Result<Value> {
    let (effect, defaults) = filter_defaults(s, p)?;
    let current = s.doc.project.effect(effect).map(|(_, found)| &found.kind);
    let Some(defaults) = defaults.filter(|defaults| !matches!(current, Some(EffectKind::Filter(filter)) if filter == defaults)) else {
        return Ok(json!({"changed": false}));
    };

    s.edit("Restore defaults", |s| {
        if let Some(edited) = s.doc.project.effect_mut(effect) {
            edited.kind = EffectKind::Filter(defaults);
        }
        Ok(())
    })?;
    Ok(json!({"changed": true}))
}

/// A Color Correct tone that changes nothing: no contrast, no luminosity, saturation as it is
const NEUTRAL_TONE: [f32; 3] = [0.0, 0.0, 0.5];

/// Mid gray as a color picker shows it (sRGB 0.5), in linear RGB
pub const MID_GRAY: f32 = 0.214;

/// The two Channel Input and Extract Direction choices: the channel the filter sits on, or its image input
const CURRENT_CHANNEL: &str = "currentChannel";
const CUSTOM_INPUT: &str = "customInput";

/// A new glow's warm light, in linear RGB
const GLOW_COLOR: [f32; 3] = [0.871, 0.578, 0.188];

/// The Glow filter as the library adds it: a soft warm halo outside the shapes
pub const LIBRARY_GLOW: Filter = Filter::HaloGlow { on_mask: false, precise: false, width: 0.15, spread: 0.0, opacity: 1.0, color: GLOW_COLOR };

/// The Drop Shadow filter as the library adds it: a half-opaque black shadow cast downward
pub const LIBRARY_DROP_SHADOW: Filter = Filter::DropShadow { angle: 90.0, distance: 0.1, size: 0.2, spread: 0.0, opacity: 0.5, color: [0.0; 3] };

/// The Gradient Curve filter as the library adds it
pub const LIBRARY_GRADIENT_CURVE: Filter = Filter::GradientCurve { preset: CurvePreset::Round, invert: false, balance: 0.5, contrast: 0.0 };

/// The Gradient Dynamic filter as the library adds it: a horizontal ramp from a light gray
pub const LIBRARY_GRADIENT_DYNAMIC: Filter = Filter::GradientDynamic { vertical: false, position: 0.0, color: [0.6; 3] };

/// The Baked Lighting Environment filter as the library adds it: the environment as it stands, full ambient occlusion
pub const LIBRARY_ENVIRONMENT_LIGHTING: Filter =
    Filter::EnvironmentLighting { horizontal: 0.0, vertical: 0.0, exposure: 0.0, height_intensity: 4.0, ambient_occlusion: 1.0, specular_occlusion: 0.0 };

pub use crate::stylized_lighting_params::{MAX_CAVITY_SMOOTHNESS, MAX_DIELECTRIC_REFLECTANCE, MAX_HEIGHT_TO_NORMAL, MAX_LIGHT_INTENSITY};

/// A Seed slider's end; any seed can be typed
const SEED_SLIDER_MAX: f32 = 999.0;

/// The Tiling range of a warp's or slope blur's source
pub const SOURCE_TILING_MIN: f32 = WarpSource::TILING_MIN;
pub const SOURCE_TILING_MAX: f32 = WarpSource::TILING_MAX;

/// The Source Tiling slider range of a noise-driven warp or blur
const NOISE_TILING_MIN: f32 = 0.5;
const NOISE_TILING_MAX: f32 = 64.0;

/// A blur Intensity slider's end, in the reference workflow's unit (see [`Filter::BLUR_INTENSITY`])
pub const BLUR_INTENSITY_SLIDER_MAX: f32 = 16.0;

/// The widest radius or width a filter takes, in texels at 2K; its slider ends at [`TEXELS_SLIDER_MAX`]
const TEXELS_MAX: f32 = 256.0;
const TEXELS_SLIDER_MAX: f32 = 128.0;

/// Slider ends where a larger value can still be typed
const SHARPEN_SLIDER_MAX: f32 = 4.0;
const GLOW_AMOUNT_SLIDER_MAX: f32 = 5.0;
pub const SKY_INTENSITY_SLIDER_MAX: f32 = 2.0;
const EDGE_WIDTH_SLIDER_MAX: f32 = 64.0;
const MOSAIC_PIXELS_SLIDER_MAX: f32 = 128.0;

/// The largest values that can be typed past those slider ends
const SHARPEN_AMOUNT_MAX: f32 = 10.0;
const GLOW_AMOUNT_MAX: f32 = 10.0;
const SKY_INTENSITY_MAX: f32 = 10.0;

/// The largest mosaic cell, in pixels
const MOSAIC_SIZE_MAX: f32 = 2048.0;

/// Height Adjust's largest Multiply
const HEIGHT_MULTIPLY_MAX: f32 = 10.0;

/// Height Adjust's Offset reaches this far either way, as the reference's does
const HEIGHT_OFFSET_MAX: f32 = 10.0;

/// Distance Bevel's largest Smoothing
pub const DISTANCE_BEVEL_SMOOTHING_MAX: f32 = Bevel::SMOOTHING_MAX;

/// Bevel Smooth's and Directional Distance's largest Distance, as a share of the texture
pub const SMOOTH_DISTANCE_MAX: f32 = 0.3;

/// Bevel Smooth's largest Smoothing
pub const BEVEL_SMOOTHING_MAX: f32 = BevelSmooth::SMOOTHING_MAX;

/// The most colors `colors` takes at once
const GRADIENT_COLORS_GIVEN_MAX: usize = 16;

/// The fewest colors a gradient's Color Quantity uses
pub const GRADIENT_QUANTITY_MIN: u8 = 2;

/// Stylized lighting's Sun Intensity end
pub const SUN_INTENSITY_MAX: f32 = 10.0;
/// Environment lighting's Height Intensity end
const HEIGHT_INTENSITY_MAX: f32 = 8.0;

/// The Gradient filter as the library adds it: black, mid gray and white at 0, 0.5 and 1 (the other seven colors wait white at 1)
fn library_gradient() -> Filter {
    let quantity = 3;
    let mut colors = vec![[0.0; 3], [MID_GRAY; 3]];
    colors.resize(usize::from(Filter::MAX_GRADIENT_COLORS), [1.0; 3]);
    let mut positions = vec![0.0, 0.5];
    positions.resize(colors.len(), 1.0);
    Filter::GradientMap { colors, positions, quantity: Some(quantity), input: GradientInput::LIBRARY, linear: true, debug: false }
}

/// The Baked Lighting Stylized filter as the library adds it: a sun 45° up in front, full sky light, no diffuse AO
pub fn library_baked_lighting() -> Filter {
    Filter::BakedLighting { azimuth: 0.0, elevation: 45.0, light: 1.0, ambient: 1.0, occlusion: 0.0, stylized: Some(Box::new(StylizedLighting::library())) }
}

/// The effect for library filter `id` with its defaults, then the parameters in `p`
pub fn build(id: &str, p: &Value) -> Result<EffectKind> {
    build_for("effect.add.filter", id, p)
}

/// The effect for library filter `id` with its defaults, then the parameters in `p`; errors name `cmd` (`effect.set` swaps a filter with it)
pub(crate) fn build_for(cmd: &str, id: &str, p: &Value) -> Result<EffectKind> {
    let def = find(id).ok_or_else(|| bad(cmd, format!("unknown filter `{id}` (see filter.list)")))?;
    let mut filter = match def.id {
        "invert" => Filter::Invert,
        "threshold" => Filter::Threshold { level: 0.01, mode: ThresholdMode::Greater },
        "blur" => Filter::Blur { radius: 0.3 * Filter::BLUR_INTENSITY, wrap: false },
        "sharpen" => Filter::Sharpen { amount: 1.0 },
        "hsl" => Filter::Hsl { hue: 0.0, saturation: 0.0, lightness: 0.0, perceptive: true },
        "contrastLuminosity" => Filter::ContrastLuminosity { contrast: 0.0, luminosity: 0.0 },
        "colorCorrect" => Filter::ColorCorrect { shadows: NEUTRAL_TONE, midtones: NEUTRAL_TONE, highlights: NEUTRAL_TONE },
        "colorBalance" => Filter::ColorBalance { shadows: [0.0; 3], midtones: [0.0; 3], highlights: [0.0; 3] },
        "gradientMap" => library_gradient(),
        "gradientCurve" => LIBRARY_GRADIENT_CURVE,
        "gradientDynamic" => LIBRARY_GRADIENT_DYNAMIC,
        "posterize" => Filter::Posterize { levels: 8.0, alpha: false },
        "clamp" => Filter::Clamp { min: 0.0, max: 1.0 },
        "smoothstep" => Filter::Smoothstep { min: 0.0, max: 1.0 },
        "heightAdjust" => Filter::HeightAdjust { invert: false, offset: 0.0, multiply: 1.0 },
        "histogramScan" => Filter::HistogramScan { position: 0.5, contrast: 0.0, invert_position: false },
        "histogramRange" => Filter::HistogramRange { range: 0.5, position: 0.5 },
        "histogramShift" => Filter::HistogramShift { offset: 0.0 },
        "warp" => Filter::SourceWarp(SourceWarp::LIBRARY),
        "blurDirectional" => Filter::DirectionalBlur { radius: 16.0, angle: 0.0 },
        "blurSlope" => Filter::SourceSlopeBlur(SourceSlopeBlur::LIBRARY),
        "highPass" => Filter::HighPass { radius: 3.0 * Filter::BLUR_INTENSITY, space: HighpassSpace::Working, alpha: false },
        "edgeDetect" => Filter::EdgeDetect { width: 2.0 },
        "grayscale" => Filter::GrayscaleConversion(GrayscaleConversion::LIBRARY),
        "normalInvert" => Filter::NormalInvert,
        "bevel" => Filter::DistanceBevel(Bevel::LIBRARY),
        "bevelSmooth" => Filter::BevelSmooth(BevelSmooth::LIBRARY),
        "directionalDistance" => Filter::DirectionalDistance(DirectionalDistance::LIBRARY),
        "mosaic" => Filter::Mosaic { size: Filter::pixel_size(32.0), alpha: true },
        "fxaa" => Filter::Fxaa { alpha: false },
        "anisotropicKuwahara" => Filter::AnisotropicKuwahara(AnisotropicKuwahara::LIBRARY),
        "pbrValidate" => Filter::PbrValidate(PbrValidate::LIBRARY),
        "heightToNormal" => Filter::HeightToNormal(HeightToNormal::LIBRARY),
        "colorMatch" => Filter::ColorMatch(ColorMatch::LIBRARY),
        "mirror" => Filter::Mirror(Mirror::LIBRARY),
        "transform" => Filter::Transform(TileTransform::LIBRARY),
        "maskOutline" => Filter::MaskOutline(MaskOutline::LIBRARY),
        "glow" => LIBRARY_GLOW,
        "dropShadow" => LIBRARY_DROP_SHADOW,
        "replaceColor" => Filter::ReplaceColor { from: [0.5, 0.05, 0.05], to: [0.05, 0.15, 0.5], tolerance: 0.2, softness: 0.15 },
        "bakedLighting" => library_baked_lighting(),
        "bakedLightingEnvironment" => LIBRARY_ENVIRONMENT_LIGHTING,
        "levels" => {
            let mut levels = Levels::default();
            crate::mask_cmds::levels_from(cmd, p, &mut levels)?;
            return Ok(EffectKind::Levels(levels));
        }
        finish => return mat_finish(finish).map(EffectKind::Material).ok_or_else(|| bad(cmd, format!("unknown filter `{id}`"))),
    };

    // A new gradient given its colors ramps through all of them, or the `quantity` given, evenly unless places are given too
    if let Filter::GradientMap { positions, quantity, .. } = &mut filter
        && p.get("colors").is_some()
    {
        *quantity = None;
        if p.get("positions").is_none() {
            positions.clear();
        }
    }

    apply(p, &mut filter, cmd)?;
    if let Filter::GradientMap { positions, quantity: Some(count), .. } = &mut filter
        && p.get("colors").is_some()
        && p.get("positions").is_none()
    {
        let last = f32::from(*count - 1);
        *positions = (0..*count).map(|index| f32::from(index) / last).collect();
    }

    Ok(EffectKind::Filter(filter))
}

/// A material finish: procedural roughness and height (tri-planar, so it is seamless).
fn mat_finish(id: &str) -> Option<Fill> {
    let (kind, scale, rough, height) = match id {
        "matFinishGrind" => (ProceduralKind::Brushed, 6.0, [0.22, 0.42], 0.012),
        "matFinishRaw" => (ProceduralKind::Clouds, 8.0, [0.45, 0.75], 0.02),
        "matFinishSanded" => (ProceduralKind::Noise, 3.0, [0.38, 0.58], 0.008),
        "matFinishHammered" => (ProceduralKind::Hammered, 6.0, [0.25, 0.45], 0.08),
        _ => return None,
    };

    let pattern = Procedural { scale, ..Procedural::new(kind) };
    let mut fill = Fill { projection: Projection::triplanar(), ..Fill::default() };
    fill.sources.insert(Channel::Roughness, Source::Procedural { pattern: pattern.clone(), low: [rough[0]; 3], high: [rough[1]; 3], ramp: Vec::new() });
    fill.sources.insert(Channel::Height, Source::Procedural { pattern, low: [-height; 3], high: [height; 3], ramp: Vec::new() });
    Some(fill)
}

/// Scalar parameters of a filter for editors: (param key, label, value, min, max); colors (color balance, gradient map) and the HSL hue have their own editors
pub fn scalar_params(filter: &Filter) -> Vec<(&'static str, &'static str, f32, f32, f32)> {
    match filter {
        Filter::Blur { radius, .. } => vec![("intensity", "Blur Intensity", *radius / Filter::BLUR_INTENSITY, 0.0, BLUR_INTENSITY_SLIDER_MAX)],
        Filter::Sharpen { amount } => vec![("amount", "Sharpen Intensity", *amount, 0.0, SHARPEN_SLIDER_MAX)],
        Filter::Threshold { level, .. } => vec![("level", "Threshold", *level, 0.0, 1.0)],
        Filter::Hsl { saturation, lightness, .. } => {
            vec![("saturation", "Saturation", *saturation, -1.0, 1.0), ("lightness", "Lightness", *lightness, -1.0, 1.0)]
        }
        Filter::ContrastLuminosity { contrast, luminosity } => {
            vec![("contrast", "Contrast", *contrast, -1.0, 1.0), ("luminosity", "Luminosity", *luminosity, -1.0, 1.0)]
        }
        Filter::Posterize { levels, .. } => vec![("levels", "Colors Amount", *levels, POSTERIZE_LEVELS_MIN, POSTERIZE_LEVELS_MAX)],
        Filter::Clamp { min, max } => vec![("min", "Min", *min, 0.0, 1.0), ("max", "Max", *max, 0.0, 1.0)],
        Filter::Smoothstep { min, max } => vec![("min", "Minimum", *min, 0.0, 1.0), ("max", "Maximum", *max, 0.0, 1.0)],
        Filter::HeightAdjust { offset, multiply, .. } => {
            vec![("offset", "Offset", *offset, -HEIGHT_OFFSET_MAX, HEIGHT_OFFSET_MAX), ("multiply", "Multiply", *multiply, 0.0, HEIGHT_MULTIPLY_MAX)]
        }
        Filter::HistogramScan { position, contrast, .. } => vec![("position", "Position", *position, 0.0, 1.0), ("contrast", "Contrast", *contrast, 0.0, 1.0)],
        Filter::HistogramRange { range, position } => vec![("range", "Range", *range, 0.0, 1.0), ("position", "Position", *position, 0.0, 1.0)],
        Filter::HistogramShift { offset } => vec![("position", "Position", Filter::shift_position(*offset), 0.0, 1.0)],
        Filter::Warp { intensity, scale, seed, .. } => {
            vec![
                ("intensity", "Intensity", *intensity, 0.0, TEXELS_SLIDER_MAX),
                ("scale", "Source Tiling", *scale, NOISE_TILING_MIN, NOISE_TILING_MAX),
                ("seed", "Seed", *seed as f32, 0.0, SEED_SLIDER_MAX),
            ]
        }
        Filter::DirectionalBlur { radius, angle } => {
            vec![
                ("intensity", "Intensity", *radius / Filter::BLUR_INTENSITY, 0.0, BLUR_INTENSITY_SLIDER_MAX),
                ("angle", "Angle", *angle, 0.0, FULL_TURN_DEGREES),
            ]
        }
        Filter::SlopeBlur { radius, scale, seed, .. } => {
            vec![
                ("radius", "Intensity", *radius, 0.0, TEXELS_SLIDER_MAX),
                ("scale", "Source Tiling", *scale, NOISE_TILING_MIN, NOISE_TILING_MAX),
                ("seed", "Seed", *seed as f32, 0.0, SEED_SLIDER_MAX),
            ]
        }
        Filter::HighPass { radius, .. } => vec![("intensity", "Radius", *radius / Filter::BLUR_INTENSITY, 0.0, BLUR_INTENSITY_SLIDER_MAX)],
        Filter::GrayscaleConversion(conversion) => conversion_params(conversion),
        Filter::AnisotropicKuwahara(kuwahara) => vec![
            ("radius", "Radius", kuwahara.radius, 0.0, AnisotropicKuwahara::MAX_RADIUS),
            ("smoothness", "Smoothness", kuwahara.smoothness, 0.0, 1.0),
            ("sharpness", "Sharpness", kuwahara.sharpness, AnisotropicKuwahara::MIN_SHARPNESS, 1.0),
            ("tensorSmoothness", "Tensor Smoothness", kuwahara.tensor_smoothness, 0.0, 1.0),
            ("anisotropy", "Anisotropy", kuwahara.anisotropy, 0.0, 1.0),
            ("angle", "Anisotropy Angle", kuwahara.angle, 0.0, FULL_TURN_DEGREES),
        ],
        Filter::Grayscale => conversion_params(&GrayscaleConversion::of_luminance()),
        Filter::HeightToNormal(converter) if converter.world_units => vec![
            ("surfaceSize", "Surface Size (cm)", converter.surface_size, HeightToNormal::MIN_SURFACE_SIZE, HeightToNormal::MAX_SURFACE_SIZE),
            ("heightDepth", "Height Depth (cm)", converter.height_depth, 0.0, HeightToNormal::MAX_HEIGHT_DEPTH),
        ],
        Filter::HeightToNormal(converter) => vec![("normalIntensity", "Normal Intensity", converter.intensity, 0.0, HeightToNormal::MAX_INTENSITY)],
        Filter::ColorMatch(matcher) => {
            vec![
                ("hue", "Hue", matcher.hue, 0.0, FULL_TURN_DEGREES),
                ("chroma", "Chroma", matcher.chroma, 0.0, ColorMatch::MAX_CHROMA),
                ("luma", "Luma", matcher.luma, 0.0, ColorMatch::MAX_LUMA),
            ]
        }
        Filter::EdgeDetect { width } => vec![("width", "Width", *width, 0.0, EDGE_WIDTH_SLIDER_MAX)],
        Filter::DirectionalWarp { intensity, angle, scale, seed, .. } => {
            vec![
                ("intensity", "Intensity", *intensity, 0.0, TEXELS_SLIDER_MAX),
                ("angle", "Angle", *angle, 0.0, FULL_TURN_DEGREES),
                ("scale", "Source Tiling", *scale, NOISE_TILING_MIN, NOISE_TILING_MAX),
                ("seed", "Seed", *seed as f32, 0.0, SEED_SLIDER_MAX),
            ]
        }
        Filter::Bevel { width, .. } => vec![("width", "Distance", *width, 0.0, TEXELS_SLIDER_MAX)],
        Filter::Mosaic { size, .. } => vec![("pixelsAmount", "Pixels Amount", Filter::pixels_amount(*size), 1.0, MOSAIC_PIXELS_SLIDER_MAX)],
        Filter::DistanceBevel(bevel) => {
            vec![("distance", "Distance", bevel.distance, -1.0, 1.0), ("smoothing", "Smoothing", bevel.smoothing, 0.0, DISTANCE_BEVEL_SMOOTHING_MAX)]
        }
        Filter::SourceWarp(warp) => {
            let mut params = vec![("seed", "Seed", warp.seed as f32, 0.0, SEED_SLIDER_MAX), ("intensity", "Intensity", warp.intensity, 0.0, 1.0)];
            if warp.mode != WarpMode::Warp {
                params.push(("angle", "Angle", warp.angle, 0.0, FULL_TURN_DEGREES));
            }
            params.extend([
                ("sourceBlur", "Source Blur", warp.source.blur, 0.0, 1.0),
                ("sourceBalance", "Source Balance", warp.source.balance, 0.0, 1.0),
                ("sourceContrast", "Source Contrast", warp.source.contrast, 0.0, 1.0),
                ("sourceTiling", "Source Tiling", warp.source.tiling, SOURCE_TILING_MIN, SOURCE_TILING_MAX),
            ]);
            params
        }
        Filter::SourceSlopeBlur(slope) => vec![
            ("seed", "Seed", slope.seed as f32, 0.0, SEED_SLIDER_MAX),
            ("intensity", "Intensity", slope.intensity, 0.0, 1.0),
            ("quality", "Quality", slope.quality, 0.0, 1.0),
            ("sourceBlur", "Blur", slope.source.blur, 0.0, 1.0),
            ("sourceBalance", "Position", slope.source.balance, 0.0, 1.0),
            ("sourceContrast", "Contrast", slope.source.contrast, 0.0, 1.0),
            ("sourceTiling", "Source Tiling", slope.source.tiling, SOURCE_TILING_MIN, SOURCE_TILING_MAX),
        ],
        Filter::BevelSmooth(smooth) => vec![
            ("distance", "Distance", smooth.distance, 0.0, SMOOTH_DISTANCE_MAX),
            ("smoothing", "Smoothing", smooth.smoothing, 0.0, BEVEL_SMOOTHING_MAX),
            ("curveOffset", "Curve Offset", smooth.curve_offset, -1.0, 1.0),
            ("curveShape", "Curve Shape", smooth.curve_shape, -1.0, 1.0),
            ("maskThreshold", "Mask Threshold", smooth.threshold, 0.0, 1.0),
            ("distanceMapMultiplier", "Distance Map Multiplier", smooth.multiplier, 0.0, 1.0),
        ],
        Filter::DirectionalDistance(trail) => vec![
            ("distance", "Distance", trail.distance, 0.0, SMOOTH_DISTANCE_MAX),
            ("angle", "Angle", trail.angle, 0.0, FULL_TURN_DEGREES),
            ("contrast", "Contrast", trail.contrast, 0.0, 1.0),
            ("distanceMapMultiplier", "Distance Map Multiplier", trail.multiplier, 0.0, 1.0),
        ],
        // Only the offset of the axis the mode mirrors across shows
        Filter::Mirror(mirror) => match mirror.mode {
            MirrorMode::AxisX => vec![("axisXOffset", "Axis X Offset", mirror.offset[0], 0.0, 1.0)],
            MirrorMode::AxisY => vec![("axisYOffset", "Axis Y Offset", mirror.offset[1], 0.0, 1.0)],
            MirrorMode::Corner => Vec::new(),
        },
        Filter::MaskOutline(outline) => vec![
            ("threshold", "Threshold", outline.threshold, 0.0, 1.0),
            ("width", "Width", outline.width, 0.0, 1.0),
            ("blur", "Blur", outline.blur, 0.0, 1.0),
            ("curveShape", "Curve Shape", outline.curve, -1.0, 1.0),
        ],
        Filter::Glow { radius, amount } => {
            vec![("radius", "Width", *radius, 0.0, TEXELS_SLIDER_MAX), ("amount", "Amount", *amount, 0.0, GLOW_AMOUNT_SLIDER_MAX)]
        }
        Filter::HaloGlow { width, spread, opacity, .. } => {
            vec![("width", "Width", *width, -1.0, 1.0), ("spread", "Spread", *spread, 0.0, 1.0), ("opacity", "Opacity", *opacity, 0.0, 1.0)]
        }
        Filter::DropShadow { angle, distance, size, spread, opacity, .. } => vec![
            ("angle", "Angle", *angle, 0.0, FULL_TURN_DEGREES),
            ("distance", "Distance", *distance, -Filter::SHADOW_DISTANCE_MAX, Filter::SHADOW_DISTANCE_MAX),
            ("size", "Size", *size, 0.0, 1.0),
            ("spread", "Spread", *spread, 0.0, 1.0),
            ("opacity", "Opacity", *opacity, 0.0, 1.0),
        ],
        Filter::BakedLighting { azimuth, elevation, light, ambient, occlusion, .. } => vec![
            ("light", "Sun Intensity", *light, 0.0, SUN_INTENSITY_MAX),
            ("azimuth", "Horizontal Angle", *azimuth, 0.0, FULL_TURN_DEGREES),
            ("elevation", "Vertical Angle", *elevation, 0.0, FULL_TURN_DEGREES),
            ("ambient", "Sky Intensity", *ambient, 0.0, SKY_INTENSITY_SLIDER_MAX),
            ("occlusion", "Diffuse AO", *occlusion, 0.0, 1.0),
        ],
        Filter::EnvironmentLighting { horizontal, vertical, exposure, height_intensity, ambient_occlusion, specular_occlusion } => vec![
            ("horizontalRotation", "Horizontal Rotation", *horizontal, 0.0, FULL_TURN_DEGREES),
            ("verticalRotation", "Vertical Rotation", *vertical, 0.0, FULL_TURN_DEGREES),
            ("exposure", "Exposure", *exposure, -Filter::ENVIRONMENT_EXPOSURE_STOPS, Filter::ENVIRONMENT_EXPOSURE_STOPS),
            ("heightIntensity", "Height Intensity", *height_intensity, 0.0, HEIGHT_INTENSITY_MAX),
            ("ambientOcclusion", "Ambient Occlusion Intensity", *ambient_occlusion, 0.0, 1.0),
            ("specularOcclusion", "Specular Occlusion Intensity", *specular_occlusion, 0.0, 1.0),
        ],
        Filter::ReplaceColor { tolerance, softness, .. } => {
            vec![("tolerance", "Tolerance", *tolerance, 0.0, 1.0), ("softness", "Softness", *softness, 0.0, 1.0)]
        }
        Filter::GradientMap { input, .. } => vec![
            ("blur", "Blur Intensity", input.blur, 0.0, BLUR_INTENSITY_SLIDER_MAX),
            ("balance", "Balance", input.balance, 0.0, 1.0),
            ("contrast", "Contrast", input.contrast, 0.0, 1.0),
        ],
        Filter::GradientCurve { balance, contrast, .. } => vec![("balance", "Balance", *balance, 0.0, 1.0), ("contrast", "Contrast", *contrast, 0.0, 1.0)],
        Filter::GradientDynamic { position, .. } => vec![("position", "Gradient Input Position", *position, 0.0, 1.0)],
        Filter::Invert
        | Filter::InvertOff
        | Filter::ColorBalance { .. }
        | Filter::ColorCorrect { .. }
        | Filter::NormalInvert
        | Filter::Fxaa { .. }
        | Filter::PbrValidate(_)
        | Filter::Transform(_)
        | Filter::Empty => Vec::new(),
    }
}

/// Grayscale Conversion's sliders: Balance and Contrast, then the Channels Weights' Red, Green, Blue and Alpha in that mode
fn conversion_params(conversion: &GrayscaleConversion) -> Vec<(&'static str, &'static str, f32, f32, f32)> {
    let mut params = vec![("balance", "Balance", conversion.balance, 0.0, 1.0), ("contrast", "Contrast", conversion.contrast, 0.0, 1.0)];
    if conversion.mode == ConversionMode::ChannelsWeights {
        let [red, green, blue, alpha] = conversion.weights;
        params.extend([
            ("red", "Red", red, 0.0, 1.0),
            ("green", "Green", green, 0.0, 1.0),
            ("blue", "Blue", blue, 0.0, 1.0),
            ("alpha", "Alpha", alpha, 0.0, 1.0),
        ]);
    }
    params
}

/// The keys `effect.set` takes for Grayscale Conversion
const CONVERSION_KEYS: [&str; 9] = ["channelInput", "invert", "balance", "contrast", "mode", "red", "green", "blue", "alpha"];

/// Set the Grayscale Conversion settings given in `p`: channelInput (currentChannel or customInput), invert, balance, contrast, mode and the red, green, blue and alpha weights
fn apply_conversion(p: &Value, conversion: &mut GrayscaleConversion, cmd: &str) -> Result<()> {
    conversion.custom_input = two_way(p, "channelInput", CURRENT_CHANNEL, CUSTOM_INPUT, cmd)?.unwrap_or(conversion.custom_input);
    set_bool(p, "invert", &mut conversion.invert, cmd)?;
    conversion.mode = choice(p, "mode", &ConversionMode::ALL, ConversionMode::id, cmd)?.unwrap_or(conversion.mode);

    let [red, green, blue, alpha] = &mut conversion.weights;
    for (key, slot) in
        [("balance", &mut conversion.balance), ("contrast", &mut conversion.contrast), ("red", red), ("green", green), ("blue", blue), ("alpha", alpha)]
    {
        if let Some(value) = f32p(p, key) {
            *slot = value.clamp(0.0, 1.0);
        }
    }
    Ok(())
}

/// Make a gradient use its first `wanted` colors: an evenly spaced ramp keeps its places from now on, and each added color starts at 1 as the ramp's color there
fn set_quantity(colors: &mut Vec<[f32; 3]>, positions: &mut Vec<f32>, quantity: &mut Option<u8>, wanted: u8) {
    let used = quantity.map_or(colors.len(), usize::from).min(colors.len());

    // The used colors without a place keep the even place the ramp gave them
    let last = used.saturating_sub(1).max(1) as f32;
    for index in positions.len()..used {
        positions.push(index as f32 / last);
    }

    let wanted_len = usize::from(wanted);

    // At 1 the blend space does not matter: only the rightmost stop counts, which need not be the last in the list
    let used_colors = colors.get(..used).unwrap_or_default();
    let fill = texturecraft_core::gradient_input::ramp(used_colors, positions, true, 1.0).unwrap_or([1.0; 3]);
    if colors.len() < wanted_len {
        colors.resize(wanted_len, fill);
    }
    if positions.len() < wanted_len {
        positions.resize(wanted_len, 1.0);
    }
    *quantity = Some(wanted);
}

/// Set the Mirror settings given in `p`: displayDebug, mode, axisXOffset, axisYOffset, invertAxisX, invertAxisY and cornerType
fn apply_mirror(p: &Value, mirror: &mut Mirror, cmd: &str) -> Result<()> {
    mirror.mode = choice(p, "mode", &MirrorMode::ALL, MirrorMode::id, cmd)?.unwrap_or(mirror.mode);
    mirror.corner = choice(p, "cornerType", &MirrorCorner::ALL, MirrorCorner::id, cmd)?.unwrap_or(mirror.corner);

    let [offset_x, offset_y] = &mut mirror.offset;
    for (key, slot) in [("axisXOffset", offset_x), ("axisYOffset", offset_y)] {
        if let Some(offset) = f32p(p, key) {
            *slot = offset.clamp(0.0, 1.0);
        }
    }

    let [invert_x, invert_y] = &mut mirror.invert;
    for (key, slot) in [("displayDebug", &mut mirror.debug), ("invertAxisX", invert_x), ("invertAxisY", invert_y)] {
        set_bool(p, key, slot, cmd)?;
    }
    Ok(())
}

/// The choice among `all` whose id (`id`) `p` gives under `key`, if it gives one; an unknown id is refused
pub(crate) fn choice<T: Copy>(p: &Value, key: &str, all: &[T], id: fn(T) -> &'static str, cmd: &str) -> Result<Option<T>> {
    if p.get(key).is_none() {
        return Ok(None);
    }

    let given = strp(p, key).unwrap_or_default();
    let found = all.iter().copied().find(|choice| id(*choice) == given);
    found.map(Some).ok_or_else(|| {
        let ids: Vec<&str> = all.iter().map(|choice| id(*choice)).collect();
        bad(cmd, format!("`{key}` must be {}", one_of(&ids)))
    })
}

/// Which of two ids `p` gives under `key` (true: `on_id`), if it gives one; any other value is refused
pub(crate) fn two_way(p: &Value, key: &str, off_id: &str, on_id: &str, cmd: &str) -> Result<Option<bool>> {
    if p.get(key).is_none() {
        return Ok(None);
    }

    match strp(p, key) {
        Some(id) if id == off_id => Ok(Some(false)),
        Some(id) if id == on_id => Ok(Some(true)),
        _ => Err(bad(cmd, format!("`{key}` must be {off_id} or {on_id}"))),
    }
}

/// Set `slot` to the true or false `p` gives under `key`, if it gives one; any other value is refused
pub(crate) fn set_bool(p: &Value, key: &str, slot: &mut bool, cmd: &str) -> Result<()> {
    if p.get(key).is_some() {
        *slot = boolp(p, key).ok_or_else(|| bad(cmd, format!("`{key}` must be true or false")))?;
    }
    Ok(())
}

/// Set `slot` to the color `p` gives under `key` (clamped to 0..1), if it gives one; any other value, null too, is refused
pub(crate) fn set_color(p: &Value, key: &str, slot: &mut [f32; 3], cmd: &str) -> Result<()> {
    if p.get(key).is_some() {
        let rgb = color_param(cmd, p, key)?.ok_or_else(|| bad(cmd, format!("`{key}` must be a color ([r, g, b] linear or \"#rrggbb\")")))?;
        *slot = rgb.map(|x| x.clamp(0.0, 1.0));
    }
    Ok(())
}

/// An array of exactly three finite numbers
fn three_numbers(given: &Value) -> Option<[f32; 3]> {
    let [first, second, third] = given.as_array()?.as_slice() else { return None };
    let number = |value: &Value| value.as_f64().map(|x| x as f32).filter(|x| x.is_finite());
    Some([number(first)?, number(second)?, number(third)?])
}

/// The allowed values of a setting as its error message lists them: "a, b or c"
fn one_of<T: std::fmt::Display>(allowed: &[T]) -> String {
    let words: Vec<String> = allowed.iter().map(ToString::to_string).collect();
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        _ => words.concat(),
    }
}

/// Set the parameters of `filter` given in `p` (clamped to their ranges); `effect.set` uses it too
pub(crate) fn apply(p: &Value, filter: &mut Filter, cmd: &str) -> Result<()> {
    let set = |slot: &mut f32, key: &str, low: f32, high: f32| {
        if let Some(value) = f32p(p, key) {
            *slot = value.clamp(low, high);
        }
    };
    let seed = |slot: &mut u32| {
        if let Some(value) = u64p(p, "seed") {
            *slot = u32::try_from(value).unwrap_or(u32::MAX);
        }
    };
    let divider = |slot: &mut u16| -> Result<()> {
        if p.get("divider").is_some() {
            let value = u64p(p, "divider").and_then(|value| u16::try_from(value).ok()).filter(|value| Filter::DIVIDERS.contains(value));
            *slot = value.ok_or_else(|| bad(cmd, format!("`divider` must be {}", one_of(&Filter::DIVIDERS))))?;
        }
        Ok(())
    };

    // A blur's Intensity in the reference workflow's unit (radius in texels at 2K ÷ Filter::BLUR_INTENSITY)
    let blur_intensity = |radius: &mut f32| {
        if let Some(intensity) = f32p(p, "intensity") {
            *radius = (intensity * Filter::BLUR_INTENSITY).clamp(0.0, TEXELS_MAX);
        }
    };
    let switch = |slot: &mut bool, key: &str| set_bool(p, key, slot, cmd);
    let shade = |slot: &mut [f32; 3]| set_color(p, "color", slot, cmd);
    let angle = |slot: &mut f32| {
        if let Some(degrees) = f32p(p, "angle") {
            *slot = degrees.rem_euclid(FULL_TURN_DEGREES);
        }
    };

    match filter {
        Filter::Invert | Filter::InvertOff => {
            let mut on = matches!(filter, Filter::Invert);
            switch(&mut on, "invert")?;
            *filter = if on { Filter::Invert } else { Filter::InvertOff };
        }
        Filter::NormalInvert | Filter::Empty => {}

        // A Grayscale saved before the settings takes its luminance weights on its first edit, so it looks the same
        Filter::Grayscale => {
            if CONVERSION_KEYS.iter().any(|key| p.get(key).is_some()) {
                let mut conversion = GrayscaleConversion::of_luminance();
                apply_conversion(p, &mut conversion, cmd)?;
                *filter = Filter::GrayscaleConversion(conversion);
            }
        }
        Filter::GrayscaleConversion(conversion) => apply_conversion(p, conversion, cmd)?,
        Filter::AnisotropicKuwahara(kuwahara) => {
            kuwahara.custom_input = two_way(p, "extractDirection", CURRENT_CHANNEL, CUSTOM_INPUT, cmd)?.unwrap_or(kuwahara.custom_input);
            set(&mut kuwahara.radius, "radius", 0.0, AnisotropicKuwahara::MAX_RADIUS);
            set(&mut kuwahara.smoothness, "smoothness", 0.0, 1.0);
            set(&mut kuwahara.sharpness, "sharpness", AnisotropicKuwahara::MIN_SHARPNESS, 1.0);
            set(&mut kuwahara.tensor_smoothness, "tensorSmoothness", 0.0, 1.0);
            set(&mut kuwahara.anisotropy, "anisotropy", 0.0, 1.0);
            angle(&mut kuwahara.angle);
        }
        Filter::PbrValidate(validate) => {
            validate.mode = choice(p, "validationMode", &ValidationMode::ALL, ValidationMode::id, cmd)?.unwrap_or(validate.mode);
            for (key, slot, allowed) in [
                ("albedoDarkRangeThreshold", &mut validate.albedo_dark, PbrValidate::ALBEDO_DARK),
                ("metalReflectanceRange", &mut validate.metal_low, PbrValidate::METAL_LOW),
            ] {
                if p.get(key).is_some() {
                    let value = u64p(p, key).and_then(|value| u8::try_from(value).ok()).filter(|value| allowed.contains(value));
                    *slot = value.ok_or_else(|| bad(cmd, format!("`{key}` must be {}", one_of(&allowed))))?;
                }
            }
        }
        Filter::HeightToNormal(converter) => {
            switch(&mut converter.overwrite, "overwriteExistingNormal")?;
            switch(&mut converter.world_units, "useWorldUnits")?;
            set(&mut converter.surface_size, "surfaceSize", HeightToNormal::MIN_SURFACE_SIZE, HeightToNormal::MAX_SURFACE_SIZE);
            set(&mut converter.height_depth, "heightDepth", 0.0, HeightToNormal::MAX_HEIGHT_DEPTH);
            set(&mut converter.intensity, "normalIntensity", 0.0, HeightToNormal::MAX_INTENSITY);
        }
        Filter::ColorMatch(matcher) => {
            matcher.source_mode = choice(p, "sourceColorMode", &SourceColorMode::ALL, SourceColorMode::id, cmd)?.unwrap_or(matcher.source_mode);
            matcher.target_mode = choice(p, "targetColorMode", &TargetColorMode::ALL, TargetColorMode::id, cmd)?.unwrap_or(matcher.target_mode);
            set_color(p, "sourceColor", &mut matcher.source_color, cmd)?;
            set_color(p, "targetColor", &mut matcher.target_color, cmd)?;
            switch(&mut matcher.custom_variation, "customColorVariation")?;
            set(&mut matcher.hue, "hue", 0.0, FULL_TURN_DEGREES);
            set(&mut matcher.chroma, "chroma", 0.0, ColorMatch::MAX_CHROMA);
            set(&mut matcher.luma, "luma", 0.0, ColorMatch::MAX_LUMA);
        }
        Filter::Threshold { level, mode } => {
            set(level, "level", 0.0, 1.0);
            *mode = choice(p, "mode", &ThresholdMode::ALL, ThresholdMode::id, cmd)?.unwrap_or(*mode);
        }
        Filter::Mirror(mirror) => apply_mirror(p, mirror, cmd)?,
        Filter::Transform(transform) => crate::tile_transform_params::apply(p, transform, cmd)?,
        Filter::SourceWarp(warp) => {
            seed(&mut warp.seed);
            warp.mode = choice(p, "warpMode", &WarpMode::ALL, WarpMode::id, cmd)?.unwrap_or(warp.mode);
            set(&mut warp.intensity, "intensity", 0.0, 1.0);
            divider(&mut warp.divider)?;
            angle(&mut warp.angle);
            if p.get("directions").is_some() {
                let count = u64p(p, "directions").and_then(|count| u8::try_from(count).ok()).filter(|count| SourceWarp::DIRECTIONS.contains(count));
                warp.directions = count.ok_or_else(|| bad(cmd, format!("`directions` must be {}", one_of(&SourceWarp::DIRECTIONS))))?;
            }
            warp.blending = choice(p, "blendingMode", &WarpBlending::ALL, WarpBlending::id, cmd)?.unwrap_or(warp.blending);
            warp.source.mode = choice(p, "sourceMode", &WarpSourceMode::ALL, WarpSourceMode::id, cmd)?.unwrap_or(warp.source.mode);
            set(&mut warp.source.blur, "sourceBlur", 0.0, 1.0);
            set(&mut warp.source.balance, "sourceBalance", 0.0, 1.0);
            set(&mut warp.source.contrast, "sourceContrast", 0.0, 1.0);
            set(&mut warp.source.tiling, "sourceTiling", SOURCE_TILING_MIN, SOURCE_TILING_MAX);
        }
        Filter::SourceSlopeBlur(slope) => {
            seed(&mut slope.seed);
            set(&mut slope.intensity, "intensity", 0.0, 1.0);
            divider(&mut slope.divider)?;
            slope.blending = choice(p, "blendingMode", &SlopeBlending::ALL, SlopeBlending::id, cmd)?.unwrap_or(slope.blending);
            set(&mut slope.quality, "quality", 0.0, 1.0);
            slope.source.mode = choice(p, "sourceMode", &WarpSourceMode::ALL, WarpSourceMode::id, cmd)?.unwrap_or(slope.source.mode);
            set(&mut slope.source.blur, "sourceBlur", 0.0, 1.0);
            set(&mut slope.source.balance, "sourceBalance", 0.0, 1.0);
            set(&mut slope.source.contrast, "sourceContrast", 0.0, 1.0);
            set(&mut slope.source.tiling, "sourceTiling", SOURCE_TILING_MIN, SOURCE_TILING_MAX);
        }
        Filter::DistanceBevel(bevel) => {
            set(&mut bevel.distance, "distance", -1.0, 1.0);
            set(&mut bevel.smoothing, "smoothing", 0.0, DISTANCE_BEVEL_SMOOTHING_MAX);
            bevel.corner = choice(p, "cornerType", &BevelCorner::ALL, BevelCorner::id, cmd)?.unwrap_or(bevel.corner);
            bevel.blur = choice(p, "blurMode", &BevelBlur::ALL, BevelBlur::id, cmd)?.unwrap_or(bevel.blur);
            switch(&mut bevel.wrap, "wrap")?;
        }
        Filter::BevelSmooth(smooth) => {
            smooth.direction = choice(p, "direction", &BevelDirection::ALL, BevelDirection::id, cmd)?.unwrap_or(smooth.direction);
            set(&mut smooth.distance, "distance", 0.0, SMOOTH_DISTANCE_MAX);
            set(&mut smooth.smoothing, "smoothing", 0.0, BEVEL_SMOOTHING_MAX);
            set(&mut smooth.curve_offset, "curveOffset", -1.0, 1.0);
            set(&mut smooth.curve_shape, "curveShape", -1.0, 1.0);
            set(&mut smooth.threshold, "maskThreshold", 0.0, 1.0);
            set(&mut smooth.multiplier, "distanceMapMultiplier", 0.0, 1.0);
            switch(&mut smooth.wrap, "wrap")?;
        }
        Filter::DirectionalDistance(trail) => {
            set(&mut trail.distance, "distance", 0.0, SMOOTH_DISTANCE_MAX);
            angle(&mut trail.angle);
            set(&mut trail.contrast, "contrast", 0.0, 1.0);
            set(&mut trail.multiplier, "distanceMapMultiplier", 0.0, 1.0);
        }
        Filter::MaskOutline(outline) => {
            outline.position = choice(p, "maskPosition", &OutlinePosition::ALL, OutlinePosition::id, cmd)?.unwrap_or(outline.position);
            switch(&mut outline.from_black, "startFromBlack")?;
            set(&mut outline.threshold, "threshold", 0.0, 1.0);
            set(&mut outline.width, "width", 0.0, 1.0);
            set(&mut outline.blur, "blur", 0.0, 1.0);
            set(&mut outline.curve, "curveShape", -1.0, 1.0);
        }
        Filter::Blur { radius, wrap: wraps } => {
            set(radius, "radius", 0.0, TEXELS_MAX);
            blur_intensity(radius);
            switch(wraps, "wrap")?;
        }
        Filter::Sharpen { amount } => set(amount, "amount", 0.0, SHARPEN_AMOUNT_MAX),
        Filter::Hsl { hue, saturation, lightness, perceptive } => {
            *perceptive = two_way(p, "mode", "regular", "perceptive", cmd)?.unwrap_or(*perceptive);
            set(hue, "hue", -1.0, 1.0);
            set(saturation, "saturation", -1.0, 1.0);
            set(lightness, "lightness", -1.0, 1.0);
        }
        Filter::ContrastLuminosity { contrast, luminosity } => {
            set(contrast, "contrast", -1.0, 1.0);
            set(luminosity, "luminosity", -1.0, 1.0);
        }
        Filter::ColorCorrect { shadows, midtones, highlights } => {
            for (key, slot) in [("shadows", shadows), ("midtones", midtones), ("highlights", highlights)] {
                if let Some(given) = p.get(key) {
                    let [contrast, luminosity, saturation] =
                        three_numbers(given).ok_or_else(|| bad(cmd, format!("`{key}` must be [contrast, luminosity, saturation]")))?;
                    *slot = [contrast.clamp(-1.0, 1.0), luminosity.clamp(-1.0, 1.0), saturation.clamp(0.0, 1.0)];
                }
            }
        }
        Filter::ColorBalance { shadows, midtones, highlights } => {
            for (key, slot) in [("shadows", shadows), ("midtones", midtones), ("highlights", highlights)] {
                if let Some(given) = p.get(key).filter(|given| !given.is_null()) {
                    let offsets = color_of(given).ok_or_else(|| bad(cmd, format!("`{key}` must be [r, g, b] offsets in -1..1")))?;
                    *slot = offsets.map(|x| x.clamp(-1.0, 1.0));
                }
            }
        }
        Filter::GradientMap { colors, positions, quantity, input, linear, debug } => {
            if let Some(list) = p.get("colors") {
                let parsed: Option<Vec<[f32; 3]>> =
                    list.as_array().and_then(|given| given.iter().map(|color| color_of(color).map(|rgb| rgb.map(|x| x.clamp(0.0, 1.0)))).collect());
                match parsed {
                    Some(parsed) if (1..=GRADIENT_COLORS_GIVEN_MAX).contains(&parsed.len()) => *colors = parsed,
                    _ => return Err(bad(cmd, format!("`colors` must be 1 to {GRADIENT_COLORS_GIVEN_MAX} colors ([r, g, b] linear or \"#rrggbb\" sRGB)"))),
                }
            }

            if let Some(list) = p.get("positions") {
                let parsed: Option<Vec<f32>> =
                    list.as_array().and_then(|given| given.iter().map(|place| place.as_f64().map(|place| (place as f32).clamp(0.0, 1.0))).collect());
                match parsed {
                    Some(parsed) if parsed.len() <= colors.len() => *positions = parsed,
                    _ => return Err(bad(cmd, "`positions` must be numbers in 0..1, at most one per color")),
                }
            }

            if p.get("quantity").is_some() {
                let wanted = u64p(p, "quantity")
                    .and_then(|count| u8::try_from(count).ok())
                    .filter(|count| (GRADIENT_QUANTITY_MIN..=Filter::MAX_GRADIENT_COLORS).contains(count));
                let wanted = wanted
                    .ok_or_else(|| bad(cmd, format!("`quantity` must be a whole number from {GRADIENT_QUANTITY_MIN} to {}", Filter::MAX_GRADIENT_COLORS)))?;
                set_quantity(colors, positions, quantity, wanted);
            }
            input.mode = choice(p, "grayscaleMode", &GrayscaleMode::ALL, GrayscaleMode::id, cmd)?.unwrap_or(input.mode);
            set(&mut input.blur, "blur", 0.0, BLUR_INTENSITY_SLIDER_MAX);
            set(&mut input.balance, "balance", 0.0, 1.0);
            set(&mut input.contrast, "contrast", 0.0, 1.0);
            for (key, slot) in [("invert", &mut input.invert), ("linear", linear), ("debug", debug)] {
                switch(slot, key)?;
            }
        }
        Filter::GradientCurve { preset, invert, balance, contrast } => {
            *preset = choice(p, "preset", &CurvePreset::ALL, CurvePreset::id, cmd)?.unwrap_or(*preset);
            switch(invert, "invert")?;
            set(balance, "balance", 0.0, 1.0);
            set(contrast, "contrast", 0.0, 1.0);
        }
        Filter::GradientDynamic { vertical, position, color } => {
            *vertical = two_way(p, "orientation", "horizontal", "vertical", cmd)?.unwrap_or(*vertical);
            set(position, "position", 0.0, 1.0);
            shade(color)?;
        }
        Filter::Posterize { levels, alpha } => {
            set(levels, "levels", POSTERIZE_LEVELS_MIN, POSTERIZE_LEVELS_MAX);
            switch(alpha, "applyToAlpha")?;
        }
        Filter::Clamp { min, max } | Filter::Smoothstep { min, max } => {
            set(min, "min", 0.0, 1.0);
            set(max, "max", 0.0, 1.0);
        }
        Filter::HeightAdjust { invert, offset, multiply } => {
            switch(invert, "invert")?;
            set(offset, "offset", -HEIGHT_OFFSET_MAX, HEIGHT_OFFSET_MAX);
            set(multiply, "multiply", 0.0, HEIGHT_MULTIPLY_MAX);
        }
        Filter::HistogramScan { position, contrast, invert_position } => {
            set(position, "position", 0.0, 1.0);
            set(contrast, "contrast", 0.0, 1.0);
            switch(invert_position, "invertPosition")?;
        }
        Filter::HistogramRange { range, position } => {
            set(range, "range", 0.0, 1.0);
            set(position, "position", 0.0, 1.0);
        }
        Filter::HistogramShift { offset } => {
            set(offset, "offset", -1.0, 1.0);
            if let Some(position) = f32p(p, "position") {
                *offset = Filter::shift_offset(position);
            }
        }
        Filter::Warp { intensity, scale, seed: noise_seed, divider: intensity_divider } => {
            set(intensity, "intensity", 0.0, TEXELS_MAX);
            set(scale, "scale", Filter::NOISE_SCALE_MIN, Filter::NOISE_SCALE_MAX);
            seed(noise_seed);
            divider(intensity_divider)?;
        }
        Filter::DirectionalBlur { radius, angle: direction } => {
            set(radius, "radius", 0.0, TEXELS_MAX);
            blur_intensity(radius);
            angle(direction);
        }
        Filter::SlopeBlur { radius, scale, seed: noise_seed, divider: intensity_divider } => {
            set(radius, "radius", 0.0, TEXELS_MAX);
            set(scale, "scale", Filter::NOISE_SCALE_MIN, Filter::NOISE_SCALE_MAX);
            seed(noise_seed);
            divider(intensity_divider)?;
        }
        Filter::HighPass { radius, space, alpha } => {
            set(radius, "radius", 0.0, TEXELS_MAX);
            blur_intensity(radius);
            *space = choice(p, "colorSpace", &HighpassSpace::ALL, HighpassSpace::id, cmd)?.unwrap_or(*space);
            switch(alpha, "applyToAlpha")?;
        }
        Filter::EdgeDetect { width } => set(width, "width", 0.0, TEXELS_MAX),
        Filter::Bevel { width, wrap: wraps } => {
            set(width, "width", 0.0, TEXELS_MAX);
            switch(wraps, "wrap")?;
        }
        Filter::Mosaic { size, alpha } => {
            set(size, "size", 1.0, MOSAIC_SIZE_MAX);
            if let Some(amount) = f32p(p, "pixelsAmount") {
                *size = Filter::pixel_size(amount.round());
            }
            switch(alpha, "applyToAlpha")?;
        }
        Filter::Fxaa { alpha } => switch(alpha, "affectAlphaChannel")?,
        Filter::Glow { radius, amount } => {
            set(radius, "radius", 0.0, TEXELS_MAX);
            set(amount, "amount", 0.0, GLOW_AMOUNT_MAX);
        }
        Filter::HaloGlow { on_mask, precise, width, spread, opacity, color } => {
            switch(on_mask, "usedOnMask")?;
            *precise = two_way(p, "mode", "soft", "precise", cmd)?.unwrap_or(*precise);
            set(width, "width", -1.0, 1.0);
            set(spread, "spread", 0.0, 1.0);
            set(opacity, "opacity", 0.0, 1.0);
            shade(color)?;
        }
        Filter::DropShadow { angle: direction, distance, size, spread, opacity, color } => {
            angle(direction);
            set(distance, "distance", -Filter::SHADOW_DISTANCE_MAX, Filter::SHADOW_DISTANCE_MAX);
            set(size, "size", 0.0, 1.0);
            set(spread, "spread", 0.0, 1.0);
            set(opacity, "opacity", 0.0, 1.0);
            shade(color)?;
        }
        Filter::ReplaceColor { from, to, tolerance, softness } => {
            for (key, slot) in [("from", from), ("to", to)] {
                if let Some(rgb) = color_param(cmd, p, key)? {
                    *slot = rgb.map(|x| x.clamp(0.0, 1.0));
                }
            }
            set(tolerance, "tolerance", 0.0, 1.0);
            set(softness, "softness", 0.0, 1.0);
        }
        Filter::BakedLighting { azimuth, elevation, light, ambient, occlusion, stylized } => {
            if let Some(degrees) = f32p(p, "azimuth") {
                *azimuth = degrees.rem_euclid(FULL_TURN_DEGREES);
            }

            // A light past the zenith comes from the far side: the angle wraps like the horizontal one
            if let Some(degrees) = f32p(p, "elevation") {
                *elevation = degrees.rem_euclid(FULL_TURN_DEGREES);
            }
            set(light, "light", 0.0, SUN_INTENSITY_MAX);
            set(ambient, "ambient", 0.0, SKY_INTENSITY_MAX);
            set(occlusion, "occlusion", 0.0, 1.0);

            // A Baked Lighting saved before the stylized settings takes them lit as it was
            if crate::stylized_lighting_params::sets_any(p) {
                crate::stylized_lighting_params::apply(p, stylized.get_or_insert_with(|| Box::new(StylizedLighting::PLAIN)), cmd)?;
            }
        }
        Filter::EnvironmentLighting { horizontal, vertical, exposure, height_intensity, ambient_occlusion, specular_occlusion } => {
            for (key, slot) in [("horizontalRotation", horizontal), ("verticalRotation", vertical)] {
                if let Some(degrees) = f32p(p, key) {
                    *slot = degrees.rem_euclid(FULL_TURN_DEGREES);
                }
            }
            set(exposure, "exposure", -Filter::ENVIRONMENT_EXPOSURE_STOPS, Filter::ENVIRONMENT_EXPOSURE_STOPS);
            set(height_intensity, "heightIntensity", 0.0, HEIGHT_INTENSITY_MAX);
            set(ambient_occlusion, "ambientOcclusion", 0.0, 1.0);
            set(specular_occlusion, "specularOcclusion", 0.0, 1.0);
        }
        Filter::DirectionalWarp { intensity, angle: direction, scale, seed: noise_seed, divider: intensity_divider } => {
            set(intensity, "intensity", 0.0, TEXELS_MAX);
            angle(direction);
            set(scale, "scale", Filter::NOISE_SCALE_MIN, Filter::NOISE_SCALE_MAX);
            seed(noise_seed);
            divider(intensity_divider)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_filter_builds_and_finds_by_id_or_label() {
        for def in FILTERS {
            let kind = build(def.id, &Value::Null).unwrap_or_else(|err| panic!("{}: {err}", def.id));

            // Material finishes are content only; other filters also run on masks, where some (normal invert) change nothing and are not offered
            assert_eq!(kind.content_only().is_none(), def.mask || matches!(kind, EffectKind::Filter(_)), "{}", def.id);
            assert_eq!(find(def.label).map(|found| found.id), Some(def.id));
        }

        let EffectKind::Filter(Filter::Posterize { levels, .. }) = build("posterize", &json!({"levels": 1000})).unwrap() else { panic!() };
        assert_eq!(levels, POSTERIZE_LEVELS_MAX);

        assert!(build("nope", &Value::Null).is_err());
        assert!(build("gradientMap", &json!({"colors": []})).is_err());
        assert!(build("colorBalance", &json!({"shadows": "x"})).is_err());

        let EffectKind::Filter(Filter::GradientMap { colors, .. }) = build("gradientMap", &json!({"colors": [[0, 0, 0], "#ffffff"]})).unwrap() else {
            panic!()
        };

        assert_eq!(colors.len(), 2);
    }

    #[test]
    fn a_new_gradient_given_colors_and_a_quantity_spaces_them_evenly() {
        let five = [[0.0; 3], [0.25; 3], [0.5; 3], [0.75; 3], [1.0; 3]];
        for (given, quantity, expected) in [
            (&five[..2], 2, vec![0.0, 1.0]),
            (&five[..], 5, vec![0.0, 0.25, 0.5, 0.75, 1.0]),
            (&five[..], 3, vec![0.0, 0.5, 1.0]),
            (&five[..2], 3, vec![0.0, 0.5, 1.0]),
        ] {
            let params = json!({"colors": given, "quantity": quantity});

            let EffectKind::Filter(Filter::GradientMap { positions, quantity: used, .. }) = build("gradientMap", &params).unwrap() else { panic!() };

            assert_eq!(positions, expected, "{params}");
            assert_eq!(used, Some(quantity));
        }
    }

    /// A gradient given its colors and their places ramps through all of them, not the library's three
    #[test]
    fn a_new_gradient_given_colors_and_places_uses_every_color() {
        let five = [[0.0; 3], [0.25; 3], [0.5; 3], [0.75; 3], [1.0; 3]];
        let places = [0.0, 0.1, 0.2, 0.3, 1.0];

        let EffectKind::Filter(filter) = build("gradientMap", &json!({"colors": five, "positions": places})).unwrap() else { panic!() };

        assert_eq!(filter.ramp_colors().len(), five.len());
        assert!(matches!(&filter, Filter::GradientMap { positions, .. } if positions[..] == places));
    }

    /// Raising Color Quantity leaves the colors the gradient had where they were, placed or not
    #[test]
    fn raising_the_quantity_keeps_colors_without_a_place_where_they_were() {
        let three = json!({"colors": [[0, 0, 0], [1, 0, 0], [1, 1, 1]], "positions": [0.0]});
        let EffectKind::Filter(mut filter) = build("gradientMap", &three).unwrap() else { panic!() };
        let middle = filter.ramp_at(0.5);

        apply(&json!({"quantity": 4}), &mut filter, "effect.set").unwrap();

        assert_eq!(filter.ramp_at(0.5), middle);
        assert!(matches!(&filter, Filter::GradientMap { positions, .. } if positions[..] == [0.0, 0.5, 1.0, 1.0]), "{filter:?}");
    }

    #[test]
    fn every_filter_row_reads_a_reference_name_without_an_internal_unit() {
        let old_saves = [
            Filter::Warp { intensity: 8.0, scale: 4.0, seed: 1, divider: 1 },
            Filter::SlopeBlur { radius: 8.0, scale: 4.0, seed: 1, divider: 1 },
            Filter::DirectionalWarp { intensity: 8.0, angle: 0.0, scale: 4.0, seed: 1, divider: 1 },
            Filter::EdgeDetect { width: 2.0 },
            Filter::Bevel { width: 8.0, wrap: false },
            Filter::Glow { radius: 16.0, amount: 1.0 },
        ];
        let library = FILTERS.iter().filter_map(|def| match build(def.id, &Value::Null) {
            Ok(EffectKind::Filter(filter)) => Some(filter),
            _ => None,
        });
        for filter in old_saves.into_iter().chain(library) {
            for (key, label, ..) in scalar_params(&filter) {
                let title_case = label.split([' ', '/']).filter(|word| !word.is_empty()).all(|word| word.starts_with(|c: char| !c.is_lowercase()));
                assert!(title_case && !label.contains("texels") && !label.contains("degrees"), "{key}: {label}");
            }
        }
    }
}
