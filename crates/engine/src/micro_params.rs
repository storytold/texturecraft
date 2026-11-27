//! The micro details controls `effect.add.generator` and `effect.set` take

use serde_json::Value;
use texturecraft_core::{Generator, GeneratorKind, MicroCurvatureType, MicroDetails, SectionParameters, WearParameters};

use crate::Result;
use crate::commands::{bad, boolp, f32p, strp};

/// The micro details controls
pub const MICRO_KEYS: [&str; 8] =
    ["useMicroDetails", "microHeight", "microNormal", "microCurvatureType", "microCurvatureIntensity", "microHeightIntensity", "microAoRadius", "microAoDepth"];

/// The input names of the micro height and micro normal images, beside the mesh map ids
pub const MICRO_HEIGHT_INPUT: &str = "microHeight";
pub const MICRO_NORMAL_INPUT: &str = "microNormal";

/// Whether micro details control `key` changes a generator of `kind`
pub fn micro_key_applies(kind: GeneratorKind, key: &str) -> bool {
    match key {
        "useMicroDetails" => MicroDetails::has_switch(kind),
        "microCurvatureType" | "microCurvatureIntensity" | "microHeightIntensity" => MicroDetails::has_curvature(kind),
        "microAoRadius" | "microAoDepth" => MicroDetails::has_occlusion(kind),
        _ => MicroDetails::defaults(kind).is_some(),
    }
}

/// The micro details controls in `params` (a micro height or normal input counts as one): they move a generator off its earlier model onto the reference's controls; `earlier_keys` are the keys that keep it there
pub fn micro_from(command: &str, params: &Value, generator: &mut Generator, earlier_keys: &[&str]) -> Result<()> {
    let input_given = matches!(strp(params, "input"), Some(MICRO_HEIGHT_INPUT | MICRO_NORMAL_INPUT));
    let mut requested_keys = MICRO_KEYS.iter().filter(|key| params.get(**key).is_some()).peekable();
    if requested_keys.peek().is_none() && !input_given {
        return Ok(());
    }

    let kind = generator.kind;
    let Some(defaults) = MicroDetails::defaults(kind) else {
        return Err(bad(command, format!("{} has no micro details: only Curvature, Ambient Occlusion and the wear kinds have them", kind.label())));
    };

    if let Some(unused) = requested_keys.find(|key| !micro_key_applies(kind, key)) {
        return Err(bad(command, format!("{} has no `{unused}`", kind.label())));
    }

    if WearParameters::defaults(kind).is_some() && ["blur", "balance", "contrast"].iter().any(|key| params.get(*key).is_some()) {
        return Err(bad(command, format!("{} has no global blur, balance or contrast beside its micro details", kind.label())));
    }

    if earlier_keys.iter().any(|key| params.get(*key).is_some()) {
        return Err(bad(command, "level, softness, noise and offset drive the earlier model: give them or the micro details, not both"));
    }

    // Validate the switches before they can change the generator model
    for key in ["useMicroDetails", "microHeight", "microNormal"] {
        if params.get(key).is_some_and(|value| !value.is_boolean()) {
            return Err(bad(command, format!("`{key}` must be a boolean")));
        }
    }

    if generator.on_earlier_model() {
        generator.offset = 0.0;
        if let Some(wear) = WearParameters::defaults(kind) {
            generator.wear = Some(wear);
        } else {
            let fresh = Generator::new(kind);
            (generator.level, generator.softness, generator.noise_amount, generator.noise_scale) =
                (fresh.level, fresh.softness, fresh.noise_amount, fresh.noise_scale);
            generator.section = SectionParameters::defaults(kind);
        }
    }

    let micro = generator.micro.get_or_insert_with(|| Box::new(defaults));
    micro.earlier = false;
    for (key, value) in [("useMicroDetails", &mut micro.enabled), ("microHeight", &mut micro.height), ("microNormal", &mut micro.normal)] {
        if let Some(given) = boolp(params, key) {
            *value = given;
        }
    }

    for (key, value) in [
        ("microCurvatureIntensity", &mut micro.curvature_intensity),
        ("microHeightIntensity", &mut micro.height_intensity),
        ("microAoRadius", &mut micro.ao_radius),
        ("microAoDepth", &mut micro.ao_depth),
    ] {
        if let Some(given) = f32p(params, key) {
            *value = given;
        }
    }

    if let Some(value) = params.get("microCurvatureType") {
        let name = value.as_str().ok_or_else(|| bad(command, "`microCurvatureType` must be a string"))?;
        micro.curvature_type = MicroCurvatureType::parse(name).ok_or_else(|| bad(command, format!("unknown curvature type `{name}`")))?;
    }

    **micro = micro.as_ref().clone().clamped(kind);
    Ok(())
}

/// `current`'s micro details carried onto a `kind` generator: its images stay, its controls start from `kind`'s defaults
pub fn swapped_micro(current: Option<&MicroDetails>, kind: GeneratorKind) -> Option<Box<MicroDetails>> {
    let current = current.filter(|micro| micro.sources().next().is_some())?;
    let defaults = MicroDetails::defaults(kind)?;
    Some(Box::new(MicroDetails { height_source: current.height_source.clone(), normal_source: current.normal_source.clone(), ..defaults }))
}
