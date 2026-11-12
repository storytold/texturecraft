//! Particle brushes (`DataEmitterPhysics`) as an approximation: a small simulation that makes the same kind of effect in
//! roughly the same place, amount and direction as the reference app, particle by particle different. Never parity: the
//! scripts that drive the reference app's particles are not read (only their names and stored tweaks), so everything
//! here is measured from the original's renders of controlled strokes. A step with particles is noted as approximated.
//!
//! The simulation is shared by every receiver (TextureCraft's `particles::simulate`): particles spawn at the stroke's
//! recorded points and along its path, travel over the surface in the mesh's space, and lay fading dabs. What differs
//! per receiver is one row of numbers ([`Behaviour`]), read with the stored tweaks of the same names
//! ([`ReplayParticles`]); a receiver not known gets the generic row. The emitter says how many spawn and how far around
//! the stroke ([`EmitterShape`]).
//!
//! The receiver and emitter names below are resource names read from project files (the scripts' file names), matched
//! without case; tweak identifiers are the scripts' own parameter names as stored.

use texturecraft_import_reader::model::{PaintEffect, PhysicsEmitter, StampEmitter, Tweak, TweakValue};

use super::{ReplayJob, ReplayParticles};

/// The words the note of a step with particles starts with.
pub const APPROXIMATE: &str = "particles approximated";
/// `TriggerDeviation` to the kink's turn, radians per unit (fitted by eye; not measured).
const KINK_ANGLE_PER_DEVIATION: f32 = 1.6;
/// The step between dabs when a project stores none or a longer one, in seconds.
const DEFAULT_STEP: f32 = 0.01;
/// `SpawnSpread` as a reach around the stroke, in mesh units (the same at every brush size).
const SPREAD_REACH: f32 = 0.1;
/// Under this `SpawnSpeed` a stroke leaves nothing (2 does, 5 doesn't).
const MIN_SPAWN_SPEED: f32 = 3.0;
/// Most particles per point or per mesh unit read from a file (a guard against hostile numbers).
const MAX_RATE: f32 = 1e4;

/// How a receiver's particles behave. Lengths are mesh units (the stored mesh spans 2), times seconds. `speed`, `life`,
/// `speed_rand`, `life_rand`, `fade_size`, `fade_opacity` are what a receiver without those tweaks uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Behaviour {
    /// Which row of the table.
    pub name: &'static str,
    /// True when the row was fitted on the original's renders of that receiver, false for a guess from a similar one.
    pub measured: bool,
    /// Particles per recorded point of a stroke (measured: neither the stroke's time nor `SpawnSpeed` changes how
    /// many), per mesh unit of its path, and at most this many in one stroke (0: no limit).
    pub per_click: f32,
    pub per_length: f32,
    pub budget: f32,
    pub speed: f32,
    pub speed_rand: f32,
    pub life: f32,
    pub life_rand: f32,
    /// Acceleration down the mesh's −Y along the surface (negative: up).
    pub gravity: f32,
    /// Share of the speed lost per second.
    pub friction: f32,
    /// Smooth random turning: how strongly the rate of turn, radians per second, drifts (per square root of a second).
    pub wander: f32,
    /// Steady turning, radians per second, either way per particle.
    pub curl: f32,
    /// Particles start in a fan this wide (radians, a full turn is all ways) around `heading`, radians from +X toward +Y
    /// of the surface as seen from outside; `away`: around the direction away from the stroke's middle instead.
    pub fan: f32,
    pub heading: f32,
    pub away: bool,
    /// A sudden turn every so many seconds (0: none), up to this many radians, with this many extra particles from it,
    /// its speed multiplied by `kink_speed`. A particle ends when its speed falls under `stop_speed` of its start.
    pub kink_time: f32,
    pub kink_angle: f32,
    pub kink_branches: f32,
    pub kink_speed: f32,
    pub stop_speed: f32,
    /// How much of the view's direction along the surface a particle keeps when it lands: particles fly in from the
    /// view, so a surface seen head-on sends them all ways and one seen at a slant sends them on (0: none).
    pub view_bias: f32,
    /// Particles start this far from the stroke.
    pub scatter: f32,
    /// Share of the size and opacity left at the end of a life at fade 1.
    pub fade_size: f32,
    pub fade_opacity: f32,
    /// Multiplies the dab's size and flow.
    pub size: f32,
    pub flow: f32,
    /// Dabs this many times further apart than the usual step, for a receiver whose particles lie side by side at full
    /// flow: the same look for fewer dabs.
    pub coarse: f32,
    /// The start speed comes from this row and not from a `ParticleSpeed` tweak.
    pub own_speed: bool,
    /// Lives, speeds and fades come from this row alone: the receiver has no such tweaks, and a project that holds any
    /// has them left from the preset the receiver replaced.
    pub own_shape: bool,
    /// Speeds are in the units of the mesh file the reference app imported, not the stored mesh's (which spans 2): a
    /// reach shrinks by the radius the reference app normalised the mesh with ([`in_file_units`]).
    pub scene_units: bool,
}

/// What a receiver not in the table does: particles leave the stroke every way with some wander.
const GENERIC: Behaviour = Behaviour {
    name: "generic",
    measured: false,
    per_click: 8.0,
    per_length: 0.0,
    budget: 0.0,
    speed: 0.25,
    speed_rand: 0.5,
    life: 1.0,
    life_rand: 0.8,
    gravity: 0.0,
    friction: 0.0,
    wander: 5.0,
    curl: 0.0,
    fan: std::f32::consts::TAU,
    heading: 0.0,
    away: false,
    kink_time: 0.0,
    kink_angle: 0.0,
    kink_branches: 0.0,
    kink_speed: 1.0,
    stop_speed: 0.0,
    view_bias: 0.0,
    scatter: 0.0,
    fade_size: 0.0,
    fade_opacity: 0.0,
    size: 1.0,
    flow: 1.0,
    coarse: 1.0,
    own_speed: false,
    own_shape: false,
    scene_units: false,
};

/// One row per receiver, fitted by eye and on statistics against the original's renders; `..GENERIC` is everything a
/// row doesn't say.
const TABLE: &[(&[&str], Behaviour)] = &[
    (&["laser"], Behaviour { name: "laser", measured: true, per_click: 10.0, view_bias: 1.0, wander: 1.0, speed_rand: 0.8, fade_size: 0.2, ..GENERIC }),
    (&["leak"], Behaviour { name: "leak", measured: true, per_click: 1.0, speed: 0.0, gravity: 0.5, friction: 0.5, wander: 0.0, size: 1.0, ..GENERIC }),
    (&["algea"], Behaviour { name: "algea", measured: true, per_click: 8.0, wander: 14.0, speed_rand: 0.5, ..GENERIC }),
    (
        &["brush"],
        Behaviour {
            name: "brush",
            measured: true,
            coarse: 3.0,
            own_shape: true,
            scene_units: true,
            per_click: 120.0,
            speed: 0.25,
            speed_rand: 0.0,
            life: 1.0,
            life_rand: 0.0,
            wander: 0.0,
            fade_size: 1.0,
            fade_opacity: 1.0,
            ..GENERIC
        },
    ),
    (
        &["glass", "glass2"],
        Behaviour {
            name: "glass",
            measured: true,
            own_shape: true,
            per_click: 50.0,
            speed: 0.25,
            speed_rand: 0.5,
            life: 1.0,
            life_rand: 0.8,
            wander: 0.5,
            fade_size: 0.1,
            fade_opacity: 0.5,
            ..GENERIC
        },
    ),
    (
        &["frac", "fracture", "fracture2"],
        Behaviour {
            name: "frac",
            measured: true,
            own_shape: true,
            per_click: 2.0,
            budget: 30.0,
            speed: 0.25,
            speed_rand: 0.0,
            life: 1.0,
            life_rand: 0.0,
            wander: 0.0,
            kink_time: 0.15,
            kink_angle: 1.6,
            kink_branches: 0.5,
            kink_speed: 1.0,
            fade_size: 1.0,
            fade_opacity: 1.0,
            ..GENERIC
        },
    ),
    (
        &["lightning"],
        Behaviour {
            name: "lightning",
            measured: true,
            own_shape: true,
            per_click: 1.0,
            speed: 0.3,
            life: 1.0,
            life_rand: 0.6,
            speed_rand: 0.2,
            heading: 1.45,
            fan: 1.0,
            wander: 8.0,
            fade_size: 0.5,
            fade_opacity: 0.2,
            ..GENERIC
        },
    ),
    (
        &["spiral"],
        Behaviour {
            name: "spiral",
            measured: true,
            own_shape: true,
            per_click: 1.5,
            speed: 0.3,
            life: 1.0,
            life_rand: 0.5,
            curl: 5.0,
            wander: 1.0,
            flow: 0.25,
            fade_size: 1.0,
            fade_opacity: 0.5,
            ..GENERIC
        },
    ),
    (
        &["rcvrust"],
        Behaviour {
            name: "rcvrust",
            measured: true,
            own_shape: true,
            per_click: 0.7,
            speed: 0.15,
            speed_rand: 0.8,
            life: 3.0,
            life_rand: 0.2,
            wander: 0.5,
            gravity: 0.3,
            friction: 0.3,
            fade_size: 1.0,
            fade_opacity: 1.0,
            fan: std::f32::consts::TAU,
            flow: 0.4,
            ..GENERIC
        },
    ),
    (
        &["defaultreceiver"],
        Behaviour {
            name: "default receiver",
            measured: true,
            own_speed: true,
            own_shape: true,
            per_click: 8.0,
            speed: 0.45,
            speed_rand: 0.6,
            life: 3.0,
            life_rand: 0.5,
            wander: 6.0,
            fade_size: 1.0,
            fade_opacity: 1.0,
            flow: 0.35,
            kink_time: 0.5,
            kink_angle: 0.6,
            kink_branches: 0.5,
            ..GENERIC
        },
    ),
    (
        &["rust"],
        Behaviour {
            name: "rust",
            measured: true,
            own_shape: true,
            per_click: 5.0,
            speed: 0.8,
            life: 3.0,
            life_rand: 0.5,
            wander: 0.5,
            gravity: 0.3,
            flow: 0.05,
            fade_size: 1.0,
            fade_opacity: 1.0,
            ..GENERIC
        },
    ),
    (
        &["rust2"],
        Behaviour {
            name: "rust2",
            measured: true,
            own_shape: true,
            per_click: 0.3,
            speed: 0.8,
            life: 3.0,
            life_rand: 0.5,
            wander: 0.5,
            gravity: 0.3,
            flow: 0.05,
            ..GENERIC
        },
    ),
    (&["rustdrop"], Behaviour { name: "rustdrop", measured: true, own_shape: true, per_click: 0.02, speed: 0.01, life: 1.0, ..GENERIC }),
    (
        &["veins"],
        Behaviour {
            name: "veins",
            measured: true,
            own_shape: true,
            per_click: 8.0,
            speed: 0.4,
            life: 1.0,
            life_rand: 0.5,
            away: true,
            fan: 0.8,
            wander: 6.0,
            fade_size: 0.0,
            fade_opacity: 0.2,
            ..GENERIC
        },
    ),
    (
        &["burn"],
        Behaviour {
            name: "burn",
            measured: false,
            own_shape: true,
            per_click: 4.0,
            speed: 0.15,
            life: 1.0,
            life_rand: 0.5,
            heading: 1.57,
            fan: 1.5,
            wander: 14.0,
            size: 2.0,
            flow: 0.7,
            ..GENERIC
        },
    ),
];

/// The script's name in a resource url: `shelf/RcvLeak?version=…` is `RcvLeak`.
pub fn script_name(url: &str) -> &str {
    let tail = url.rsplit('/').next().unwrap_or(url);
    tail.split('?').next().unwrap_or(tail)
}

/// A receiver's name as the table holds it. `RcvRust` behaves unlike `Rust`, so that prefix stays; the others are the
/// same with or without it (seen in renders).
fn row_name(url: &str) -> String {
    let lower = script_name(url).to_lowercase();
    let name = if lower == "rcvrust" { lower.as_str() } else { lower.trim_start_matches("rcv") };
    name.split([' ', '_']).collect()
}

/// The row for a receiver's url; unknown names get the generic row, never a refusal.
pub fn behaviour(receiver_url: &str) -> Behaviour {
    let name = row_name(receiver_url);
    TABLE.iter().find(|(names, _)| names.contains(&name.as_str())).map_or(GENERIC, |(_, b)| *b)
}

/// How an emitter places particles: how many per the receiver's count, and scattered how far around the stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmitterShape {
    pub name: &'static str,
    pub measured: bool,
    pub count: f32,
    pub scatter: f32,
}

/// The shape for an emitter's url; unknown names spawn on the stroke.
pub fn emitter_shape(url: &str) -> EmitterShape {
    match script_name(url).to_lowercase().as_str() {
        "single" | "emitsingle" => EmitterShape { name: "single", measured: true, count: 1.0, scatter: 0.0 },
        "spray" => EmitterShape { name: "spray", measured: true, count: 8.0, scatter: 0.02 },
        "emitpaint" => EmitterShape { name: "paint", measured: true, count: 8.0, scatter: 0.02 },
        "rain" | "manualrain" | "emitrain" | "emitmanrain" => EmitterShape { name: "rain", measured: true, count: 0.0, scatter: 0.0 },
        _ => EmitterShape { name: "generic", measured: false, count: 1.0, scatter: 0.0 },
    }
}

/// The value of tweak `id`, if finite.
fn tweak(list: &[Tweak], id: &str) -> Option<f32> {
    list.iter().find(|t| t.identifier == id).and_then(|t| match &t.value {
        TweakValue::Floats(v) => v.first().copied().filter(|x| x.is_finite()),
        _ => None,
    })
}

/// The physics emitter of `effect`'s brush, if it paints with particles.
pub fn emitter(effect: &PaintEffect) -> Option<&PhysicsEmitter> {
    match effect.brush.as_ref().map(|b| &b.emitter) {
        Some(StampEmitter::Physics(p)) => Some(p),
        _ => None,
    }
}

/// A receiver's numbers with the stored tweaks in, and the note naming what was approximated.
pub fn particles(p: &PhysicsEmitter) -> (ReplayParticles, String) {
    let mut row = behaviour(&p.receiver);
    let shape = emitter_shape(&p.emitter);
    let r = |id: &str| tweak(&p.receiver_tweaks, id);
    // how it spawns is the emitter's: its copy changes the render, the receiver's doesn't (measured)
    let spawn = |id: &str| tweak(&p.emitter_tweaks, id).or_else(|| tweak(&p.receiver_tweaks, id));
    if let Some(life) = r("ParticleLife").or_else(|| r("Lifetime")) {
        row.life = life;
    }
    if let Some(rand) = r("ParticleLifeRand").filter(|_| !row.own_shape) {
        row.life_rand = rand;
    }
    // a receiver moved by gravity alone has no start speed to set
    if let Some(speed) = r("ParticleSpeed").filter(|_| row.speed > 0.0 && !row.own_speed) {
        row.speed = speed;
    }
    if let Some(rand) = r("ParticleSpeedRand").filter(|_| !row.own_shape) {
        row.speed_rand = rand;
    }
    if let Some(factor) = r("SpeedFactorAtTrigger") {
        row.kink_speed = factor;
    }
    if let Some(ratio) = r("StopSpeedRatio") {
        row.stop_speed = ratio;
    }
    if let (Some(a), Some(b)) = (r("TriggerTimeMin"), r("TriggerTimeMax")) {
        row.kink_time = (a + b) * 0.5;
    }
    if let Some(deviation) = r("TriggerDeviation") {
        row.kink_angle = deviation * KINK_ANGLE_PER_DEVIATION;
    }
    if let Some(opacity) = r("ParticleOpacity") {
        row.flow *= opacity;
    }
    // `SpawnSpeed` doesn't change how many (5 … 30 measured), but particles too slow to reach the surface leave nothing
    let reaches = spawn("SpawnSpeed").is_none_or(|v| v >= MIN_SPAWN_SPEED);
    // a stored step of 0 never moves anything, and leaves nothing (seen in a project)
    let still = p.delta_time.is_some_and(|d| d <= 0.0);
    let rate = if reaches && !still { shape.count * spawn("SpawnRateFactor").unwrap_or(1.0).max(0.0) } else { 0.0 };
    let rate = |v: f32| (v * rate).clamp(0.0, MAX_RATE);
    let particles = ReplayParticles {
        per_point: rate(row.per_click),
        per_length: rate(row.per_length),
        budget: row.budget,
        speed: row.speed,
        speed_rand: row.speed_rand,
        life: row.life,
        life_rand: row.life_rand,
        gravity: row.gravity * r("GravityFactor").unwrap_or(1.0),
        friction: r("Friction").unwrap_or(row.friction).max(0.0),
        wander: row.wander,
        curl: row.curl,
        fan: row.fan,
        heading: row.heading,
        away: row.away,
        kink_time: row.kink_time,
        kink_angle: row.kink_angle,
        kink_branches: row.kink_branches,
        kink_speed: row.kink_speed,
        stop_speed: row.stop_speed,
        view_bias: row.view_bias,
        spread: r("Spread").filter(|_| row.view_bias != 0.0).unwrap_or(1.0),
        scatter: row.scatter + spawn("SpawnSpread").map_or(shape.scatter, |v| v * SPREAD_REACH),
        fade_size: r("FadeSize").filter(|_| !row.own_shape).unwrap_or(1.0 - row.fade_size),
        fade_size_rand: r("FadeSizeRand").filter(|_| !row.own_shape).unwrap_or(0.0),
        fade_flow: r("FadeOpacity").filter(|_| !row.own_shape).or_else(|| r("FadeOut")).unwrap_or(1.0 - row.fade_opacity),
        fade_flow_rand: r("FadeOpacityRand").filter(|_| !row.own_shape).unwrap_or(0.0),
        size: row.size,
        flow: row.flow,
        step: p.delta_time.filter(|d| *d > 1e-4).unwrap_or(DEFAULT_STEP).min(DEFAULT_STEP) * row.coarse,
        file_units: row.scene_units,
    };
    (particles, format!("{APPROXIMATE}: {} on {}", row.name, shape.name))
}

/// Puts the speeds of particle steps given in the units of the mesh file the reference app imported into the stored
/// mesh's: divided by `radius`, the radius the reference app normalised the mesh with (left as they are when it is not
/// known).
pub fn in_file_units(jobs: &mut [ReplayJob], radius: Option<f32>) {
    for p in jobs.iter_mut().flat_map(|j| &mut j.steps).filter_map(|s| s.particles.as_mut()).filter(|p| p.file_units) {
        if let Some(r) = radius.filter(|r| r.is_finite() && *r > 0.0) {
            p.speed /= r;
        }
        p.file_units = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn physics(receiver: &str, tweaks: &[(&str, f32)]) -> PhysicsEmitter {
        PhysicsEmitter {
            emitter: "shelf/EmitSingle?version=1".into(),
            receiver: format!("shelf/{receiver}?version=1"),
            delta_time: Some(0.01),
            receiver_tweaks: tweaks.iter().map(|(id, v)| Tweak { identifier: (*id).to_string(), value: TweakValue::Floats(vec![*v]) }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn receivers_are_found_by_name_and_unknown_ones_get_the_generic_row() {
        assert_eq!(behaviour("a.b/RcvFrac?version=1").name, "frac");
        assert_eq!(behaviour("shelf/Leak?version=2").name, "leak");
        assert_eq!(behaviour("tool/default_receiver?version=3").name, "default receiver");
        // the same word, a different behaviour
        assert_ne!(behaviour("x/RcvRust?v=1").name, behaviour("x/Rust?v=1").name);
        let unknown = behaviour("x/SomethingNew?v=1");
        assert_eq!((unknown.name, unknown.measured), ("generic", false));
        assert_eq!(emitter_shape("x/SomethingNew?v=1").name, "generic");
        assert_eq!(emitter_shape("x/Rain?v=1").count, 0.0);
    }

    #[test]
    fn tweaks_set_the_numbers_and_the_note_names_the_row() {
        let (p, note) = particles(&physics("RcvLeak", &[("ParticleLife", 2.0), ("GravityFactor", 2.0), ("ParticleOpacity", 0.5)]));
        assert_eq!(note, "particles approximated: leak on single");
        assert_eq!((p.life, p.gravity, p.flow, p.per_point, p.speed), (2.0, 1.0, 0.5, 1.0, 0.0));
        assert_eq!(p.step, 0.01);
        // a receiver with its own shape keeps its fades whatever the project stores
        let (frac, _) = particles(&physics("RcvFrac", &[("FadeSize", 0.1), ("TriggerDeviation", 1.0)]));
        assert_eq!((frac.fade_size, frac.kink_angle, frac.budget), (0.0, 1.6, 30.0));
    }

    #[test]
    fn too_slow_a_spawn_or_a_stored_step_of_zero_spawns_nothing() {
        let mut p = physics("RcvLeak", &[]);
        p.emitter_tweaks = vec![Tweak { identifier: "SpawnSpeed".into(), value: TweakValue::Floats(vec![2.0]) }];
        assert_eq!(particles(&p).0.per_point, 0.0);
        let mut p = physics("RcvLeak", &[]);
        p.delta_time = Some(0.0);
        assert_eq!(particles(&p).0.per_point, 0.0);
        // an unreadable step is the usual one
        let mut p = physics("RcvLeak", &[]);
        p.delta_time = Some(f32::NAN);
        assert_eq!((particles(&p).0.per_point, particles(&p).0.step), (1.0, 0.01));
    }

    #[test]
    fn hostile_tweaks_stay_bounded() {
        let mut p = physics("RcvLeak", &[("ParticleLife", f32::INFINITY), ("Friction", -5.0)]);
        p.emitter_tweaks = vec![Tweak { identifier: "SpawnRateFactor".into(), value: TweakValue::Floats(vec![1e30]) }];
        let (q, _) = particles(&p);
        assert_eq!((q.life, q.friction, q.per_point), (1.0, 0.0, MAX_RATE));
    }
}
