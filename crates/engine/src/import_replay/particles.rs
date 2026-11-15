//! Particle strokes: each stroke's particles spawn at its recorded points and along its path on the mesh, move by
//! [`texturecraft_paint::particles::simulate`], and their dabs are laid by the same rasteriser as every other replayed
//! dab (a texel's distance along the surface, no culling: particles lie on the surface they reach). Symmetry copies
//! and the stroke's own dabs are not laid (measured: a mirrored stroke does not mirror its particles).

use texturecraft_geom::glam::{Vec2, Vec3};
use texturecraft_import::replay::{ReplayBrush, ReplayParticles, ReplayStroke, ReplayTip, StrokeSpace};
use texturecraft_paint::particles::{ParticleDab, ParticleStroke, ParticleSurface, Physics, SurfacePoint, simulate};
use texturecraft_paint::{Facing, PickCtx, RasterCtx, Stamp, StrokeCanvas, pick, rasterize};

use super::{SetMesh, Tip, ViewCamera, mesh_dabs, raster_ctx, uv_dabs};

/// Spawn places along a stroke lie this far apart, in mesh units.
pub const PATH_STEP: f32 = 0.01;
/// A particle further than this many samples from the surface has left it (past an open edge).
const EDGE_SAMPLES: f32 = 2.5;
/// The surface is sampled at most this finely per side of the texture for that test: a gap of the same size in mesh
/// units at every resolution above it.
const EDGE_RESOLUTION: f32 = 512.0;
/// Dabs laid per batch.
const BATCH: usize = 4096;

/// The simulation's numbers for `p`.
fn physics(p: &ReplayParticles) -> Physics {
    Physics {
        per_point: p.per_point,
        per_length: p.per_length,
        budget: p.budget,
        speed: p.speed,
        speed_rand: p.speed_rand,
        life: p.life,
        life_rand: p.life_rand,
        gravity: p.gravity,
        friction: p.friction,
        wander: p.wander,
        curl: p.curl,
        fan: p.fan,
        heading: p.heading,
        away: p.away,
        kink_time: p.kink_time,
        kink_angle: p.kink_angle,
        kink_branches: p.kink_branches,
        kink_speed: p.kink_speed,
        stop_speed: p.stop_speed,
        view_bias: p.view_bias,
        spread: p.spread,
        scatter: p.scatter,
        fade_size: p.fade_size,
        fade_size_rand: p.fade_size_rand,
        fade_flow: p.fade_flow,
        fade_flow_rand: p.fade_flow_rand,
        size: p.size,
        flow: p.flow,
        step: p.step,
    }
}

/// A plain round brush [`PATH_STEP`] wide that lays a dab every [`PATH_STEP`]: where a stroke's path lies on the mesh.
fn path_brush(brush: &ReplayBrush) -> ReplayBrush {
    ReplayBrush {
        radius: PATH_STEP,
        spacing: 0.5,
        flow: 1.0,
        size_pressure: None,
        flow_pressure: None,
        size_jitter: 0.0,
        flow_jitter: 0.0,
        position_jitter: 0.0,
        rotation_jitter: 0.0,
        follow: false,
        culling: None,
        tip: ReplayTip::Round { hardness: 1.0 },
        cycle: Vec::new(),
        cycle_drawn_start: false,
        ..brush.clone()
    }
}

/// Where `stroke`'s recorded points and its path (every [`PATH_STEP`]) lie on the set, and the direction its view looked
/// along.
fn places(set: &SetMesh, brush: &ReplayBrush, stroke: &ReplayStroke) -> (Vec<SurfacePoint>, Vec<SurfacePoint>, Option<Vec3>) {
    let brush = path_brush(brush);
    let Ok(tip) = Tip::of(&brush) else { return (Vec::new(), Vec::new(), None) };
    let plain = ReplayStroke { lazy: 0.0, copies: 0, ..stroke.clone() };
    let on = |d: &Stamp| SurfacePoint { at: d.center, normal: d.normal };
    let (mut points, mut path) = (Vec::with_capacity(stroke.points.len()), Vec::new());
    match &stroke.space {
        StrokeSpace::Mesh { view, projection } => {
            let Some(camera) = ViewCamera::new(view, projection, set.center, set.radius) else { return (points, path, None) };
            let ctx = PickCtx { mesh: &set.mesh, bvh: &set.bvh, camera: &camera.orbit, viewport: camera.viewport };
            points.extend(stroke.points.iter().filter_map(|p| pick(&ctx, camera.screen(p.position))).map(|h| SurfacePoint { at: h.pos, normal: h.nrm }));
            mesh_dabs(set, &brush, &tip, &plain, &camera, false, |dabs| path.extend(dabs.iter().map(on)));
            // the view looks down its −z
            let view = Vec3::new(-view[2], -view[6], -view[10]);
            (points, path, view.is_finite().then_some(view))
        }
        StrokeSpace::Uv => {
            let res = set.resolution as f32;
            for p in &stroke.points {
                let t = Vec2::from_array(p.position) * res;
                if t.x >= 0.0
                    && t.y >= 0.0
                    && t.x < res
                    && t.y < res
                    && let Some(s) = set.gbuf.surface(&set.mesh, t.y as usize * set.resolution as usize + t.x as usize)
                {
                    points.push(SurfacePoint { at: s.pos, normal: s.nrm });
                }
            }
            let world_per_uv = set.world_per_texel * res;
            path.extend(uv_dabs(set, &brush, &tip, &plain).iter().filter_map(|d| d.on_mesh(&set.mesh, &set.gbuf, world_per_uv)).map(|d| on(&d)));
            (points, path, None)
        }
    }
}

/// Lays the dabs of `stroke`'s particles into `canvas`.
pub(super) fn lay(set: &SetMesh, brush: &ReplayBrush, tip: &Tip, particles: &ReplayParticles, stroke: &ReplayStroke, canvas: &mut StrokeCanvas) {
    let (points, path, view) = places(set, brush, stroke);
    let samples = (set.resolution as f32 / EDGE_RESOLUTION).max(1.0);
    let surface = ParticleSurface { mesh: &set.mesh, bvh: &set.bvh, texel: set.world_per_texel, gap: EDGE_SAMPLES * samples * set.world_per_texel };
    let particle_stroke = ParticleStroke { points: &points, path: &path, path_step: PATH_STEP, seed: stroke.seed, view };
    let dab = ParticleDab { radius: brush.radius, flow: brush.flow, flow_jitter: brush.flow_jitter, hardness: tip.hardness };
    let ctx = RasterCtx { facing: Facing::Any, ..raster_ctx(set, brush, set.center, tip.shape()) };
    let mut batch = Vec::with_capacity(BATCH);
    simulate(&surface, &physics(particles), &particle_stroke, &dab, |d| {
        batch.push(d);
        if batch.len() >= BATCH {
            rasterize(canvas, &ctx, &batch);
            batch.clear();
        }
    });
    rasterize(canvas, &ctx, &batch);
}

#[cfg(test)]
mod tests {
    use texturecraft_core::{Channel, LayerId};
    use texturecraft_geom::glam::Mat4;
    use texturecraft_geom::{Mesh, primitives};
    use texturecraft_import::replay::{PaintMode, ReplayJob, ReplayScene, ReplayStep, ReplayTarget, ReplayedPixels, StrokePoint, StrokeReplayer, TipAlignment};
    use texturecraft_texel::from_u16;

    use super::*;
    use crate::import_replay::BrushReplayer;

    const RES: u32 = 128;

    /// A 2 × 2 wall at z = 0 facing +z, its top row up.
    fn wall() -> Mesh {
        let mut m = primitives::plane(2.0, 16);
        for p in &mut m.positions {
            *p = Vec3::new(p.x, -p.z, 0.0);
        }
        m.normals.fill(Vec3::Z);
        m
    }

    /// A leak: particles that run down the surface for a second.
    fn leak() -> ReplayParticles {
        ReplayParticles {
            per_point: 1.0,
            per_length: 0.0,
            budget: 0.0,
            speed: 0.0,
            speed_rand: 0.0,
            life: 1.0,
            life_rand: 0.0,
            gravity: 1.0,
            friction: 0.5,
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
            file_units: false,
        }
    }

    fn step(space: StrokeSpace, particles: Option<ReplayParticles>) -> ReplayStep {
        let brush = ReplayBrush {
            radius: 0.03,
            spacing: 0.1,
            flow: 1.0,
            size_pressure: None,
            flow_pressure: None,
            size_jitter: 0.0,
            flow_jitter: 0.0,
            position_jitter: 0.0,
            position_both: false,
            position_normal: false,
            rotation: 0.0,
            rotation_jitter: 0.0,
            follow: false,
            alignment: TipAlignment::Tangent,
            culling: None,
            tip: ReplayTip::Round { hardness: 0.9 },
            cycle: vec![],
            cycle_drawn_start: false,
            up: None,
        };
        let points = match space {
            StrokeSpace::Uv => vec![[0.5, 0.5]; 3],
            StrokeSpace::Mesh { .. } => vec![[0.0, 0.0]; 3],
        };
        let stroke = ReplayStroke {
            space,
            points: points.into_iter().map(|position| StrokePoint { position, pressure: 1.0 }).collect(),
            seed: 3,
            lazy: 0.0,
            copies: 0,
            copy_matrix: [0.0; 16],
        };
        ReplayStep {
            mode: PaintMode::Paint,
            values: vec![(Channel::BaseColor, [1.0, 0.0, 0.0], 1.0)],
            sources: vec![],
            mask_value: 1.0,
            mask_source: None,
            opacity: 1.0,
            brush,
            stencil: None,
            projection: None,
            placed: None,
            strokes: vec![stroke],
            paths: vec![],
            ribbon: None,
            particles,
            notes: vec![],
        }
    }

    /// The covered texels of a mask job of `step`: (count, mean row, mean column).
    fn covered(step: ReplayStep) -> (usize, f32, f32) {
        let mesh = wall();
        let job = ReplayJob { target: ReplayTarget::Mask(LayerId(7)), label: "test".into(), steps: vec![step] };
        let out = BrushReplayer.replay(&ReplayScene { mesh: &mesh, material: 0, resolution: RES }, &[job]);
        let Some(Ok(ReplayedPixels::Mask(img))) = out.first() else { panic!("a mask: {out:?}") };
        let (mut n, mut row, mut column) = (0, 0.0, 0.0);
        for y in 0..RES {
            for x in 0..RES {
                if from_u16(img.get(x, y)[1]) > 0.01 {
                    n += 1;
                    row += y as f32;
                    column += x as f32;
                }
            }
        }
        (n, row / n.max(1) as f32, column / n.max(1) as f32)
    }

    fn camera() -> StrokeSpace {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let projection = Mat4::perspective_rh_gl(40f32.to_radians(), 1.0, 0.1, 100.0);
        StrokeSpace::Mesh { view: view.to_cols_array(), projection: projection.to_cols_array() }
    }

    #[test]
    fn a_leak_runs_down_the_wall_from_where_the_stroke_was_in_both_views() {
        let c = RES as f32 / 2.0;
        for space in [StrokeSpace::Uv, camera()] {
            let (dab, dab_row, _) = covered(step(space.clone(), None));
            let (n, row, column) = covered(step(space, Some(leak())));
            assert!(dab > 0 && n > 2 * dab, "{dab} {n}");
            // rows run down the wall: the particles' marks lie below the stroke's own dab
            assert!(row > dab_row + 5.0 && (column - c).abs() < 2.0, "row {row} (dab {dab_row}), column {column}");
        }
    }

    #[test]
    fn particles_that_never_spawn_lay_nothing_and_a_replay_repeats() {
        assert_eq!(covered(step(StrokeSpace::Uv, Some(ReplayParticles { per_point: 0.0, ..leak() }))).0, 0);
        let wander = ReplayParticles { per_point: 8.0, speed: 0.3, wander: 10.0, gravity: 0.0, ..leak() };
        assert_eq!(covered(step(camera(), Some(wander))), covered(step(camera(), Some(wander))));
    }
}
