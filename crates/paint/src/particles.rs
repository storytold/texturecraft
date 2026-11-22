//! Particle brush: each stamp emits a few particles that slide down the surface under gravity
//! (world −Y) and fall off overhangs, painting a small dab per simulation step. Deterministic:
//! particles are seeded by the stamp seed.
//!
//! [`simulate`] runs a whole recorded stroke's particles instead, with speed, wander, curl, kinks and fades
//! ([`Physics`]): the replay of imported particle strokes uses it; live painting does not.

use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Bvh, Mesh, Surface, surface_at};
use texturecraft_texel::noise::pcg3d;

use crate::brush::Particles;
use crate::stroke::{PickCtx, Stamp};

/// Particle dab radius relative to the brush stamp.
const PARTICLE_SIZE: f32 = 0.35;

/// The surface hit by a ray, if any within `tmax`.
fn cast(ctx: &PickCtx, o: Vec3, d: Vec3, tmax: f32) -> Option<Surface> {
    let hit = ctx.bvh.closest(o, d, tmax)?;
    surface_at(ctx.mesh, hit.tri, hit.u, hit.v)
}

/// Project `p` onto the surface along the normal `n` (within `reach`).
fn snap(ctx: &PickCtx, p: Vec3, n: Vec3, reach: f32) -> Option<Surface> {
    cast(ctx, p + n * reach, -n, reach * 2.0)
}

/// Particle dabs for one brush stamp.
pub fn emit(ctx: &PickCtx, p: &Particles, s: &Stamp) -> Vec<Stamp> {
    let size = s.radius * PARTICLE_SIZE;
    let step = size * p.gravity;
    let fall = s.radius * 20.0;
    let unit = |v: u32| v as f32 / u32::MAX as f32;
    let mut out = Vec::new();
    for k in 0..p.count {
        let r = pcg3d(s.seed, k, 0xD057);
        let (a, d) = (unit(r[0]) * std::f32::consts::TAU, unit(r[1]).sqrt());
        let start = s.center + (s.tangent * a.cos() + s.bitangent * a.sin()) * d * s.radius;
        let Some(mut at) = snap(ctx, start, s.normal, s.radius) else { continue };
        for life in 0..p.lifetime {
            let tangent = at.nrm.any_orthonormal_vector();
            out.push(Stamp {
                center: at.pos,
                normal: at.nrm,
                tangent,
                bitangent: at.nrm.cross(tangent),
                radius: size,
                flow: s.flow * (1.0 - life as f32 / p.lifetime as f32),
                seed: s.seed.wrapping_mul(17).wrapping_add(k),
                ..*s
            });
            if step <= 0.0 {
                break;
            }
            let slide = Vec3::NEG_Y - at.nrm * at.nrm.dot(Vec3::NEG_Y);
            let next = if slide.length() > 0.05 {
                // Slide downhill; past an edge the particle drops onto what lies below.
                let to = at.pos + slide.normalize() * step;
                snap(ctx, to, at.nrm, step).or_else(|| cast(ctx, to + at.nrm * step, Vec3::NEG_Y, fall))
            } else if at.nrm.y < 0.0 {
                // Hanging under a ceiling: fall straight down.
                cast(ctx, at.pos + at.nrm * step, Vec3::NEG_Y, fall)
            } else {
                None // resting on flat ground
            };
            match next {
                Some(h) => at = h,
                None => break,
            }
        }
    }
    out
}

/// How long a random turn of [`simulate`] lasts, in seconds.
const WANDER_TIME: f32 = 0.15;
/// A dab under a texel still lays a whole texel at its flow: radius at least this many texels.
const MIN_DAB_TEXELS: f32 = 0.75;
/// Kinks branch at most this many generations deep (a cost guard).
const MAX_GENERATIONS: u32 = 12;
/// The seed spreader of [`simulate`]'s generator (the 64-bit golden ratio).
const RNG_SPREAD: u64 = 0x9E37_79B9_7F4A_7C15;
/// A step divides into at most this many.
const MAX_SUBSTEPS: f32 = 32.0;
/// Most dabs one stroke's particles may lay (a cost guard).
pub const MAX_DABS: usize = 6_000_000;
/// Most particles one stroke may hold, branches included (a cost guard).
const MAX_PARTICLES: usize = 200_000;
/// Fewest seconds between dabs.
const MIN_STEP: f32 = 1e-4;

/// How a stroke's particles behave in [`simulate`]. Lengths are mesh units, times seconds, angles radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Physics {
    /// Particles per point the pen recorded, per mesh unit of the stroke's path, and at most this many in one stroke (0:
    /// no limit, thinned evenly over the path).
    pub per_point: f32,
    pub per_length: f32,
    pub budget: f32,
    /// Start speed and life, each lowered per particle by up to this share (`speed · (1 − speed_rand·u)`).
    pub speed: f32,
    pub speed_rand: f32,
    pub life: f32,
    pub life_rand: f32,
    /// Acceleration down the mesh's −Y along the surface (negative: up).
    pub gravity: f32,
    /// Share of the speed lost per second.
    pub friction: f32,
    /// Smooth random turning: how strongly the rate of turn drifts (radians per second, per square root of a second).
    pub wander: f32,
    /// Steady turning, radians per second, either way per particle.
    pub curl: f32,
    /// Particles start in a fan this wide around `heading` (from the surface's +X toward +Y as seen from outside); with
    /// `away`, around the direction away from the stroke's middle instead.
    pub fan: f32,
    pub heading: f32,
    pub away: bool,
    /// A sudden turn every `kink_time` seconds or so (0: none), up to `kink_angle` either way, with `kink_branches`
    /// extra particles from it; the speed is multiplied by `kink_speed`, and a particle slower than `stop_speed` of its
    /// start speed after a kink ends.
    pub kink_time: f32,
    pub kink_angle: f32,
    pub kink_branches: f32,
    pub kink_speed: f32,
    pub stop_speed: f32,
    /// How much of the view's direction along the surface a particle keeps where it lands (0: none), and the share of
    /// its speed that goes its own way.
    pub view_bias: f32,
    pub spread: f32,
    /// Particles start up to this far from the stroke.
    pub scatter: f32,
    /// Share of the size and of the flow lost over a life, each lowered per particle by up to its `_rand` share.
    pub fade_size: f32,
    pub fade_size_rand: f32,
    pub fade_flow: f32,
    pub fade_flow_rand: f32,
    /// Multiplies the dab's size and flow.
    pub size: f32,
    pub flow: f32,
    /// Seconds between dabs.
    pub step: f32,
}

/// What [`simulate`]'s particles move over: the mesh, the side of a texel on it, and how far from the surface a
/// particle may get before it has left it (past an open edge).
pub struct ParticleSurface<'a> {
    pub mesh: &'a Mesh,
    pub bvh: &'a Bvh,
    pub texel: f32,
    pub gap: f32,
}

/// A place on the surface: point and unit normal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePoint {
    pub at: Vec3,
    pub normal: Vec3,
}

impl ParticleSurface<'_> {
    /// The surface under `p`: `p` on the tangent plane of the nearest surface point (so a particle keeps going past an
    /// edge until it is `gap` away), with that point's normal; `None` further than `gap` (past an open edge or out in
    /// the air).
    fn at(&self, p: Vec3) -> Option<SurfacePoint> {
        let hit = self.bvh.nearest(p, self.gap.max(1e-6), &|_| true)?;
        let s = surface_at(self.mesh, hit.tri, hit.u, hit.v)?;
        let at = p - s.nrm * (p - s.pos).dot(s.nrm);
        (at.is_finite() && s.nrm.is_finite()).then_some(SurfacePoint { at, normal: s.nrm })
    }
}

/// One stroke for [`simulate`]: where the pen's recorded points lie on the surface, its path in short even steps
/// (`path_step` mesh units apart), its seed, and the direction the view looked along (3D-view strokes).
pub struct ParticleStroke<'a> {
    pub points: &'a [SurfacePoint],
    pub path: &'a [SurfacePoint],
    pub path_step: f32,
    pub seed: u32,
    pub view: Option<Vec3>,
}

/// The dab a particle lays at full size: radius (mesh units), flow, flow jitter (a dab's flow is `1 − jitter·u` of it)
/// and the tip's hardness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleDab {
    pub radius: f32,
    pub flow: f32,
    pub flow_jitter: f32,
    pub hardness: f32,
}

/// A deterministic generator of even numbers in 0..1 (xorshift).
struct Rng(u64);

impl Rng {
    fn new(seed: u32) -> Rng {
        Rng(u64::from(seed).wrapping_mul(RNG_SPREAD) | 1)
    }
    fn unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
    /// −1..1.
    fn both(&mut self) -> f32 {
        2.0 * self.unit() - 1.0
    }
    /// Normal, mean 0, deviation 1.
    fn normal(&mut self) -> f32 {
        let (a, b) = (self.unit(), self.unit());
        (-2.0 * (1.0 - b).max(1e-7).ln()).sqrt() * (std::f32::consts::TAU * a).cos()
    }
    /// `n` rounded up or down at random, by its fraction.
    fn whole(&mut self, n: f32) -> usize {
        let n = if n.is_finite() { n.clamp(0.0, MAX_PARTICLES as f32) } else { 0.0 };
        n.floor() as usize + usize::from(self.unit() < n.fract())
    }
}

/// `a` on the plane of the unit normal `n`.
fn in_plane(a: Vec3, n: Vec3) -> Vec3 {
    a - n * a.dot(n)
}

/// `a` (on the plane of the unit normal `n`) turned by `angle` around `n`.
fn turned(a: Vec3, n: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    a * c + n.cross(a) * s
}

/// A direction along the surface at `n`, `angle` from the surface's +X toward +Y.
fn heading(n: Vec3, angle: f32) -> Vec3 {
    let x = in_plane(Vec3::X, n).try_normalize().or_else(|| in_plane(Vec3::Z, n).try_normalize()).unwrap_or(Vec3::X);
    let (s, c) = angle.sin_cos();
    x * c + n.cross(x) * s
}

/// One particle.
#[derive(Debug, Clone, Copy)]
struct Particle {
    at: Vec3,
    velocity: Vec3,
    start_speed: f32,
    age: f32,
    life: f32,
    next_kink: f32,
    /// The steady turn's sign and share, and the rate of the random turn now.
    curl: f32,
    turning: f32,
    fade_size: f32,
    fade_flow: f32,
    generation: u32,
}

/// A new particle at `on`.
fn born(on: &SurfacePoint, p: &Physics, middle: Vec3, view: Option<Vec3>, rng: &mut Rng) -> Particle {
    let n = on.normal;
    let spread = p.fan * rng.both() * 0.5;
    let base = match p.away {
        true => match in_plane(on.at - middle, n).try_normalize() {
            Some(d) => turned(d, n, spread),
            None => heading(n, rng.unit() * std::f32::consts::TAU),
        },
        false => heading(n, p.heading + spread),
    } * p.spread;
    // lands with what the flight keeps along the surface
    let base = match view {
        Some(d) if p.view_bias != 0.0 => base + in_plane(d, n) * p.view_bias,
        _ => base,
    };
    let speed = p.speed * (1.0 - p.speed_rand * rng.unit());
    let life = p.life * (1.0 - p.life_rand * rng.unit());
    let curl = if rng.unit() < 0.5 { 1.0 } else { -1.0 } * (0.5 + 0.5 * rng.unit());
    Particle {
        at: on.at,
        velocity: base * speed,
        start_speed: speed.max(1e-6),
        age: 0.0,
        life: life.max(1e-3),
        next_kink: if p.kink_time > 0.0 { p.kink_time * (0.5 + rng.unit()) } else { f32::MAX },
        curl,
        turning: 0.0,
        fade_size: p.fade_size * (1.0 - p.fade_size_rand * rng.unit()),
        fade_flow: p.fade_flow * (1.0 - p.fade_flow_rand * rng.unit()),
        generation: 0,
    }
}

/// Runs one stroke's particles over `surface`, handing each dab to `lay` in order (the tip along the particle's
/// motion). Particles spawn at the stroke's recorded points and along its path, travel along the surface (over edges as
/// the mesh bends, ending past open edges) and lay fading dabs every [`Physics::step`] seconds, closer where they move
/// further than a dab's reach. Deterministic: the stroke's seed draws everything.
pub fn simulate(surface: &ParticleSurface, physics: &Physics, stroke: &ParticleStroke, dab: &ParticleDab, mut lay: impl FnMut(Stamp)) {
    let p = physics;
    let Some(first) = stroke.path.first().or(stroke.points.first()) else { return };
    let mut rng = Rng::new(stroke.seed);
    let all = stroke.path.len().max(1) as f32;
    let middle = stroke.path.iter().fold(Vec3::ZERO, |m, s| m + s.at / all);
    let middle = if stroke.path.is_empty() { first.at } else { middle };
    let view = stroke.view.and_then(Vec3::try_normalize);
    let mut queue: Vec<Particle> = Vec::new();
    let spawn = |place: &SurfacePoint, rng: &mut Rng, queue: &mut Vec<Particle>| {
        if queue.len() >= MAX_PARTICLES {
            return;
        }
        let mut at = place.at;
        if p.scatter > 0.0 {
            at += heading(place.normal, rng.unit() * std::f32::consts::TAU) * (p.scatter * rng.unit().sqrt());
        }
        if let Some(on) = surface.at(at).or_else(|| surface.at(place.at)) {
            queue.push(born(&on, p, middle, view, rng));
        }
    };
    // each recorded point sends its own particles, then each step of the path
    for place in stroke.points {
        for _ in 0..rng.whole(p.per_point) {
            spawn(place, &mut rng, &mut queue);
        }
    }
    let per_step = p.per_length * stroke.path_step;
    for place in stroke.path.iter().skip(1) {
        for _ in 0..rng.whole(per_step) {
            spawn(place, &mut rng, &mut queue);
        }
    }
    if p.budget > 0.0 && queue.len() as f32 > p.budget {
        let keep = p.budget / queue.len() as f32;
        queue.retain(|_| rng.unit() < keep);
    }
    let radius = dab.radius * p.size;
    let step = if p.step.is_finite() { p.step.max(MIN_STEP) } else { MIN_STEP };
    let (mut laid, mut made) = (0usize, queue.len());
    while let Some(mut q) = queue.pop() {
        let mut along = q.velocity.try_normalize().unwrap_or(Vec3::X);
        loop {
            if q.age.is_nan() || q.age > q.life || laid >= MAX_DABS {
                break;
            }
            let Some(on) = surface.at(q.at) else { break };
            let n = on.normal;
            let t = q.age / q.life;
            let dab_radius = radius * (1.0 - q.fade_size * t).max(0.0);
            let jitter = 1.0 - dab.flow_jitter * rng.unit();
            let dab_flow = (dab.flow * p.flow * jitter * (1.0 - q.fade_flow * t).max(0.0)).clamp(0.0, 1.0);
            if dab_radius > 0.0 && dab_flow > 0.0 {
                let tangent = in_plane(along, n).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector());
                lay(Stamp {
                    center: on.at,
                    normal: n,
                    tangent,
                    bitangent: n.cross(tangent),
                    radius: dab_radius.max(surface.texel * MIN_DAB_TEXELS),
                    flow: dab_flow,
                    hardness: dab.hardness,
                    depth: 1.0,
                    seed: u32::try_from(laid).unwrap_or(u32::MAX),
                    source: None,
                });
            }
            laid += 1;
            // along the surface, keeping the speed
            let speed = q.velocity.length();
            let mut v = in_plane(q.velocity, n).try_normalize().map_or(q.velocity, |d| d * speed);
            // fast particles still draw lines: no step goes further than a dab's reach
            let reach = dab_radius.max(surface.texel);
            let dt = step / (v.length() * step / reach).ceil().clamp(1.0, MAX_SUBSTEPS);
            v += in_plane(Vec3::NEG_Y, n) * (p.gravity * dt);
            v *= (1.0 - p.friction * dt).max(0.0);
            q.turning += p.wander * dt.sqrt() * rng.normal() - q.turning * (dt / WANDER_TIME).min(1.0);
            let turn = (q.turning + p.curl * q.curl) * dt;
            if turn != 0.0 {
                v = turned(v, n, turn);
            }
            q.age += dt;
            if q.age >= q.next_kink {
                let speed = v.length() * p.kink_speed;
                let d = v.try_normalize().unwrap_or(along);
                for _ in 0..rng.whole(p.kink_branches) {
                    if made < MAX_PARTICLES && q.generation < MAX_GENERATIONS {
                        let out = turned(d, n, p.kink_angle * rng.both());
                        queue.push(Particle { velocity: out * speed, next_kink: q.age + p.kink_time * (0.5 + rng.unit()), generation: q.generation + 1, ..q });
                        made += 1;
                    }
                }
                v = turned(d, n, p.kink_angle * rng.both()) * speed;
                q.next_kink = q.age + p.kink_time * (0.5 + rng.unit());
                q.generation += 1;
                if speed < p.stop_speed * q.start_speed {
                    break;
                }
            }
            if !v.is_finite() {
                break;
            }
            q.velocity = v;
            along = v.try_normalize().unwrap_or(along);
            q.at = on.at + v * dt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use texturecraft_geom::glam::Vec2;
    use texturecraft_geom::{Bvh, OrbitCamera, primitives};

    #[test]
    fn particles_slide_down_a_wall_deterministically() {
        // A vertical wall: the plane rotated to face +Z
        let mut mesh = primitives::plane(2.0, 8);
        for position in &mut mesh.positions {
            *position = Vec3::new(position.x, -position.z, 0.0);
        }
        mesh.normals.fill(Vec3::Z);
        let bvh = Bvh::new(&mesh);
        let camera = OrbitCamera::default();
        let context = PickCtx { mesh: &mesh, bvh: &bvh, camera: &camera, viewport: Vec2::new(100.0, 100.0) };
        let stamp = Stamp {
            center: Vec3::ZERO,
            normal: Vec3::Z,
            tangent: Vec3::X,
            bitangent: Vec3::Y,
            radius: 0.1,
            flow: 1.0,
            hardness: 0.5,
            depth: 1.0,
            seed: 3,
            source: None,
        };
        let particles = Particles { enabled: true, count: 4, lifetime: 20, gravity: 1.0 };

        let dabs = emit(&context, &particles, &stamp);

        assert_eq!(dabs, emit(&context, &particles, &stamp));
        assert_eq!(dabs.len(), 80);
        let lowest = dabs.iter().map(|d| d.center.y).fold(f32::MAX, f32::min);
        assert!(lowest < -0.4, "{lowest}");
        assert!(dabs.iter().all(|d| d.center.z.abs() < 1e-3));
    }

    /// A 2 × 2 wall facing +Z, its +Y up.
    fn wall() -> texturecraft_geom::Mesh {
        let mut mesh = primitives::plane(2.0, 16);
        for position in &mut mesh.positions {
            *position = Vec3::new(position.x, -position.z, 0.0);
        }
        mesh.normals.fill(Vec3::Z);
        mesh
    }

    /// Particles that stay where they land and fade out over one second, one per recorded point.
    fn still() -> Physics {
        Physics {
            per_point: 1.0,
            per_length: 0.0,
            budget: 0.0,
            speed: 0.0,
            speed_rand: 0.0,
            life: 1.0,
            life_rand: 0.0,
            gravity: 0.0,
            friction: 0.0,
            wander: 0.0,
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
            spread: 1.0,
            scatter: 0.0,
            fade_size: 0.0,
            fade_size_rand: 0.0,
            fade_flow: 0.0,
            fade_flow_rand: 0.0,
            size: 1.0,
            flow: 1.0,
            step: 0.01,
        }
    }

    const DAB: ParticleDab = ParticleDab { radius: 0.01, flow: 1.0, flow_jitter: 0.0, hardness: 0.5 };

    /// The dabs of `physics` for a stroke whose recorded points and path all lie at `at` on the wall.
    fn run(mesh: &texturecraft_geom::Mesh, physics: &Physics, at: &[Vec3], seed: u32) -> Vec<Stamp> {
        let bvh = Bvh::new(mesh);
        let surface = ParticleSurface { mesh, bvh: &bvh, texel: 2.0 / 256.0, gap: 2.5 * 2.0 / 256.0 };
        let places: Vec<SurfacePoint> = at.iter().map(|p| SurfacePoint { at: *p, normal: Vec3::Z }).collect();
        let stroke = ParticleStroke { points: &places, path: &places, path_step: 0.01, seed, view: None };
        let mut out = Vec::new();
        simulate(&surface, physics, &stroke, &DAB, |d| out.push(d));
        out
    }

    fn mean(dabs: &[Stamp]) -> Vec3 {
        dabs.iter().fold(Vec3::ZERO, |m, d| m + d.center) / dabs.len().max(1) as f32
    }

    #[test]
    fn a_leak_runs_down_the_wall_and_a_run_repeats() {
        let leak = Physics { speed: 0.0, gravity: 0.5, friction: 0.5, ..still() };
        let dabs = run(&wall(), &leak, &[Vec3::ZERO; 4], 7);
        assert_eq!(dabs.len(), 4 * 101, "a dab every 0.01 s for a second, the first where it starts");
        assert_eq!(dabs, run(&wall(), &leak, &[Vec3::ZERO; 4], 7));
        let m = mean(&dabs);
        assert!(m.y < -0.05 && m.x.abs() < 1e-3, "{m}");
        assert!(dabs.iter().all(|d| d.center.z.abs() < 1e-4 && d.normal == Vec3::Z));
    }

    #[test]
    fn a_seed_changes_the_pattern_not_the_amount() {
        let algae = Physics { per_point: 8.0, speed: 0.25, speed_rand: 0.5, life_rand: 0.8, wander: 14.0, ..still() };
        let line = [Vec3::new(-0.3, 0.0, 0.0), Vec3::new(0.3, 0.0, 0.0)];
        let (a, b) = (run(&wall(), &algae, &line, 1), run(&wall(), &algae, &line, 2));
        assert_ne!(a, b);
        assert!((a.len() as f32 / b.len() as f32 - 1.0).abs() < 0.5, "{} {}", a.len(), b.len());
    }

    #[test]
    fn more_recorded_points_send_more_particles_and_longer_lives_reach_further() {
        let leak = Physics { gravity: 0.5, ..still() };
        assert!(run(&wall(), &leak, &[Vec3::ZERO; 40], 3).len() > 3 * run(&wall(), &leak, &[Vec3::ZERO; 2], 3).len());
        let reach = |life: f32| mean(&run(&wall(), &Physics { life, ..leak }, &[Vec3::ZERO; 4], 5)).y;
        assert!(reach(2.0) < reach(0.5) - 0.05, "{} {}", reach(2.0), reach(0.5));
    }

    #[test]
    fn particles_end_past_an_open_edge_and_off_the_mesh_nothing_happens() {
        // sent straight at the right edge: none lays a dab beyond it
        let fast = Physics { speed: 1.0, fan: 0.0, life: 2.0, ..still() };
        let dabs = run(&wall(), &fast, &[Vec3::new(0.9, 0.0, 0.0)], 1);
        assert!(!dabs.is_empty() && dabs.iter().all(|d| d.center.x <= 1.0 + 2.5 * 2.0 / 256.0));
        assert!(dabs.len() < 30, "{}", dabs.len());
        assert!(run(&wall(), &fast, &[Vec3::new(5.0, 5.0, 0.0)], 1).is_empty());
    }

    #[test]
    fn kinks_branch_and_fades_shrink_the_dabs() {
        let straight = Physics { speed: 0.25, fan: 0.0, ..still() };
        let kinked = Physics { kink_time: 0.15, kink_angle: 1.6, kink_branches: 1.0, ..straight };
        assert!(run(&wall(), &kinked, &[Vec3::ZERO], 4).len() > 2 * run(&wall(), &straight, &[Vec3::ZERO], 4).len());
        let fading = run(&wall(), &Physics { fade_size: 1.0, fade_flow: 1.0, ..straight }, &[Vec3::ZERO], 4);
        let (first, last) = (fading[0], fading[fading.len() - 1]);
        assert!(last.radius < first.radius && last.flow < 0.1 * first.flow, "{first:?} {last:?}");
        // the tip lies along the motion (+X here)
        assert!(first.tangent.x > 0.99);
    }

    #[test]
    fn hostile_numbers_lay_nothing_or_stop() {
        let bad = Physics { per_point: f32::NAN, per_length: f32::INFINITY, step: f32::NAN, life: f32::NAN, ..still() };
        assert!(run(&wall(), &bad, &[Vec3::ZERO; 3], 1).len() <= 3);
        let huge = Physics { per_point: 1e30, life: 0.0, ..still() };
        assert!(run(&wall(), &huge, &[Vec3::ZERO], 1).len() <= MAX_PARTICLES);
    }
}
