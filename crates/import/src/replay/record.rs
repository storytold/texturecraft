//! Replay jobs as the stroke records a project keeps ([`StrokeRecord`]), and back: the import records every job it
//! builds, so that a saved project can lay its imported strokes again with TextureCraft's brush, without the original
//! file.
//!
//! A record holds a job as the replayer reads it, the brush already mapped onto neutral quantities. What it does not
//! hold: images a brush paints (the record names them as not kept: a job read back from it fails with that reason) and
//! the report notes. A brush painting a graph keeps the graph's reference; the job read back renders it again
//! ([`super::source::render`]).

use std::sync::Arc;

use texturecraft_core::LayerId;
use texturecraft_core::strokes::{
    PathPiece as RecordedPiece, PathStroke, RecordedStroke, RibbonTiling as RecordedTiling, StencilValues, StrokeBrush, StrokeMode, StrokeParticles,
    StrokePoint as RecordedPoint, StrokeRecord, StrokeRibbon, StrokeSource, StrokeSpace as RecordedSpace, StrokeStencil, StrokeStep, StrokeTarget, StrokeTip,
    StrokeTipAlignment,
};

use super::{
    PaintMode, PathPiece, ReplayBrush, ReplayJob, ReplayParticles, ReplayPath, ReplayRibbon, ReplaySource, ReplayStencil, ReplayStep, ReplayStroke,
    ReplayTarget, ReplayTexels, ReplayTip, RibbonTiling, StencilImage, StrokePoint, StrokeSpace, TipAlignment,
};

/// What a record says of a brush image it does not keep.
pub const IMAGE_NOT_KEPT: &str = "the image the brush paints is not kept with the strokes";

/// The record of `job`.
pub fn record(job: &ReplayJob) -> StrokeRecord {
    let target = match job.target {
        ReplayTarget::Content(_) => StrokeTarget::Content,
        ReplayTarget::Mask(_) => StrokeTarget::Mask,
    };
    StrokeRecord { target, label: job.label.clone(), steps: job.steps.iter().map(step).collect() }
}

/// The job `record` stands for, painting into `id`.
pub fn job(id: LayerId, record: &StrokeRecord) -> ReplayJob {
    let target = match record.target {
        StrokeTarget::Content => ReplayTarget::Content(id),
        StrokeTarget::Mask => ReplayTarget::Mask(id),
    };
    ReplayJob { target, label: record.label.clone(), steps: record.steps.iter().map(replay_step).collect() }
}

fn step(s: &ReplayStep) -> StrokeStep {
    StrokeStep {
        mode: match s.mode {
            PaintMode::Paint => StrokeMode::Paint,
            PaintMode::Erase => StrokeMode::Erase,
            PaintMode::Smudge => StrokeMode::Smudge,
        },
        values: s.values.clone(),
        sources: s.sources.iter().map(|(c, src, o)| (*c, source(src), *o)).collect(),
        mask_value: s.mask_value,
        mask_source: s.mask_source.as_ref().map(source),
        opacity: s.opacity,
        brush: brush(&s.brush),
        stencil: s.stencil.as_ref().map(stencil),
        projection: s.projection.as_ref().map(stencil),
        placed: s.placed,
        strokes: s.strokes.iter().map(stroke).collect(),
        paths: s.paths.iter().map(path).collect(),
        ribbon: s.ribbon.map(|r| StrokeRibbon {
            width: r.width,
            copy: r.copy,
            tiling: match r.tiling {
                RibbonTiling::Repeat => RecordedTiling::Repeat,
                RibbonTiling::Whole => RecordedTiling::Whole,
                RibbonTiling::Caps { guides } => RecordedTiling::Caps { guides },
            },
            depth: r.depth,
        }),
        particles: s.particles.map(|p| StrokeParticles {
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
            file_units: p.file_units,
        }),
    }
}

fn replay_step(s: &StrokeStep) -> ReplayStep {
    ReplayStep {
        mode: match s.mode {
            StrokeMode::Paint => PaintMode::Paint,
            StrokeMode::Erase => PaintMode::Erase,
            StrokeMode::Smudge => PaintMode::Smudge,
        },
        values: s.values.clone(),
        sources: s.sources.iter().map(|(c, src, o)| (*c, replay_source(src), *o)).collect(),
        mask_value: s.mask_value,
        mask_source: s.mask_source.as_ref().map(replay_source),
        opacity: s.opacity,
        brush: replay_brush(&s.brush),
        stencil: s.stencil.as_ref().map(replay_stencil),
        projection: s.projection.as_ref().map(replay_stencil),
        placed: s.placed,
        strokes: s.strokes.iter().map(replay_stroke).collect(),
        paths: s.paths.iter().map(replay_path).collect(),
        ribbon: s.ribbon.map(|r| ReplayRibbon {
            width: r.width,
            copy: r.copy,
            tiling: match r.tiling {
                RecordedTiling::Repeat => RibbonTiling::Repeat,
                RecordedTiling::Whole => RibbonTiling::Whole,
                RecordedTiling::Caps { guides } => RibbonTiling::Caps { guides },
            },
            depth: r.depth,
        }),
        particles: s.particles.map(|p| ReplayParticles {
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
            file_units: p.file_units,
        }),
        notes: Vec::new(),
    }
}

fn source(s: &ReplaySource) -> StrokeSource {
    match s {
        ReplaySource::Graph(g) => StrokeSource::Graph(g.clone()),
        ReplaySource::Texels(t) => match &**t {
            ReplayTexels::Image(_) => StrokeSource::NotKept(IMAGE_NOT_KEPT.to_string()),
            ReplayTexels::Graph { .. } => StrokeSource::NotKept("the graph the brush paints is kept only as its render".to_string()),
        },
        ReplaySource::Failed(why) => StrokeSource::NotKept(why.clone()),
    }
}

fn replay_source(s: &StrokeSource) -> ReplaySource {
    match s {
        StrokeSource::Graph(g) => ReplaySource::Graph(g.clone()),
        StrokeSource::NotKept(why) => ReplaySource::Failed(why.clone()),
    }
}

fn brush(b: &ReplayBrush) -> StrokeBrush {
    StrokeBrush {
        radius: b.radius,
        spacing: b.spacing,
        flow: b.flow,
        size_pressure: b.size_pressure,
        flow_pressure: b.flow_pressure,
        size_jitter: b.size_jitter,
        flow_jitter: b.flow_jitter,
        position_jitter: b.position_jitter,
        position_both: b.position_both,
        position_normal: b.position_normal,
        rotation: b.rotation,
        rotation_jitter: b.rotation_jitter,
        follow: b.follow,
        alignment: match b.alignment {
            TipAlignment::Tangent => StrokeTipAlignment::Tangent,
            TipAlignment::Camera => StrokeTipAlignment::Camera,
            TipAlignment::Uv => StrokeTipAlignment::Uv,
        },
        culling: b.culling,
        tip: tip(&b.tip),
        cycle: b.cycle.iter().map(tip).collect(),
        cycle_drawn_start: b.cycle_drawn_start,
        up: b.up,
    }
}

fn replay_brush(b: &StrokeBrush) -> ReplayBrush {
    ReplayBrush {
        radius: b.radius,
        spacing: b.spacing,
        flow: b.flow,
        size_pressure: b.size_pressure,
        flow_pressure: b.flow_pressure,
        size_jitter: b.size_jitter,
        flow_jitter: b.flow_jitter,
        position_jitter: b.position_jitter,
        position_both: b.position_both,
        position_normal: b.position_normal,
        rotation: b.rotation,
        rotation_jitter: b.rotation_jitter,
        follow: b.follow,
        alignment: match b.alignment {
            StrokeTipAlignment::Tangent => TipAlignment::Tangent,
            StrokeTipAlignment::Camera => TipAlignment::Camera,
            StrokeTipAlignment::Uv => TipAlignment::Uv,
        },
        culling: b.culling,
        tip: replay_tip(&b.tip),
        cycle: b.cycle.iter().map(replay_tip).collect(),
        cycle_drawn_start: b.cycle_drawn_start,
        up: b.up,
    }
}

fn tip(t: &ReplayTip) -> StrokeTip {
    match t {
        ReplayTip::Round { hardness } => StrokeTip::Round { hardness: *hardness },
        ReplayTip::Image { width, height, values } => StrokeTip::Image { width: *width, height: *height, values: values.to_vec() },
    }
}

fn replay_tip(t: &StrokeTip) -> ReplayTip {
    match t {
        StrokeTip::Round { hardness } => ReplayTip::Round { hardness: *hardness },
        StrokeTip::Image { width, height, values } => ReplayTip::Image { width: *width, height: *height, values: Arc::from(values.as_slice()) },
    }
}

fn stencil(s: &ReplayStencil) -> StrokeStencil {
    let image = match &s.image {
        StencilImage::Uniform(v) => StencilValues::Uniform(*v),
        StencilImage::Image { width, height, values } => StencilValues::Image { width: *width, height: *height, values: values.to_vec() },
    };
    StrokeStencil { image, matrix: s.matrix }
}

fn replay_stencil(s: &StrokeStencil) -> ReplayStencil {
    let image = match &s.image {
        StencilValues::Uniform(v) => StencilImage::Uniform(*v),
        StencilValues::Image { width, height, values } => StencilImage::Image { width: *width, height: *height, values: Arc::from(values.as_slice()) },
    };
    ReplayStencil { image, matrix: s.matrix }
}

fn stroke(s: &ReplayStroke) -> RecordedStroke {
    RecordedStroke {
        space: match &s.space {
            StrokeSpace::Mesh { view, projection } => RecordedSpace::Mesh { view: *view, projection: *projection },
            StrokeSpace::Uv => RecordedSpace::Uv,
        },
        points: s.points.iter().map(|p| RecordedPoint { position: p.position, pressure: p.pressure }).collect(),
        seed: s.seed,
        lazy: s.lazy,
        copies: s.copies,
        copy_matrix: s.copy_matrix,
    }
}

fn replay_stroke(s: &RecordedStroke) -> ReplayStroke {
    ReplayStroke {
        space: match &s.space {
            RecordedSpace::Mesh { view, projection } => StrokeSpace::Mesh { view: *view, projection: *projection },
            RecordedSpace::Uv => StrokeSpace::Uv,
        },
        points: s.points.iter().map(|p| StrokePoint { position: p.position, pressure: p.pressure }).collect(),
        seed: s.seed,
        lazy: s.lazy,
        copies: s.copies,
        copy_matrix: s.copy_matrix,
    }
}

fn path(p: &ReplayPath) -> PathStroke {
    PathStroke {
        pieces: p.pieces.iter().map(|q| RecordedPiece { points: q.points, normals: q.normals, size: q.size }).collect(),
        anchor: p.anchor,
        seed: p.seed,
        copies: p.copies,
        copy_matrix: p.copy_matrix,
    }
}

fn replay_path(p: &PathStroke) -> ReplayPath {
    ReplayPath {
        pieces: p.pieces.iter().map(|q| PathPiece { points: q.points, normals: q.normals, size: q.size }).collect(),
        anchor: p.anchor,
        seed: p.seed,
        copies: p.copies,
        copy_matrix: p.copy_matrix,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use texturecraft_compose::ImageRes;
    use texturecraft_core::{Channel, GraphRef, LayerId};

    use super::super::*;
    use super::{IMAGE_NOT_KEPT, job, record};

    fn brush() -> ReplayBrush {
        let image = ReplayTip::Image { width: 2, height: 1, values: Arc::from(vec![0.5, 1.0]) };
        ReplayBrush {
            radius: 0.04,
            spacing: 0.2,
            flow: 0.9,
            size_pressure: Some(0.05),
            flow_pressure: Some(0.1),
            size_jitter: 0.2,
            flow_jitter: 0.3,
            position_jitter: 0.4,
            position_both: false,
            position_normal: true,
            rotation: 0.5,
            rotation_jitter: 0.6,
            follow: true,
            alignment: TipAlignment::Uv,
            culling: None,
            tip: image.clone(),
            cycle: vec![ReplayTip::Round { hardness: 0.3 }, image],
            cycle_drawn_start: true,
            up: Some([0.0, 0.0, 1.0]),
        }
    }

    fn step(mode: PaintMode) -> ReplayStep {
        let stroke = ReplayStroke {
            space: StrokeSpace::Mesh { view: [0.5; 16], projection: [0.25; 16] },
            points: vec![StrokePoint { position: [0.0, 0.1], pressure: 0.7 }],
            seed: 11,
            lazy: 0.1,
            copies: 2,
            copy_matrix: [3.0; 16],
        };
        let path = ReplayPath {
            pieces: vec![PathPiece { points: [[0.1; 3]; 4], normals: [[0.0, 1.0, 0.0]; 2], size: [1.0, 0.5] }],
            anchor: Some(([1.0; 3], [0.0, 0.0, 1.0], 0.5)),
            seed: 12,
            copies: 1,
            copy_matrix: [4.0; 16],
        };
        ReplayStep {
            mode,
            values: vec![(Channel::BaseColor, [0.2, 0.4, 0.6], 0.8)],
            sources: vec![(Channel::Roughness, ReplaySource::Graph(Box::new(GraphRef::new("a".repeat(16), "dirt"))), 0.5)],
            mask_value: 0.3,
            mask_source: Some(ReplaySource::Graph(Box::new(GraphRef::new("b".repeat(16), "edges")))),
            opacity: 0.6,
            brush: brush(),
            stencil: Some(ReplayStencil { image: StencilImage::Image { width: 1, height: 1, values: Arc::from(vec![0.75]) }, matrix: [0.1; 9] }),
            projection: Some(ReplayStencil { image: StencilImage::Uniform(0.4), matrix: [0.2; 9] }),
            placed: Some([0.3; 9]),
            strokes: vec![stroke.clone(), ReplayStroke { space: StrokeSpace::Uv, ..stroke }],
            paths: vec![path],
            ribbon: Some(ReplayRibbon { width: 0.2, copy: 0.5, tiling: RibbonTiling::Repeat, depth: 0.01 }),
            particles: Some(ReplayParticles {
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
            }),
            notes: vec![],
        }
    }

    #[test]
    fn a_job_read_back_from_its_record_is_the_job() {
        for (target, id) in [(ReplayTarget::Content(LayerId(3)), LayerId(3)), (ReplayTarget::Mask(LayerId(4)), LayerId(4))] {
            let mut smudge = step(PaintMode::Smudge);
            smudge.ribbon = Some(ReplayRibbon { tiling: RibbonTiling::Caps { guides: [0.1, 0.9] }, ..smudge.ribbon.unwrap() });
            let mut erase = step(PaintMode::Erase);
            erase.ribbon = Some(ReplayRibbon { tiling: RibbonTiling::Whole, ..erase.ribbon.unwrap() });
            erase.brush.alignment = TipAlignment::Camera;
            let original = ReplayJob { target, label: "Layer / effect".into(), steps: vec![step(PaintMode::Paint), smudge, erase] };
            assert_eq!(job(id, &record(&original)), original);
        }
    }

    #[test]
    fn notes_and_brush_images_are_not_kept() {
        let mut s = step(PaintMode::Paint);
        s.notes.push("a note for the report".into());
        let image = ImageRes { width: 1, height: 1, rgba: vec![[1.0; 4]], srgb: false };
        s.sources.push((Channel::Metallic, ReplaySource::Texels(Arc::new(ReplayTexels::Image(Arc::new(image)))), 1.0));
        s.sources.push((Channel::Height, ReplaySource::Failed("did not render".into()), 1.0));
        let original = ReplayJob { target: ReplayTarget::Content(LayerId(1)), label: String::new(), steps: vec![s] };
        let back = job(LayerId(1), &record(&original));
        let step = &back.steps[0];
        assert!(step.notes.is_empty());
        assert_eq!(step.sources[1].1, ReplaySource::Failed(IMAGE_NOT_KEPT.into()));
        assert_eq!(step.sources[2].1, ReplaySource::Failed("did not render".into()));
        assert_eq!(step.sources[0], original.steps[0].sources[0]);
    }
}
