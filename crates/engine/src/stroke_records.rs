//! Laying a project's stroke records again ([`Session::replay_stroke_record`]): the strokes an import kept behind its
//! pixels ([`texturecraft_core::strokes`]) painted once more with TextureCraft's brush, as the import painted them.

use texturecraft_core::{LayerId, StoredStrokes};
use texturecraft_import::replay::{ReplayScene, ReplayedPixels, StrokeReplayer, record, source};

use crate::import_replay::BrushReplayer;
use crate::session::Session;
use crate::{EngineError, Result};

impl Session {
    /// Paints the stroke record keyed by `id` (a paint layer, a content or mask paint effect) again on the mesh of the
    /// texture set holding it, at the set's resolution: the pixels it lays, in the stores' layouts. The document is not
    /// changed; graphs the brush paints render with the project's graphs, baked maps and images.
    pub fn replay_stroke_record(&self, id: LayerId) -> Result<ReplayedPixels> {
        let stored = self.doc.strokes.get(&id).ok_or_else(|| EngineError::Other(format!("no stroke record for {}", id.0)))?;
        let record = match stored {
            StoredStrokes::Record(r) => r,
            StoredStrokes::Unreadable(u) => return Err(EngineError::Other(format!("the stroke record of {} could not be read: {}", id.0, u.reason))),
        };
        let project = &self.doc.project;
        let owner = if project.layers.contains_key(&id) { Some(id) } else { project.effect(id).map(|(owner, _)| owner) };
        let ts = owner.and_then(|o| project.locate(o)).map(|(ts, _, _)| ts).ok_or_else(|| EngineError::Other(format!("{} is in no texture set", id.0)))?;
        let set = project.texture_sets.get(ts).ok_or_else(|| EngineError::Other(format!("no texture set {ts}")))?;
        let mut jobs = [record::job(id, record)];
        let empty = Default::default();
        let env = texturecraft_compose::graphs::RenderEnv {
            baked: self.doc.baked.get(&ts).unwrap_or(&empty),
            images: &self.images,
            side: texturecraft_compose::graphs::RenderEnv::side_for((set.resolution, set.resolution)),
        };
        source::render(&mut jobs, &self.doc.graphs, &env);
        let scene = ReplayScene { mesh: &self.mesh, material: set.material, resolution: set.resolution };
        BrushReplayer.replay(&scene, &jobs).into_iter().next().unwrap_or_else(|| Err("the replayer gave no result".into())).map_err(EngineError::Other)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use texturecraft_core::{Channel, Effect, EffectKind, Fill, Layer, LayerKind, Mask, MaskBackground, Project, TextureSet};
    use texturecraft_geom::glam::{Mat4, Vec3};
    use texturecraft_geom::{Mesh, primitives};
    use texturecraft_import::Imported;
    use texturecraft_import::replay::{
        PaintMode, PathPiece, ReplayBrush, ReplayJob, ReplayParticles, ReplayPath, ReplayStep, ReplayStroke, ReplayTarget, ReplayTip, StrokePoint, StrokeSpace,
        TipAlignment,
    };

    use super::*;

    const RES: u32 = 128;

    /// A 2 × 2 wall at z = 0 facing +z.
    fn wall() -> Mesh {
        let mut m = primitives::plane(2.0, 16);
        for p in &mut m.positions {
            *p = Vec3::new(p.x, -p.z, 0.0);
        }
        for n in &mut m.normals {
            *n = Vec3::Z;
        }
        m
    }

    fn step(mode: PaintMode, strokes: Vec<ReplayStroke>, paths: Vec<ReplayPath>) -> ReplayStep {
        let brush = ReplayBrush {
            radius: 0.1,
            spacing: 0.1,
            flow: 1.0,
            size_pressure: Some(0.2),
            flow_pressure: None,
            size_jitter: 0.3,
            flow_jitter: 0.2,
            position_jitter: 0.1,
            position_both: true,
            position_normal: true,
            rotation: 0.0,
            rotation_jitter: 0.5,
            follow: false,
            alignment: TipAlignment::Tangent,
            culling: Some(0.5),
            tip: ReplayTip::Round { hardness: 0.8 },
            cycle: vec![],
            cycle_drawn_start: false,
            up: None,
        };
        ReplayStep {
            mode,
            values: vec![(Channel::BaseColor, [1.0, 0.25, 0.0], 1.0), (Channel::Roughness, [0.2; 3], 0.5)],
            sources: vec![],
            mask_value: 0.75,
            mask_source: None,
            opacity: 0.9,
            brush,
            stencil: None,
            projection: None,
            placed: None,
            strokes,
            paths,
            ribbon: None,
            particles: None,
            notes: vec![],
        }
    }

    fn particles() -> ReplayParticles {
        ReplayParticles {
            per_point: 2.0,
            per_length: 0.5,
            budget: 64.0,
            speed: 0.3,
            speed_rand: 0.2,
            life: 0.5,
            life_rand: 0.1,
            gravity: 1.0,
            friction: 0.5,
            wander: 0.1,
            curl: 0.2,
            fan: 1.0,
            heading: 0.3,
            away: true,
            kink_time: 0.1,
            kink_angle: 0.4,
            kink_branches: 2.0,
            kink_speed: 1.0,
            stop_speed: 0.01,
            view_bias: 0.2,
            spread: 1.0,
            scatter: 0.1,
            fade_size: 0.2,
            fade_size_rand: 0.1,
            fade_flow: 0.3,
            fade_flow_rand: 0.1,
            size: 0.8,
            flow: 0.9,
            step: 0.01,
            file_units: false,
        }
    }

    fn strokes() -> Vec<ReplayStroke> {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let projection = Mat4::perspective_rh_gl(40f32.to_radians(), 1.0, 0.1, 100.0);
        let mesh = StrokeSpace::Mesh { view: view.to_cols_array(), projection: projection.to_cols_array() };
        let points = |ps: &[[f32; 2]]| ps.iter().enumerate().map(|(i, p)| StrokePoint { position: *p, pressure: 0.5 + 0.1 * i as f32 }).collect();
        vec![
            ReplayStroke { space: mesh, points: points(&[[-0.3, 0.0], [0.0, 0.1], [0.3, 0.0]]), seed: 5, lazy: 0.0, copies: 0, copy_matrix: [0.0; 16] },
            ReplayStroke { space: StrokeSpace::Uv, points: points(&[[0.2, 0.8], [0.6, 0.7]]), seed: 6, lazy: 0.0, copies: 0, copy_matrix: [0.0; 16] },
        ]
    }

    fn path() -> ReplayPath {
        let piece =
            PathPiece { points: [[-0.8, -0.5, 0.0], [-0.4, -0.6, 0.0], [0.4, -0.4, 0.0], [0.8, -0.5, 0.0]], normals: [[0.0, 0.0, 1.0]; 2], size: [1.0, 0.5] };
        ReplayPath { pieces: vec![piece], anchor: None, seed: 9, copies: 0, copy_matrix: [0.0; 16] }
    }

    /// The import's side, synthetic: a project with a paint layer and a mask paint effect, their strokes replayed into
    /// the pixels and recorded.
    fn imported() -> (Imported, [LayerId; 2]) {
        let mesh = wall();
        let mut project = Project::new("strokes", "mesh.glb");
        project.texture_sets.push(TextureSet::new("wall", 0, RES));
        let layer = Layer::new(project.alloc_id(), "Paint", LayerKind::Paint, &Channel::DEFAULT_SET);
        let paint = project.insert_layer(0, layer, None, false).unwrap();
        let mut masked = Layer::new(project.alloc_id(), "Masked", LayerKind::Fill(Fill::default()), &Channel::DEFAULT_SET);
        let effect = project.alloc_id();
        masked.mask = Some(Mask { effects: vec![Effect::new(effect, EffectKind::Paint)], ..Mask::new(MaskBackground::Black) });
        project.insert_layer(0, masked, None, false).unwrap();
        let jobs = [
            ReplayJob {
                target: ReplayTarget::Content(paint),
                label: "Paint".into(),
                steps: vec![step(PaintMode::Paint, strokes(), vec![path()]), step(PaintMode::Erase, strokes()[1..].to_vec(), vec![])],
            },
            ReplayJob {
                target: ReplayTarget::Mask(effect),
                label: "Masked / mask".into(),
                steps: vec![
                    step(PaintMode::Paint, strokes(), vec![]),
                    ReplayStep { particles: Some(particles()), ..step(PaintMode::Paint, strokes(), vec![]) },
                ],
            },
        ];
        let scene = ReplayScene { mesh: &mesh, material: 0, resolution: RES };
        let mut pixels = texturecraft_compose::PixelStore::default();
        for (job, out) in jobs.iter().zip(BrushReplayer.replay(&scene, &jobs)) {
            match out.unwrap() {
                ReplayedPixels::Content(images) => pixels.layers.extend(images.into_iter().map(|(c, img)| ((job.target.id(), c), img))),
                ReplayedPixels::Mask(img) => {
                    pixels.masks.insert(job.target.id(), img);
                }
            }
        }
        let strokes = jobs.iter().map(|j| (j.target.id(), record::record(j).into())).collect();
        let imported =
            Imported { project, pixels, baked: BTreeMap::new(), mesh, resources: BTreeMap::new(), graphs: Default::default(), sets: Vec::new(), strokes };
        (imported, [paint, effect])
    }

    #[test]
    fn strokes_replayed_from_a_saved_project_paint_the_imported_pixels() {
        let (imported, [paint, effect]) = imported();
        let pixels = imported.pixels.clone();
        assert!(pixels.masks.get(&effect).is_some_and(|m| m.allocated_count() > 0), "the mask job painted");
        assert!(pixels.layers.get(&(paint, Channel::BaseColor)).is_some_and(|m| m.allocated_count() > 0), "the content job painted");
        let session = Session::from_imported(imported).unwrap();
        let bytes = session.to_project_bytes().unwrap();
        let opened = Session::from_project_bytes(&bytes).unwrap();
        assert_eq!(opened.doc.strokes, session.doc.strokes);
        // laid again from the opened project: the very pixels the import laid
        let Ok(ReplayedPixels::Content(images)) = opened.replay_stroke_record(paint) else { panic!("content pixels") };
        assert_eq!(images.len(), 2);
        for (c, img) in images {
            assert_eq!(Some(&img), pixels.layers.get(&(paint, c)), "{c:?}");
            assert_eq!(Some(&img), opened.doc.pixels.layers.get(&(paint, c)), "{c:?} as saved");
        }
        let Ok(ReplayedPixels::Mask(img)) = opened.replay_stroke_record(effect) else { panic!("mask pixels") };
        assert_eq!(Some(&img), pixels.masks.get(&effect));
        // project.inspect shows the records
        let inspect = crate::execute(&mut Session::from_project_bytes(&bytes).unwrap(), "project.inspect", &serde_json::json!({})).unwrap();
        assert!(inspect.to_string().contains(r#""strokes":{"steps":2,"strokes":4}"#), "{inspect}");
    }

    #[test]
    fn replaying_a_missing_or_unreadable_record_is_an_error() {
        let (imported, [paint, _]) = imported();
        let mut session = Session::from_imported(imported).unwrap();
        assert!(session.replay_stroke_record(LayerId(9999)).is_err());
        let bytes = std::sync::Arc::new(vec![1, 2, 3]);
        session.doc.strokes.insert(paint, StoredStrokes::Unreadable(texturecraft_core::strokes::UnreadableStrokes { bytes, reason: "damaged".into() }));
        let Err(e) = session.replay_stroke_record(paint) else { panic!("an error") };
        assert!(e.to_string().contains("damaged"), "{e}");
    }
}
