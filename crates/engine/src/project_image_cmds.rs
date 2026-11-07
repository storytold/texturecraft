//! "Project whole image": bake the Projection tool's stencil from the current camera onto the
//! visible surface in one step, with the brush's culling and angle fade (so grazing surfaces do
//! not stretch the image), the quick mask limit and the tool's material.

use serde_json::{Value, json};
use texturecraft_compose::PaintTarget;
use texturecraft_core::Channel;
use texturecraft_paint::{CommitSource, ProjectCtx, Staging, commit_layer, commit_mask, project_coverage};

use crate::commands::{CommandSpec, always, boolp};
use crate::tool_cmds::viewport_of;
use crate::{EngineError, Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "tool.stencil.project",
        "Project whole image",
        [],
        None,
        "{viewport?: [w,h], backfaceCulling?: bool (default the brush's), angleFade?: [from, to] degrees (default the brush's), occlusion?: bool (default true: hidden surfaces stay clean)} — bakes the stencil (tool.stencil) from the current camera onto the surface into the selected layer or mask, one undo step; baseColorOnly (tool.stencil) limits it to the base color",
        always,
        project
    )]
}

fn project(s: &mut Session, p: &Value) -> Result<Value> {
    if s.quick_mask.editing {
        return Err(EngineError::Other("leave the quick mask to project an image".into()));
    }
    let viewport = viewport_of(s, p);
    let stencil = s.capture_stencil(viewport).ok_or_else(|| EngineError::Other("no stencil image: pick one first (tool.stencil {image})".into()))?;
    let ts = s.ts();
    let (gbuf, mut canvas) = s.fill_canvas(ts)?;
    let b = &s.tool.brush;
    let culling = boolp(p, "backfaceCulling").unwrap_or(b.backface_culling);
    let mut fade = b.angle_fade;
    if let Some([a, z]) = p.get("angleFade").and_then(Value::as_array).and_then(|v| Some([v.first()?.as_f64()?, v.get(1)?.as_f64()?]))
        && a.is_finite()
        && z.is_finite()
    {
        fade = (a.clamp(0.0, 90.0) as f32, z.clamp(0.0, 90.0) as f32);
    }
    let depth = if boolp(p, "occlusion").unwrap_or(true) { Some(s.eye_depth(viewport)) } else { None };
    let mesh = s.mesh.clone();
    let eye = s.camera.eye();
    let ctx = ProjectCtx { mesh: &mesh, gbuf: &gbuf, depth: depth.as_deref(), eye, backface_culling: culling, angle_fade: fade };
    project_coverage(&mut canvas, &ctx, &stencil);
    if canvas.dirty.is_empty() {
        return Err(EngineError::Other("the image does not cover any visible surface from this camera".into()));
    }
    let stencil_fn = |x: u32, y: u32| {
        let pos = gbuf.surface(&mesh, (y * gbuf.width + x) as usize)?.pos;
        stencil.sample_world(pos)
    };
    let size = (canvas.alpha.width(), canvas.alpha.height());
    let opacity = s.tool.brush.opacity;
    let base_color_only = s.tool.stencil.as_ref().is_some_and(|st| st.base_color_only);
    let tiles = canvas.dirty.len();
    s.edit("Project image", |s| {
        let src = CommitSource { alpha: &canvas.alpha, tiles: &canvas.dirty, opacity, erase: false, per_texel: Some(&stencil_fn), carried: None };
        let target = s.paint_target()?;
        let mut values = s.stroke_values(target);
        if base_color_only {
            values.retain(|c, _| *c == Channel::BaseColor);
        }
        if values.is_empty() && matches!(target, PaintTarget::Layer(_)) {
            return Err(EngineError::Other("no channel enabled to paint: enable channels in the tool's material".into()));
        }
        match target {
            PaintTarget::Layer(id) => commit_layer(&mut s.doc.pixels, id, size, &values, src, &mut Staging::default()),
            PaintTarget::Mask(eid) => commit_mask(&mut s.doc.pixels, eid, size, s.tool.mask_value, src, &mut Staging::default()),
        }
        Ok(())
    })?;
    Ok(json!({"tiles": tiles}))
}
