//! Interactive painting on the session: strokes (begin / continue / end / cancel), polygon fill
//! and the material picker. The UI, CLI and agents all go through these.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use texturecraft_compose::PaintTarget;
use texturecraft_core::{Channel, Effect, EffectKind, LayerId, LayerKind, Mask, MaskBackground, Source};
use texturecraft_geom::glam::Vec2;
use texturecraft_geom::{Bvh, Mesh, OrbitCamera};
use texturecraft_paint::{
    CommitSource, EyeDepth, PickCtx, Pickup, PickupMode, RasterCtx, Shape, Staging, Stroke, StrokeCanvas, SurfaceHit, commit_layer, commit_mask, pick,
    pick_filtered, rasterize,
};
use texturecraft_texel::{TILE, TILE_AREA};

use crate::brush_device::{BrushTiles, DeviceStroke, DeviceStrokePreview, ProjectedTiles};
use crate::session::{
    ActiveStroke, CloneFrom, DisplayStrokeEnd, QUICK_MASK_STROKE_LABEL, ReleasedDab, Session, StackTarget, Tool, history_label, next_mesh_id, stroke_template,
};
use crate::stroke_replay::{PaintPlacement, RecordedPaint, StrokeReplays, WorldPaint, record_sample, record_uv_dabs};
use crate::uv_stroke_state::{UV_STAMP_BATCH, UvStrokeState};
use crate::uv_symmetry::UvMirror;
use crate::{EngineError, Result};

/// The brush size (as in `BrushSettings::size`) at which a brush material's pattern has its natural size; a smaller brush makes the pattern smaller in proportion
pub const BRUSH_PATTERN_SIZE: f32 = 0.1;

/// Brush size floor for the pattern scale
const MIN_PATTERN_BRUSH_SIZE: f32 = 0.0005;

/// Limit CPU-side storage for projected color to one readback batch.
const PROJECTION_READBACK_BATCH: usize = 128;

/// The view inputs used to classify occlusion when a world-space brush stroke began.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct StrokeView {
    pub(crate) camera: OrbitCamera,
    pub(crate) size: (u32, u32),
    hidden_materials: BTreeSet<usize>,
}

pub(crate) struct StrokeVisibility {
    mesh_id: u64,
    view: StrokeView,
    bvh: Arc<Bvh>,
    depth: Arc<EyeDepth>,
}

/// Material lookup in mesh order, with the same first range taking priority as `Mesh::material_of`.
fn visible_triangle_mask(mesh: &Mesh, hidden_materials: &BTreeSet<usize>) -> Vec<bool> {
    let mut shown = vec![!hidden_materials.contains(&0); mesh.triangle_count()];
    for submesh in mesh.submeshes.iter().rev() {
        let start = submesh.start.div_ceil(3) as usize;
        let end = (submesh.start.saturating_add(submesh.count).div_ceil(3) as usize).min(shown.len());
        if let Some(range) = shown.get_mut(start..end) {
            range.fill(!hidden_materials.contains(&submesh.material));
        }
    }
    shown
}

impl StrokeView {
    fn capture(camera: OrbitCamera, viewport: Vec2, sets: &[texturecraft_core::TextureSet]) -> Self {
        let size = ((viewport.x / 2.0).max(64.0) as u32, (viewport.y / 2.0).max(64.0) as u32);
        let mut hidden_materials = BTreeSet::new();
        // The viewport uses the last texture set mapped to each mesh material.
        for set in sets {
            if set.visible {
                hidden_materials.remove(&set.material);
            } else {
                hidden_materials.insert(set.material);
            }
        }
        Self { camera, size, hidden_materials }
    }

    /// Identical triangle selection for cursor rays, exact occlusion and camera depth.
    pub(crate) fn visibility(&self, mesh: &Mesh, full_bvh: &Arc<Bvh>, previous: Option<(&Self, &Arc<Bvh>)>) -> (Arc<Bvh>, EyeDepth) {
        let (width, height) = self.size;
        if self.hidden_materials.is_empty() {
            (full_bvh.clone(), EyeDepth::render(mesh, &self.camera, width, height))
        } else {
            let shown = visible_triangle_mask(mesh, &self.hidden_materials);
            let include = |triangle| shown.get(triangle).copied().unwrap_or(false);
            let existing = previous.filter(|(view, _)| self.hidden_materials == view.hidden_materials);
            let bvh = existing.map_or_else(|| Arc::new(Bvh::new_filtered(mesh, include)), |(_, bvh)| bvh.clone());
            (bvh, EyeDepth::render_filtered(mesh, &self.camera, width, height, include))
        }
    }

    pub(crate) fn hidden_count(&self) -> usize {
        self.hidden_materials.len()
    }
}

impl Session {
    fn stroke_visibility(&mut self, viewport: Vec2) -> (Arc<Bvh>, Arc<EyeDepth>) {
        let view = StrokeView::capture(self.camera, viewport, &self.doc.project.texture_sets);
        let cached = self.stroke_visibility.as_ref().filter(|scene| scene.mesh_id == self.mesh_id);
        if let Some(scene) = cached.filter(|scene| scene.view == view) {
            return (scene.bvh.clone(), scene.depth.clone());
        }
        let (bvh, depth) = view.visibility(&self.mesh, &self.bvh, cached.map(|scene| (&scene.view, &scene.bvh)));
        let depth = Arc::new(depth);
        self.stroke_visibility = Some(StrokeVisibility { mesh_id: self.mesh_id, view, bvh: bvh.clone(), depth: depth.clone() });
        (bvh, depth)
    }

    pub fn pick(&self, screen: Vec2, viewport: Vec2) -> Option<SurfaceHit> {
        let ctx = PickCtx { mesh: &self.mesh, bvh: &self.bvh, camera: &self.camera, viewport };
        let sets = &self.doc.project.texture_sets;
        if sets.iter().all(|set| set.visible) {
            pick(&ctx, screen)
        } else {
            pick_filtered(&ctx, screen, &|triangle| {
                let material = self.mesh.material_of(triangle as usize).unwrap_or(0);
                sets.iter().rfind(|set| set.material == material).is_none_or(|set| set.visible)
            })
        }
    }

    /// The texture set that paints mesh material `material` (a [`SurfaceHit::material`])
    pub fn texture_set_of_material(&self, material: usize) -> Option<usize> {
        self.doc.project.texture_sets.iter().rposition(|t| t.material == material)
    }

    /// The texture set of the surface under viewport pixel `screen`
    pub fn texture_set_at(&self, screen: Vec2, viewport: Vec2) -> Option<usize> {
        self.pick(screen, viewport).and_then(|hit| self.texture_set_of_material(hit.material))
    }

    /// Resolve what a stroke on the selected layer paints into, creating the mask paint effect
    /// when painting a mask. Must run inside the stroke's history snapshot.
    fn paint_target(&mut self) -> Result<PaintTarget> {
        let id = self.selected_layer().ok_or_else(|| EngineError::Other("select a layer to paint on".into()))?;

        // Painting an instance paints its source unless its own mask overrides the mask
        let p = &self.doc.project;
        let id = if self.target == StackTarget::Mask { p.mask_owner(id) } else { p.content_layer(id).map_or(id, |l| l.id) };
        let owner = match self.target {
            StackTarget::Effect(e) => p.effect(e).map_or(id, |(owner, _)| owner),
            _ => id,
        };
        crate::lock_cmds::refuse_locked(p, owner)?;
        match self.target {
            StackTarget::Content => {
                let l = self.doc.project.layer(id)?;
                if matches!(l.kind, LayerKind::Paint) {
                    Ok(PaintTarget::Layer(id))
                } else {
                    Err(EngineError::Other(format!("\"{}\" is a {} layer: paint on a paint layer or on its mask", l.name, l.kind.name())))
                }
            }
            StackTarget::Effect(e) => match self.doc.project.effect(e) {
                // Content paint effects hold channel pixels like a paint layer
                Some((owner, eff)) if matches!(eff.kind, EffectKind::Paint) => {
                    let content = self.doc.project.layer(owner)?.content_effects.iter().any(|x| x.id == e);
                    Ok(if content { PaintTarget::Layer(e) } else { PaintTarget::Mask(e) })
                }

                // Path sublayers are painted only from their stored paths
                Some((owner, eff)) if matches!(eff.kind, EffectKind::Path(_)) && self.painting_paths => {
                    let content = self.doc.project.layer(owner)?.content_effects.iter().any(|x| x.id == e);
                    Ok(if content { PaintTarget::Layer(e) } else { PaintTarget::Mask(e) })
                }
                Some((_, eff)) if matches!(eff.kind, EffectKind::Path(_)) => {
                    Err(EngineError::Other("a path sublayer holds paths only: edit them with the Path tool, or paint on a paint sublayer".into()))
                }

                // Paint below a selected non-paint effect so new paint passes through it
                Some((owner, _)) => self.paint_target_below(owner, e),
                None => Err(EngineError::Other("the selected effect is not a paint effect".into())),
            },
            StackTarget::Mask => {
                let eid = self.doc.project.alloc_id();
                let layer = self.doc.project.layer_mut(id)?;
                let mask = layer.mask.get_or_insert_with(|| Mask::new(MaskBackground::Black));
                if let Some(top) = mask.effects.last().filter(|e| matches!(e.kind, EffectKind::Paint)) {
                    return Ok(PaintTarget::Mask(top.id));
                }

                mask.effects.push(Effect::new(eid, EffectKind::Paint));
                Ok(PaintTarget::Mask(eid))
            }
        }
    }

    /// Whether the stroke the selection starts paints a mask (the layer's mask, or a paint or
    /// filter sublayer inside it).
    fn paints_mask(&self) -> bool {
        match self.target {
            StackTarget::Content => false,
            StackTarget::Mask => true,
            StackTarget::Effect(effect) => {
                let project = &self.doc.project;
                project.effect(effect).and_then(|(owner, _)| project.layers.get(&owner)).is_some_and(|l| !l.content_effects.iter().any(|e| e.id == effect))
            }
        }
    }

    /// What a stroke paints while non-paint effect `effect` of layer `owner` is selected: the
    /// nearest paint effect below it in the same stack, else the paint layer's own pixels (for
    /// content) or a new paint effect inserted just below it (for a mask).
    fn paint_target_below(&mut self, owner: LayerId, effect: LayerId) -> Result<PaintTarget> {
        let eid = self.doc.project.alloc_id();
        let layer = self.doc.project.layer_mut(owner)?;
        let below = |effects: &[Effect], at: usize| effects.get(..at).and_then(|b| b.iter().rev().find(|x| matches!(x.kind, EffectKind::Paint))).map(|x| x.id);
        if let Some(at) = layer.content_effects.iter().position(|x| x.id == effect) {
            if let Some(id) = below(&layer.content_effects, at) {
                return Ok(PaintTarget::Layer(id));
            }
            return if matches!(layer.kind, LayerKind::Paint) {
                Ok(PaintTarget::Layer(owner))
            } else {
                Err(EngineError::Other(format!(
                    "\"{}\" is a {} layer: paint on a paint layer or on its mask (a filter changes what lies below it)",
                    layer.name,
                    layer.kind.name()
                )))
            };
        }
        let mask = layer.mask.as_mut().ok_or_else(|| EngineError::Other("the selected effect is not a paint effect".into()))?;
        let at = mask.effects.iter().position(|x| x.id == effect).ok_or_else(|| EngineError::Other("the selected effect is not a paint effect".into()))?;
        if let Some(id) = below(&mask.effects, at) {
            return Ok(PaintTarget::Mask(id));
        }
        mask.effects.insert(at, Effect::new(eid, EffectKind::Paint));
        Ok(PaintTarget::Mask(eid))
    }

    /// The pattern of the brush's material (`tool.material`) as a pickup over the texture, for the
    /// channels in `values`: the material's fill, evaluated with the compose fill code tri-planar
    /// in world space, so it stays fixed to the surface and continuous across stamps and strokes.
    /// Its size follows the brush size ([`BRUSH_PATTERN_SIZE`] is the natural size). Channels the
    /// material lacks carry the tool's value. None without a brush material.
    fn brush_pattern(&self, values: &BTreeMap<Channel, [f32; 3]>) -> Option<Pickup> {
        let id = self.tool.material.clone()?;
        let mut sources = crate::materials::brush_sources(&id)?;
        sources.retain(|(channel, _)| values.contains_key(channel));
        for (channel, value) in values {
            if !sources.iter().any(|(set, _)| set == channel) {
                sources.push((*channel, Source::Uniform(*value)));
            }
        }
        let ts = self.ts();
        let set = self.doc.project.texture_sets.get(ts)?;
        let mut project = crate::material_lib::one_set_document(set, "Brush material").project;
        let layer_id = project.alloc_id();
        let scale = BRUSH_PATTERN_SIZE / self.tool.brush.size.max(MIN_PATTERN_BRUSH_SIZE);
        project.insert_layer(0, crate::materials::brush_fill(layer_id, sources, scale), None, false).ok()?;
        let snapshot = texturecraft_paint::pickup::PickupSnapshot {
            project,
            ts: 0,
            target: PaintTarget::Layer(layer_id),
            mesh: self.mesh.clone(),
            bounds: self.mesh.bounds(),
            pixels: Default::default(),
            baked: self.doc.baked.get(&ts).cloned().unwrap_or_default(),
            images: Default::default(),
            graphs: Default::default(),
        };
        Some(Pickup::from_snapshot(PickupMode::Clone { offset: [0; 2] }, snapshot, self.caches.get(ts)?.gbuf.clone(), values.keys().copied()))
    }

    fn stroke_values(&self, target: PaintTarget) -> BTreeMap<Channel, [f32; 3]> {
        let ts = self.ts();
        let Ok(set) = self.doc.project.texture_set(ts) else { return BTreeMap::new() };
        let layer_channels = match target {
            // A content paint effect paints its owner layer's channels
            PaintTarget::Layer(id) => {
                let owner = self.doc.project.effect(id).map(|(owner, _)| owner).unwrap_or(id);
                self.doc.project.layer(owner).map(|l| l.channels.keys().copied().collect::<BTreeSet<_>>()).unwrap_or_default()
            }
            PaintTarget::Mask(_) => BTreeSet::new(),
        };
        self.tool
            .values
            .iter()
            .filter(|(c, _)| self.tool.enabled.contains(c) && set.has_channel(**c) && layer_channels.contains(c))
            .map(|(c, v)| (*c, *v))
            .collect()
    }

    /// Common setup of 3D and 2D strokes. `first_uv` is where the stroke starts on the mesh
    /// (the Clone tool's destination anchor).
    fn start_stroke(&mut self, viewport: Vec2, uv_mode: bool, first_uv: Option<Vec2>) -> Result<()> {
        self.refuse_while_baking()?;
        let tool = self.tool.tool;
        let erase = tool == Tool::Eraser;
        let quick_mask = self.quick_mask.editing;
        let ts = self.ts();
        let size = self.caches.get(ts).map(|c| c.size).unwrap_or((1024, 1024));
        let pickup = if quick_mask { None } else { self.pickup_mode(first_uv)? };
        let composite_pickup = tool == Tool::Smudge || (tool == Tool::Clone && self.tool.clone_from == CloneFrom::Composite);
        if pickup.is_some() && composite_pickup && !self.paints_mask() && self.solo.is_none() {
            // Only a composite pickup reads the viewport cache
            self.update();
        }

        // Cloning from the selected layer reads that layer alone
        let layer_source = match self.selected_layer() {
            Some(id) if pickup.is_some() && tool == Tool::Clone && self.tool.clone_from == CloneFrom::Layer && !self.paints_mask() => {
                Some(self.flatten(ts, id).ok_or_else(|| EngineError::Other("clone from a paint or fill layer (folders have no content of their own)".into()))?)
            }
            _ => None,
        };
        let before = self.doc.clone();
        let target = if quick_mask {
            None
        } else {
            match self.paint_target() {
                Ok(t) => Some(t),
                Err(e) => {
                    self.doc = before;
                    return Err(e);
                }
            }
        };
        let values = target.map(|t| self.stroke_values(t)).unwrap_or_default();

        // Clone and smudge on a mask read the mask as it was before the stroke
        let mask_owner = match target {
            Some(PaintTarget::Mask(effect)) if pickup.is_some() => self.doc.project.effect(effect).map(|(owner, _)| owner),
            _ => None,
        };
        let stencil = if tool == Tool::Projection && !quick_mask { self.capture_stencil(viewport) } else { None };
        let problem = if !erase && values.is_empty() && matches!(target, Some(PaintTarget::Layer(_))) {
            Some("no channel enabled to paint: enable channels in the tool's material")
        } else if tool == Tool::Projection && !quick_mask && stencil.is_none() && !uv_mode {
            Some("the Projection tool needs a stencil: pick an image in the tool properties (tool.stencil)")
        } else {
            None
        };
        if let Some(msg) = problem {
            self.doc = before;
            return Err(EngineError::Other(msg.into()));
        }

        let (bvh, depth) = if uv_mode {
            (self.bvh.clone(), None)
        } else {
            let (bvh, depth) = self.stroke_visibility(viewport);
            (bvh, Some(depth))
        };
        let stroke = Stroke::new(&self.tool.brush, &self.mesh);
        let mut canvas = StrokeCanvas::new(size.0, size.1).with_stamp_blending(stroke.settings.stamp_blending);
        if !quick_mask {
            canvas.limit = self.stroke_limit(ts, size);
        }
        if let Some((mode, offset)) = pickup
            && let Some(cache) = self.caches.get(ts)
        {
            let mode = match mode {
                PickupMode::Clone { .. } => {
                    PickupMode::Clone { offset: [(offset[0] * size.0 as f32).round() as i64, (offset[1] * size.1 as f32).round() as i64] }
                }
                m => m,
            };
            let mut pickup = match (&layer_source, mask_owner) {
                // On a mask the stroke reads the mask's own values
                (_, Some(owner)) => {
                    let snapshot = texturecraft_paint::pickup::PickupSnapshot {
                        project: self.doc.project.clone(),
                        ts,
                        target: PaintTarget::Mask(owner),
                        mesh: self.mesh.clone(),
                        bounds: self.mesh.bounds(),
                        pixels: self.doc.pixels.clone(),
                        baked: self.doc.baked.get(&ts).cloned().unwrap_or_default(),
                        images: self.images.clone(),
                        graphs: self.doc.graphs.clone(),
                    };
                    Pickup::from_snapshot(mode, snapshot, cache.gbuf.clone(), [])
                }
                (Some(flat), None) => Pickup::from_layer(mode, flat, size, cache.gbuf.clone(), values.keys().copied()),
                (None, None) => Pickup::new(
                    mode,
                    self.document_composite(ts, &cache.covered_tiles).ok_or_else(|| EngineError::Other("texture set not ready".into()))?.into_owned(),
                    cache.gbuf.clone(),
                    values.keys().copied(),
                ),
            };
            if tool == Tool::Smudge && stroke.settings.symmetry.any() {
                // Every mirrored copy of a smudge drags its own trail, in the 3D and the 2D view
                pickup.isolate_copies();
            }
            canvas.pickup = Some(pickup);
            if tool == Tool::Clone
                && self.tool.clone_aligned
                && let Some(src) = self.tool.clone_source.as_mut()
            {
                src.offset.get_or_insert(offset);
            }
        }
        if canvas.pickup.is_none() && tool == Tool::Paint && matches!(target, Some(PaintTarget::Layer(_))) {
            canvas.pickup = self.brush_pattern(&values);
        }
        let label = if quick_mask {
            QUICK_MASK_STROKE_LABEL.to_string()
        } else {
            let on_mask = matches!(target, Some(PaintTarget::Mask(_)));
            history_label(stroke_template(tool, on_mask), &self.stroke_layer_name())
        };
        let settings = &stroke.settings;
        let image_alpha = settings.alpha_image.as_ref().and_then(|name| self.images.get(name)).cloned();
        let library_alpha = if image_alpha.is_none() { settings.library_alpha_image() } else { None };
        let can_record = matches!(tool, Tool::Paint | Tool::Eraser)
            && stencil.is_none()
            && canvas.limit.is_none()
            && canvas.pickup.is_none()
            && !settings.symmetry.any()
            && settings.alpha_image.is_none()
            && target.is_some_and(|target| StrokeReplays::can_record(&self.doc.project, target, ts));
        let replay = if !can_record {
            None
        } else if uv_mode {
            Some(PaintPlacement::Uv { samples: Vec::new(), dabs: Some(Vec::new()) })
        } else {
            self.stroke_visibility
                .as_ref()
                .zip(self.doc.project.texture_sets.get(ts))
                .map(|(scene, set)| PaintPlacement::World { world: WorldPaint { view: scene.view.clone(), dabs: Vec::new(), material: set.material } })
        };
        let gpu_tool = tool == Tool::Paint || (quick_mask && erase) || (tool == Tool::Projection && !uv_mode && stencil.is_some());
        let device = if gpu_tool
            && canvas.pickup.is_none()
            && !settings.symmetry.any()
            && let Some(backend) = &self.brush_device
            && let Some(cache) = self.caches.get(ts)
            && backend.surface.ptr_eq(&std::sync::Arc::downgrade(&cache.gbuf))
            && target.is_none_or(|target| {
                self.doc
                    .project
                    .sets_drawing(self.paint_owner(target))
                    .into_iter()
                    .all(|other| other == ts || self.caches.get(other).is_none_or(|other| other.size != size))
            }) {
            let surface = crate::brush_device::BrushSurface {
                mesh: self.mesh.clone(),
                bvh: bvh.clone(),
                uv: cache.gbuf.clone(),
                material: self.doc.project.texture_set(ts)?.material,
                depth: depth.clone(),
                settings: settings.clone(),
                alpha_image: image_alpha.clone(),
                stencil: stencil.clone(),
                limit: canvas.limit.clone(),
            };
            match backend.device.start(surface) {
                Ok(device) => Some(device),
                Err(error) => {
                    self.log.performance(format!("GPU brush could not start: {error}"));
                    None
                }
            }
        } else {
            None
        };
        if device.is_some() {
            let detail = if stencil.is_some() { "stroke coverage, stencil sampling and live paint" } else { "stroke coverage and live paint" };
            self.log.performance(format!("Brush: GPU ({detail})"));
        }
        self.stroke = Some(ActiveStroke {
            display_id: next_mesh_id(),
            display_tiles: BTreeSet::new(),
            displayed_mask_value: self.tool.mask_value,
            ts,
            target,
            stroke,
            image_alpha,
            library_alpha,
            canvas,
            bvh,
            depth,
            uv: uv_mode.then(UvStrokeState::default),
            camera: self.camera,
            viewport,
            values,
            erase,
            stencil,
            before,
            label,
            staging: Staging::default(),
            device,
            device_uv_stamps: Vec::new(),
            replay,
        });
        Ok(())
    }

    /// The name of the layer a stroke paints on (the owner layer when an effect is selected)
    fn stroke_layer_name(&self) -> String {
        let p = &self.doc.project;
        let Some(id) = self.selected_layer() else { return String::new() };
        let owner = p.effect(id).map_or(id, |(owner, _)| owner);
        p.layer(owner).map(|l| l.name.clone()).unwrap_or_default()
    }

    /// Clone / smudge pickup for the current tool, with the clone offset (source minus
    /// destination) in texture coordinates.
    fn pickup_mode(&self, first_uv: Option<Vec2>) -> Result<Option<(PickupMode, [f32; 2])>> {
        match self.tool.tool {
            Tool::Smudge => Ok(Some((PickupMode::Smudge { strength: self.tool.smudge_strength }, [0.0; 2]))),
            Tool::Clone => {
                let src =
                    self.tool.clone_source.ok_or_else(|| EngineError::Other("set a clone source first: V + click on the mesh (tool.cloneSource)".into()))?;

                // Not aligned: every stroke measures the offset from its own first point
                let kept = if self.tool.clone_aligned { src.offset } else { None };
                let offset = match (kept, first_uv) {
                    (Some(o), _) => o,
                    (None, Some(p)) => [src.uv[0] - p.x, src.uv[1] - p.y],
                    (None, None) => return Err(EngineError::Other("start the clone stroke on the mesh".into())),
                };

                Ok(Some((PickupMode::Clone { offset: [0; 2] }, offset)))
            }
            _ => Ok(None),
        }
    }

    /// The stencil placed for the current camera and viewport.
    fn capture_stencil(&self, viewport: Vec2) -> Option<texturecraft_compose::Stencil> {
        let st = self.tool.stencil.as_ref()?;
        let image = self.images.get(&st.image)?.clone();
        let aspect = viewport.x.max(1.0) / viewport.y.max(1.0);
        let (center, half, rotation) = st.placement(image.width as f32 / image.height.max(1) as f32, aspect);
        Some(texturecraft_compose::Stencil { image, view_proj: self.camera.view_proj(aspect), center, half, rotation, tile: st.tile })
    }

    /// Painting is paused while a bake preview shows the baked maps instead of the material.
    pub(crate) fn refuse_while_baking(&self) -> Result<()> {
        match self.painting_blocked() {
            Some(why) => Err(EngineError::Other(why.into())),
            None => Ok(()),
        }
    }

    /// Start a stroke at `screen` (viewport pixels). Returns false when the cursor misses the
    /// mesh (the stroke starts anyway and resumes when the cursor enters the mesh).
    pub fn begin_stroke(&mut self, screen: Vec2, viewport: Vec2, pressure: f32) -> Result<bool> {
        self.refuse_while_baking()?;
        self.cancel_stroke();
        match self.tool.tool {
            Tool::PolygonFill => return self.polygon_fill(screen, viewport).map(|_| true),
            Tool::ColorPicker => return self.color_pick(screen, viewport).map(|_| true),
            Tool::GeometryMask => return crate::instance_cmds::toggle_part_at(self, screen, viewport).map(|_| true),
            _ => {}
        }
        let first_uv = if self.tool.tool == Tool::Clone { self.pick(screen, viewport).map(|h| h.uv) } else { None };
        self.start_stroke(viewport, false, first_uv)?;
        Ok(self.continue_stroke(screen, pressure))
    }

    /// Start a stroke in the 2D view at texture coordinate `uv` (polygon fill fills there).
    pub fn begin_stroke_uv(&mut self, uv: Vec2, pressure: f32) -> Result<()> {
        self.refuse_while_baking()?;
        self.cancel_stroke();
        match self.tool.tool {
            Tool::PolygonFill => return self.polygon_fill_uv(uv).map(|_| ()),
            Tool::ColorPicker | Tool::GeometryMask => return Err(EngineError::Other("use this tool in the 3D view".into())),
            _ => {}
        }

        self.start_stroke(self.viewport, true, Some(uv))?;
        self.continue_stroke_uv(uv, pressure);
        Ok(())
    }

    /// Add a 2D view sample (texture coordinates). Brush size is relative to the UV square.
    pub fn continue_stroke_uv(&mut self, uv: Vec2, pressure: f32) {
        let Some(active) = self.stroke.as_mut() else { return };
        let Some(progress) = active.uv.as_mut() else { return };
        let Some(cache) = self.caches.get_mut(active.ts) else { return };
        record_sample(&mut active.replay, uv, pressure);

        let settings = &active.stroke.settings;
        let shape = Shape::new(settings.alpha, active.image_alpha.as_deref(), active.library_alpha.as_deref(), settings.alpha_params);

        // The copies of each dab: the Clone tool's pickup offset is fixed, so it paints none
        let material = self.doc.project.texture_sets.get(active.ts).map(|t| t.material).unwrap_or(0);
        let mirror = (self.tool.tool != Tool::Clone)
            .then(|| UvMirror::new(&settings.symmetry, active.stroke.center(), &self.mesh, &self.bvh, &cache.gbuf, material))
            .flatten();
        progress.sample(settings, uv, pressure, |batch| {
            record_uv_dabs(&mut active.replay, batch);
            if let Some(mirror) = &mirror {
                for index in 0..batch.len() {
                    if let Some(stamp) = batch.get(index).copied() {
                        mirror.add_copies(&stamp, batch);
                    }
                }
            }
            if let Some(device) = &mut active.device {
                active.device_uv_stamps.extend(batch.iter().copied());
                match device.append_uv(batch) {
                    Ok(tiles) => {
                        active.display_tiles.extend(&tiles);
                        active.canvas.dirty.extend(tiles);
                    }
                    Err(error) => {
                        active.device = None;
                        active.display_tiles.append(&mut active.canvas.dirty);
                        self.log.performance(format!("GPU brush switched to CPU: {error}"));
                        for stamps in std::mem::take(&mut active.device_uv_stamps).chunks(UV_STAMP_BATCH) {
                            texturecraft_paint::rasterize_uv(&mut active.canvas, &cache.gbuf, shape, stamps);
                        }
                    }
                }
            } else {
                texturecraft_paint::rasterize_uv(&mut active.canvas, &cache.gbuf, shape, batch);
            }
        });
        self.stroke_painted();
    }

    /// Add a pointer sample to the active stroke. Returns whether it hit the mesh.
    pub fn continue_stroke(&mut self, screen: Vec2, pressure: f32) -> bool {
        let Some(st) = self.stroke.as_mut() else { return false };
        let Some(cache) = self.caches.get_mut(st.ts) else { return false };
        let material = self.doc.project.texture_sets.get(st.ts).map(|t| t.material).unwrap_or(0);
        let ctx = PickCtx { mesh: &self.mesh, bvh: &st.bvh, camera: &st.camera, viewport: st.viewport };
        let stamps = st.stroke.add(&ctx, screen, pressure);
        let hit = st.stroke.last_hit().is_some();
        if stamps.is_empty() {
            return hit;
        }
        let rctx = RasterCtx {
            mesh: &self.mesh,
            bvh: &st.bvh,
            gbuf: &cache.gbuf,
            material,
            depth: st.depth.as_deref(),
            eye: st.camera.eye(),
            backface_culling: st.stroke.settings.backface_culling,
            angle_fade: st.stroke.settings.angle_fade,
            shape: Shape::new(st.stroke.settings.alpha, st.image_alpha.as_deref(), st.library_alpha.as_deref(), st.stroke.settings.alpha_params),
            facing: texturecraft_paint::Facing::Front,
            reach: texturecraft_paint::Reach::Plane,
        };
        if let Some(device) = &mut st.device {
            match device.append_world(&stamps) {
                Ok(tiles) => {
                    st.display_tiles.extend(&tiles);
                    st.canvas.dirty.extend(tiles);
                    if st.target.is_none() {
                        self.quick_mask.revision = self.quick_mask.revision.wrapping_add(1);
                    }
                    return hit;
                }
                Err(error) => {
                    st.device = None;
                    st.display_tiles.append(&mut st.canvas.dirty);
                    self.log.performance(format!("GPU brush switched to CPU: {error}"));
                    rasterize(&mut st.canvas, &rctx, &st.stroke.stamps);
                }
            }
        } else {
            rasterize(&mut st.canvas, &rctx, &stamps);
        }
        self.stroke_painted();
        hit
    }

    /// After stamps were rasterized: prepare the tiles the commit will write (so it only swaps
    /// them in) and mark the touched tiles for the live preview.
    fn stroke_painted(&mut self) {
        let Some(st) = self.stroke.as_mut() else { return };
        let recent = std::mem::take(&mut st.canvas.recent);
        let (ts, target) = (st.ts, st.target);
        let Some(t) = target else {
            self.quick_mask.revision = self.quick_mask.revision.wrapping_add(1);
            return;
        };
        let gbuf = self.caches.get(ts).map(|c| c.gbuf().clone());
        let (pixels, mask_value) = (&self.doc.pixels, self.tool.mask_value);
        let mut staging = std::mem::take(&mut st.staging);
        with_commit_source(st, gbuf.as_deref(), &self.mesh, &recent, None, |src| match t {
            PaintTarget::Layer(id) => staging.stage_layer(pixels, id, &st.values, &src),
            PaintTarget::Mask(eid) => staging.stage_mask(pixels, eid, mask_value, &src),
        });
        st.staging = staging;
        st.display_tiles.extend(recent.iter().copied());
        self.mark_painted(ts, target, recent, false);
    }

    pub fn stroke_active(&self) -> bool {
        self.stroke.is_some()
    }

    pub fn stroke_texture_set(&self) -> Option<usize> {
        self.stroke.as_ref().map(|stroke| stroke.ts)
    }

    pub fn stroke_mask_owner(&self) -> Option<LayerId> {
        let Some(PaintTarget::Mask(effect)) = self.stroke.as_ref()?.target else { return None };
        self.doc.project.effect(effect).map(|(layer, _)| layer)
    }

    /// The live material compositor uses the device's coverage directly while the stroke is active.
    pub fn device_stroke(&self) -> Option<DeviceStrokePreview<'_>> {
        let stroke = self.stroke.as_ref()?;
        Some(DeviceStrokePreview {
            ts: stroke.ts,
            target: stroke.target?,
            values: &stroke.values,
            mask_value: self.tool.mask_value,
            opacity: stroke.stroke.settings.opacity,
            stroke: stroke.device.as_deref()?,
        })
    }

    /// Finish the GPU coverage when committing or switching to a CPU-only preview.
    pub fn materialize_device_stroke(&mut self, preview_fallback: Option<&str>) {
        let _ = self.take_device_stroke(preview_fallback);
    }

    fn take_device_stroke(&mut self, preview_fallback: Option<&str>) -> Option<Box<dyn DeviceStroke>> {
        let stroke = self.stroke.as_mut()?;
        let device = stroke.device.take()?;
        let valid = match device.read_alpha(&stroke.canvas.dirty) {
            Ok(alpha) => {
                stroke.canvas.dirty = alpha.allocated().collect();
                stroke.canvas.alpha = alpha;
                true
            }
            Err(error) => {
                stroke.display_tiles.append(&mut stroke.canvas.dirty);
                self.log.performance(format!("GPU stroke readback failed, replaying stroke on CPU: {error}"));
                let cache = self.caches.get(stroke.ts)?;
                let settings = &stroke.stroke.settings;
                let shape = Shape::new(settings.alpha, stroke.image_alpha.as_deref(), stroke.library_alpha.as_deref(), settings.alpha_params);
                if stroke.uv.is_some() {
                    for stamps in stroke.device_uv_stamps.chunks(UV_STAMP_BATCH) {
                        texturecraft_paint::rasterize_uv(&mut stroke.canvas, &cache.gbuf, shape, stamps);
                    }
                } else {
                    let context = RasterCtx {
                        mesh: &self.mesh,
                        bvh: &stroke.bvh,
                        gbuf: &cache.gbuf,
                        material: self.doc.project.texture_sets.get(stroke.ts).map_or(0, |set| set.material),
                        depth: stroke.depth.as_deref(),
                        eye: stroke.camera.eye(),
                        backface_culling: stroke.stroke.settings.backface_culling,
                        angle_fade: stroke.stroke.settings.angle_fade,
                        shape,
                        facing: texturecraft_paint::Facing::Front,
                        reach: texturecraft_paint::Reach::Plane,
                    };
                    rasterize(&mut stroke.canvas, &context, &stroke.stroke.stamps);
                }
                false
            }
        };
        stroke.device_uv_stamps.clear();
        stroke.display_tiles.extend(&stroke.canvas.dirty);
        stroke.canvas.recent.extend(&stroke.canvas.dirty);
        if let Some(reason) = preview_fallback {
            self.log.performance(format!("Brush: CPU ({reason})"));
            self.stroke_painted();
        }
        valid.then_some(device)
    }

    /// Identity and tiles of a supported live stroke since the display last drew it.
    pub fn pending_display_stroke(&self, ts: usize) -> Option<(u64, &BTreeSet<(u32, u32)>)> {
        let stroke = self.stroke.as_ref()?;
        let tiles = if matches!(stroke.target, Some(PaintTarget::Mask(_))) && stroke.displayed_mask_value != self.tool.mask_value {
            &stroke.canvas.dirty
        } else {
            &stroke.display_tiles
        };
        let can_preview = stroke.stencil.is_none() || stroke.device.is_some();
        (stroke.ts == ts && stroke.target.is_some() && can_preview && stroke.canvas.pickup.is_none()).then_some((stroke.display_id, tiles))
    }

    /// Copy-on-write material pixels for display. The staged blends are the same tiles the
    /// completed stroke will install. The live viewport does not change the document or Undo.
    pub fn display_stroke_pixels(&self, ts: usize, texel_bounds: Option<([u32; 2], [u32; 2])>) -> Option<texturecraft_compose::PixelStore> {
        self.pending_display_stroke(ts)?;
        let stroke = self.stroke.as_ref()?;
        if stroke.device.is_some() {
            return None;
        }
        let tiles_in_bounds;
        let touched = if let Some((min, max)) = texel_bounds {
            tiles_in_bounds = stroke
                .canvas
                .dirty
                .iter()
                .copied()
                .filter(|&(x, y)| {
                    let (x, y) = (x * TILE, y * TILE);
                    x >= min[0] && x < max[0] && y >= min[1] && y < max[1]
                })
                .collect();
            &tiles_in_bounds
        } else {
            &stroke.canvas.dirty
        };
        let size = (stroke.canvas.alpha.width(), stroke.canvas.alpha.height());
        let mut pixels = self.doc.pixels.clone();
        let mut staging = stroke.staging.snapshot(touched);
        with_commit_source(stroke, None, &self.mesh, touched, None, |source| match stroke.target {
            Some(PaintTarget::Layer(id)) => commit_layer(&mut pixels, id, size, &stroke.values, source, &mut staging),
            Some(PaintTarget::Mask(id)) => commit_mask(&mut pixels, id, size, self.tool.mask_value, source, &mut staging),
            None => {}
        });
        Some(pixels)
    }

    pub fn clear_display_stroke_tiles(&mut self) {
        if let Some(stroke) = self.stroke.as_mut() {
            stroke.display_tiles.clear();
            stroke.displayed_mask_value = self.tool.mask_value;
        }
    }

    pub fn finished_display_stroke(&self, ts: usize, id: u64, previous_revision: u64) -> Option<&BTreeSet<(u32, u32)>> {
        let end = self.display_stroke_end.as_ref()?;
        let revision = self.caches.get(ts)?.material_revision;
        (end.ts == ts && end.id == id && end.previous_revision == previous_revision && end.revision == revision).then_some(&end.tiles)
    }

    /// Commit the active stroke (one undo step).
    pub fn end_stroke(&mut self) -> Result<()> {
        let device = self.take_device_stroke(None);
        let brush_tile = self.tile_demand(None).brush;
        let Some(mut st) = self.stroke.take() else { return Ok(()) };
        let size = (st.canvas.alpha.width(), st.canvas.alpha.height());
        if st.canvas.dirty.is_empty() {
            // Nothing painted: drop any mask effect the stroke created.
            self.doc = st.before;
            return Ok(());
        }
        let gbuf = self.caches.get(st.ts).map(|c| c.gbuf().clone());
        let mesh = self.mesh.clone();
        let mut staging = std::mem::take(&mut st.staging);
        let mask_value = self.tool.mask_value;
        let Some(target) = st.target else {
            with_commit_source(&st, gbuf.as_deref(), &mesh, &st.canvas.dirty, None, |src| self.commit_quick_mask(st.ts, size, src));
            return Ok(());
        };
        let pixels = &mut self.doc.pixels;
        let mut commit = |tiles: &BrushTiles, projection: Option<&ProjectedTiles>| {
            with_commit_source(&st, gbuf.as_deref(), &mesh, tiles, projection, |src| match target {
                PaintTarget::Layer(id) => commit_layer(pixels, id, size, &st.values, src, &mut staging),
                PaintTarget::Mask(eid) => commit_mask(pixels, eid, size, mask_value, src, &mut staging),
            });
        };
        if let Some(device) = device.as_deref().filter(|_| st.stencil.is_some()) {
            let tiles: Vec<_> = st.canvas.dirty.iter().copied().collect();
            let mut gpu_readback = Some(device);
            for batch in tiles.chunks(PROJECTION_READBACK_BATCH) {
                let tiles: BrushTiles = batch.iter().copied().collect();
                let projection = gpu_readback.and_then(|device| match device.read_projection(&tiles) {
                    Ok(samples) if tiles.iter().all(|tile| samples.get(tile).is_some_and(|data| data.len() == TILE_AREA)) => Some(samples),
                    Ok(_) => {
                        self.log.performance("GPU stencil readback missed tiles, sampling remaining tiles on CPU");
                        gpu_readback = None;
                        None
                    }
                    Err(error) => {
                        self.log.performance(format!("GPU stencil readback failed, sampling remaining tiles on CPU: {error}"));
                        gpu_readback = None;
                        None
                    }
                });
                commit(&tiles, projection.as_ref());
            }
        } else {
            commit(&st.canvas.dirty, None);
        }
        let operation = st.replay.take().map(|mut placement| {
            if let PaintPlacement::World { world } = &mut placement {
                world.dabs = std::mem::take(&mut st.stroke.stamps);
            }
            RecordedPaint { settings: st.stroke.settings.clone(), placement, values: st.values.clone(), mask_value, erase: st.erase }
        });
        let material = self.doc.project.texture_sets.get(st.ts).map_or(0, |set| set.material);
        if self.doc.stroke_replays.commit(target, operation, &st.before.pixels, &self.doc.pixels, &self.mesh, material) {
            self.note_raster_paint();
        }
        if matches!(target, PaintTarget::Mask(_)) && st.displayed_mask_value != mask_value {
            st.display_tiles.extend(&st.canvas.dirty);
        }
        self.undo.push((st.label.clone(), st.before));
        if self.undo.len() > crate::session::MAX_HISTORY {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.modified = true;
        self.revision = self.revision.wrapping_add(1);
        let previous_revision = self.caches.get(st.ts).map_or(0, |cache| cache.material_revision);
        // The live preview already shows the committed result: the stroke's tiles recomposite
        // from the committed pixels at low priority, without holding up this frame.
        self.mark_settle(st.ts, st.target, st.canvas.dirty.iter().copied());
        if let Some(cache) = self.caches.get_mut(st.ts) {
            self.display_stroke_end =
                Some(DisplayStrokeEnd { id: st.display_id, ts: st.ts, previous_revision, revision: cache.material_revision, tiles: st.display_tiles });
            cache.finished_dab = brush_tile
                .filter(|tile| st.canvas.dirty.contains(tile) && (cache.dirty.contains(tile) || cache.upload.contains(tile) || cache.settle.contains(tile)))
                .map(ReleasedDab::new);
        }
        Ok(())
    }

    /// Abandon the active stroke (Esc).
    pub fn cancel_stroke(&mut self) {
        if let Some(st) = self.stroke.take() {
            self.mark_painted(st.ts, st.target, st.canvas.dirty.iter().copied(), false);
            if st.target.is_none() {
                self.quick_mask.revision = self.quick_mask.revision.wrapping_add(1);
            }
            self.doc = st.before;
            // A paint effect created by the stroke can get the same ID on a different owner.
            for cache in &mut self.caches {
                cache.invalidate_paint_margin();
            }
            if let Some(cache) = self.caches.get(st.ts) {
                let revision = cache.material_revision;
                self.display_stroke_end =
                    Some(DisplayStrokeEnd { id: st.display_id, ts: st.ts, previous_revision: revision, revision, tiles: st.canvas.dirty });
            }
        }
    }

    /// Paint a whole stroke through viewport points (scripted strokes from CLI/MCP).
    pub fn stroke_points(&mut self, points: &[Vec2], viewport: Vec2) -> Result<usize> {
        let Some(first) = points.first() else { return Err(EngineError::Other("a stroke needs at least one point".into())) };
        self.begin_stroke(*first, viewport, 1.0)?;
        let mut hits = 0;
        for p in points.iter().skip(1) {
            if self.continue_stroke(*p, 1.0) {
                hits += 1;
            }
        }
        let stamps = self.stroke.as_ref().map(|s| s.stroke.stamps.len()).unwrap_or(0);
        self.end_stroke()?;
        let _ = hits;
        Ok(stamps)
    }

    /// Paint a whole 2D view stroke through texture coordinates. Returns the touched tiles.
    pub fn stroke_uv_points(&mut self, points: &[Vec2]) -> Result<usize> {
        let Some(first) = points.first() else { return Err(EngineError::Other("a stroke needs at least one point".into())) };
        self.begin_stroke_uv(*first, 1.0)?;
        for q in points.iter().skip(1) {
            self.continue_stroke_uv(*q, 1.0);
        }
        self.materialize_device_stroke(None);
        let tiles = self.stroke_tiles();
        self.end_stroke()?;
        Ok(tiles)
    }

    /// The G-buffer of texture set `ts` and an empty canvas for a one-shot fill, limited by the
    /// quick mask unless the fill edits it.
    pub(crate) fn fill_canvas(&self, ts: usize) -> Result<(std::sync::Arc<texturecraft_geom::UvGBuffer>, StrokeCanvas)> {
        let gbuf = self.caches.get(ts).map(|c| c.gbuf().clone()).ok_or_else(|| EngineError::Other("texture set not ready".into()))?;
        let mut canvas = StrokeCanvas::new(gbuf.width, gbuf.height);
        if !self.quick_mask.editing {
            canvas.limit = self.stroke_limit(ts, (gbuf.width, gbuf.height));
        }
        Ok((gbuf, canvas))
    }

    /// Commit a one-shot fill with the tool's material (one undo step), or into the quick mask.
    pub(crate) fn commit_fill(&mut self, label: &str, ts: usize, canvas: &StrokeCanvas) -> Result<()> {
        if self.quick_mask.editing {
            return self.paint_fill(ts, canvas);
        }

        // Each fill is its own step, never merged into a box drag's step of the same label
        self.edit_merging(label, false, |session| session.paint_fill(ts, canvas))
    }

    /// Paint a one-shot fill with the tool's material into the paint target, or into the quick mask while it is edited (no undo step)
    pub(crate) fn paint_fill(&mut self, ts: usize, canvas: &StrokeCanvas) -> Result<()> {
        let size = (canvas.alpha.width(), canvas.alpha.height());
        let opacity = self.tool.brush.opacity;
        let source = CommitSource { alpha: &canvas.alpha, tiles: &canvas.dirty, opacity, erase: false, per_texel: None, carried: None, carried_mask: None };
        if self.quick_mask.editing {
            self.commit_quick_mask(ts, size, source);
            return Ok(());
        }

        let target = self.paint_target()?;
        let values = self.stroke_values(target);
        match target {
            PaintTarget::Layer(id) => commit_layer(&mut self.doc.pixels, id, size, &values, source, &mut Staging::default()),
            PaintTarget::Mask(effect) => commit_mask(&mut self.doc.pixels, effect, size, self.tool.mask_value, source, &mut Staging::default()),
        }
        Ok(())
    }

    /// Material picker: copy the material under the cursor into the tool's paint values
    pub fn color_pick(&mut self, screen: Vec2, viewport: Vec2) -> Result<BTreeMap<Channel, [f32; 3]>> {
        let hit = self.pick(screen, viewport).ok_or_else(|| EngineError::Other("nothing under the cursor".into()))?;
        let texture_set = self.texture_set_of_material(hit.material).unwrap_or(self.ts());
        let cache = self.caches.get(texture_set).ok_or_else(|| EngineError::Other("texture set not ready".into()))?;

        let (width, height) = (cache.size.0 as f32, cache.size.1 as f32);
        let (x, y) = ((hit.uv.x * width).clamp(0.0, width - 1.0) as u32, (hit.uv.y * height).clamp(0.0, height - 1.0) as u32);

        let picked = self.document_sample(texture_set, x, y).ok_or_else(|| EngineError::Other("texture set not ready".into()))?;
        self.tool.values.extend(&picked);
        self.tool.material = None;
        Ok(picked)
    }

    /// The stroke canvas alpha of the active stroke (for overlays and tests).
    pub fn stroke_tiles(&self) -> usize {
        self.stroke.as_ref().map(|s| s.canvas.dirty.len()).unwrap_or(0)
    }

    /// Where the Clone tool reads for destination `at` (texture coordinates), in texture
    /// coordinates: the source itself until the first stroke fixes the offset, or when `at` is
    /// None. None without a source or when that spot is outside the texture. The far edge
    /// (1.0) counts as inside, so the 2D marker shows a source set on the edge; reads of the
    /// surface use `clone_source_uv`, which stays half-open.
    pub fn clone_source_marker_uv(&self, at: Option<Vec2>) -> Option<Vec2> {
        let src = self.tool.clone_source?;
        let uv = match (src.offset, at) {
            (Some(o), Some(p)) => Vec2::new(p.x + o[0], p.y + o[1]),
            _ => Vec2::from(src.uv),
        };
        ((0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y)).then_some(uv)
    }

    /// Like `clone_source_marker_uv`, but half-open: None on the far edge, where the G-buffer
    /// has no texel.
    pub fn clone_source_uv(&self, at: Option<Vec2>) -> Option<Vec2> {
        self.clone_source_marker_uv(at).filter(|uv| uv.x < 1.0 && uv.y < 1.0)
    }

    /// Where the Clone tool reads for destination `at` (texture coordinates) as a surface
    /// point: the source itself until the first stroke fixes the offset, or when `at` is None.
    pub fn clone_source_point(&self, at: Option<Vec2>) -> Option<texturecraft_geom::glam::Vec3> {
        let uv = self.clone_source_uv(at)?;
        let g = &self.caches.get(self.ts())?.gbuf;
        let (x, y) = ((uv.x * g.width as f32) as usize, (uv.y * g.height as f32) as usize);
        Some(g.surface(&self.mesh, y * g.width as usize + x)?.pos)
    }

    pub fn selected_layer_id(&self) -> Option<LayerId> {
        self.selected_layer()
    }
}

/// Run `f` with stroke `st` as a commit source over `tiles` (the Projection tool's stencil
/// colors and alpha come from the surface under each texel).
fn with_commit_source<R>(
    st: &ActiveStroke,
    gbuf: Option<&texturecraft_geom::UvGBuffer>,
    mesh: &texturecraft_geom::Mesh,
    tiles: &BTreeSet<(u32, u32)>,
    projection: Option<&ProjectedTiles>,
    f: impl FnOnce(CommitSource) -> R,
) -> R {
    let stencil_fn = |x: u32, y: u32| -> Option<([f32; 3], f32)> {
        if let Some(projection) = projection {
            let tile = projection.get(&(x / TILE, y / TILE))?;
            let i = ((y % TILE) * TILE + x % TILE) as usize;
            let &[r, g, b, a] = tile.get(i)?;
            return Some(([r, g, b], a));
        }
        let g = gbuf?;
        let p = g.surface(mesh, (y * g.width + x) as usize)?.pos;
        st.stencil.as_ref()?.sample_world(p)
    };
    let per_texel: Option<texturecraft_paint::TexelFn> = if st.stencil.is_some() { Some(&stencil_fn) } else { None };
    let carried = st.canvas.pickup.as_ref().map(|p| &p.values);
    let carried_mask = st.canvas.pickup.as_ref().and_then(|p| p.mask_values());
    f(CommitSource { alpha: &st.canvas.alpha, tiles, opacity: st.stroke.settings.opacity, erase: st.erase, per_texel, carried, carried_mask })
}
