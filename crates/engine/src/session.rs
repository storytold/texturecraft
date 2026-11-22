//! The editing session: document, mesh, per-texture-set caches, history, tool state.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use texturecraft_compose::{BakedMaps, BelowCache, Composite, PaintTarget, PixelStore, StackKey, StrokePreview, covered_tiles};
use texturecraft_core::{Channel, Layer, LayerId, LayerKind, Project, TextureSet};
use texturecraft_geom::glam::Vec2;
use texturecraft_geom::{Bvh, Mesh, OrbitCamera, UvGBuffer};
use texturecraft_paint::{BrushSettings, EyeDepth, Stroke, StrokeCanvas};
use texturecraft_texel::TILE;

pub use crate::display_padding::{DisplayPadding, UvLookup};
use crate::fill_mode::FillMode;
pub use crate::stencil_look::StencilLook;
use crate::{EngineError, Result};

/// Padding (texels) applied to composites for display.
pub const DISPLAY_PADDING: u32 = 8;
/// Undo steps kept.
pub const MAX_HISTORY: usize = 200;

/// Everything that undo/redo restores. Pixel data is copy-on-write, so snapshots are cheap.
#[derive(Clone, Debug)]
pub struct Document {
    pub project: Project,
    pub pixels: PixelStore,
    pub(crate) stroke_replays: crate::stroke_replay::StrokeReplays,
    pub baked: BTreeMap<usize, BakedMaps>,
    pub bake_paint: BTreeMap<(usize, texturecraft_core::BakePaintTarget), Arc<texturecraft_bake::BakePaintMap>>,
    /// Imported image resources by name: original file bytes.
    pub resources: BTreeMap<String, Arc<Vec<u8>>>,
    /// Texts of the graphs the layers run, and their cooked outputs (shared by every snapshot).
    pub graphs: texturecraft_compose::GraphStore,
    /// Stroke records behind imported pixels, by the id that keys the pixels (saved with the project).
    pub strokes: BTreeMap<texturecraft_core::LayerId, texturecraft_core::StoredStrokes>,
}

/// A released dab keeps priority until its display upload is sent.
#[derive(Clone)]
pub struct ReleasedDab {
    tile: (u32, u32),
    uploaded: Arc<AtomicBool>,
}

impl ReleasedDab {
    pub(crate) fn new(tile: (u32, u32)) -> Self {
        Self { tile, uploaded: Arc::default() }
    }

    pub(crate) fn pending_tile(&self) -> Option<(u32, u32)> {
        (!self.uploaded.load(Ordering::Acquire)).then_some(self.tile)
    }

    pub fn acknowledge_upload(&self) {
        self.uploaded.store(true, Ordering::Release);
    }
}

/// Derived per-texture-set data (rebuilt when the mesh or resolution changes).
pub struct SetCache {
    /// Texels across and down (`TextureSet::size`).
    pub size: (u32, u32),
    pub gbuf: UvLookup,
    pub dilation: DisplayPadding,
    pub covered_tiles: Arc<Vec<(u32, u32)>>,
    pub composite: Composite,
    /// Tiles to recomposite.
    pub dirty: BTreeSet<(u32, u32)>,
    /// Tiles to recomposite at low priority: what they show is already right (a committed
    /// stroke's live preview), but it was composited from the stroke, not the layer pixels.
    pub settle: BTreeSet<(u32, u32)>,
    /// Tiles whose display textures must be re-uploaded.
    pub upload: BTreeSet<(u32, u32)>,
    /// A dab released before its first display upload.
    pub(crate) finished_dab: Option<ReleasedDab>,
    /// Bumps on every composite change (thumbnails, exports, caches).
    pub revision: u64,
    /// Changes when this set's material inputs change, independently of CPU display work.
    pub material_revision: u64,
    /// Everything but the layer being edited, per tile (repaints evaluate only that layer).
    pub below: BelowCache,
    /// Tiles per composite chunk that fit the last frame's budget (0: not measured yet).
    pub chunk: usize,
    /// Invalidated when the document stack is synchronized.
    paint_margin: Option<(PaintTarget, Option<u32>)>,
}

impl SetCache {
    pub(crate) fn new(mesh: &Arc<Mesh>, ts: &TextureSet, lookup_budget: usize) -> SetCache {
        let (w, h) = ts.size();
        let coverage = Arc::new(UvGBuffer::new(mesh, ts.material, w, h));
        let seed = (UvLookup::raster_bytes(&coverage) <= lookup_budget).then(|| Arc::clone(&coverage));
        let gbuf = UvLookup::lazy(Arc::clone(mesh), ts.material, (w, h), seed);
        // Padding stays inside each UV tile.
        let covered = covered_tiles(&coverage);
        let dilation = DisplayPadding::with_lookup(&gbuf, &coverage, !covered.is_empty(), ts.block_resolution());
        let all: BTreeSet<(u32, u32)> = texturecraft_compose::all_tiles(w, h).into_iter().collect();
        SetCache {
            size: (w, h),
            gbuf,
            dilation,
            covered_tiles: Arc::new(covered),
            composite: Composite { block_scales: ts.block_scales(), ..Composite::with_size(&ts.channel_list(), w, h) },
            dirty: all,
            settle: BTreeSet::new(),
            upload: BTreeSet::new(),
            finished_dab: None,
            revision: 0,
            material_revision: 0,
            below: BelowCache::default(),
            chunk: 0,
            paint_margin: None,
        }
    }

    /// Replay a lost or recreated GPU texture. Tiles still awaiting composition will be
    /// queued with their finished pixels by update_tiles.
    pub fn queue_ready_uploads(&mut self) {
        let (cols, rows) = (self.size.0.div_ceil(TILE), self.size.1.div_ceil(TILE));
        if self.dirty.len() < cols as usize * rows as usize {
            self.upload.extend((0..cols).flat_map(|x| (0..rows).map(move |y| (x, y))).filter(|t| !self.dirty.contains(t)));
        }
    }

    /// Drain one upload slice, giving the brush and visible UVs first use of it.
    pub fn take_upload_batch(&mut self, limit: usize, demand: crate::TileDemand) -> Vec<(u32, u32)> {
        if self.upload.len() <= limit {
            std::mem::take(&mut self.upload).into_iter().collect()
        } else {
            let mut tiles = demand.take(&mut self.upload, limit);
            tiles.extend((tiles.len()..limit).filter_map(|_| self.upload.pop_first()));
            tiles
        }
    }

    /// Carry this receipt with the scene so a lost render callback leaves the dab pending.
    pub fn released_dab_in(&self, tiles: &[(u32, u32)]) -> Option<ReleasedDab> {
        let dab = self.finished_dab.as_ref()?;
        tiles.contains(&dab.pending_tile()?).then(|| dab.clone())
    }

    fn queue_painted(
        &mut self,
        project: &Project,
        ts: usize,
        target: PaintTarget,
        tiles: &[(u32, u32)],
        queue: impl FnOnce(&mut Self) -> &mut BTreeSet<(u32, u32)>,
    ) {
        if tiles.is_empty() {
            return;
        }
        let margin = match self.paint_margin {
            Some((cached, margin)) if cached == target => margin,
            _ => {
                let margin = texturecraft_compose::eval::painted_tile_margin(project, ts, target);
                self.paint_margin = Some((target, margin));
                margin
            }
        };
        let covered = Arc::clone(&self.covered_tiles);
        let (width, height) = self.size;
        let queue = queue(self);
        match margin {
            Some(0) => queue.extend(tiles.iter().copied()),
            Some(radius) => {
                let (last_x, last_y) = (width.saturating_sub(1) / TILE, height.saturating_sub(1) / TILE);
                let mut add_rect = |(left, top): (u32, u32), (right, bottom): (u32, u32)| {
                    let left = left.saturating_sub(radius);
                    let right = right.saturating_add(radius).min(last_x);
                    for row in top.saturating_sub(radius)..=bottom.saturating_add(radius).min(last_y) {
                        // Covered tiles are sorted by row, then column.
                        let start = covered.partition_point(|&(tx, ty)| (ty, tx) < (row, left));
                        queue.extend(covered[start..].iter().take_while(|&&(tx, ty)| ty == row && tx <= right).copied());
                    }
                };
                // Coalesce expensive, overlapping halos without scanning distant atlas tiles.
                let diameter = (radius as usize).saturating_mul(2).saturating_add(1);
                if diameter.saturating_mul(diameter).saturating_mul(tiles.len()) >= covered.len() {
                    let bounds = tiles.iter().fold(((last_x, last_y), (0, 0)), |((l, t), (r, b)), &(x, y)| ((l.min(x), t.min(y)), (r.max(x), b.max(y))));
                    add_rect(bounds.0, bounds.1);
                } else {
                    for &tile in tiles {
                        add_rect(tile, tile);
                    }
                }
            }
            None => queue.extend(covered.iter().copied()),
        }
    }

    /// The UV lookup: which triangle each texel shows, and where on it.
    pub fn gbuf(&self) -> &Arc<UvGBuffer> {
        &self.gbuf
    }

    /// The dense map that pads the composite past the UV islands, built on first use.
    pub fn dilation(&self) -> &Arc<texturecraft_texel::DilationMap> {
        &self.dilation
    }

    /// Bytes the UV lookup and padding map hold (0 while they are not built).
    pub fn lookup_bytes(&self) -> usize {
        self.gbuf.bytes().saturating_add(self.dilation.bytes())
    }

    /// Lets go of the UV lookup and padding map; both rebuild from the same mesh.
    pub fn release_lookup(&mut self) {
        self.gbuf = self.gbuf.evicted();
        self.dilation.evict(&self.gbuf);
    }

    pub fn invalidate_all(&mut self) {
        self.material_revision = self.material_revision.wrapping_add(1);
        self.dirty.extend(self.covered_tiles.iter().copied());
    }

    pub(crate) fn invalidate_paint_margin(&mut self) {
        self.paint_margin = None;
    }
}

/// Active tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Tool {
    #[default]
    Paint,
    Eraser,
    Projection,
    PolygonFill,
    Smudge,
    Clone,
    ColorPicker,
    /// Paint along an editable curve (or fill a closed one).
    Path,
    /// Click mesh parts to show or hide the selected layer on them.
    GeometryMask,
}

impl Tool {
    pub const ALL: [Tool; 9] =
        [Tool::Paint, Tool::Eraser, Tool::Projection, Tool::PolygonFill, Tool::Smudge, Tool::Clone, Tool::ColorPicker, Tool::Path, Tool::GeometryMask];
    pub fn label(self) -> &'static str {
        match self {
            Tool::Paint => "Paint",
            Tool::Eraser => "Eraser",
            Tool::Projection => "Projection",
            Tool::PolygonFill => "Polygon fill",
            Tool::Smudge => "Smudge",
            Tool::Clone => "Clone",
            Tool::ColorPicker => "Material picker",
            Tool::Path => "Path",
            Tool::GeometryMask => "Geometry mask",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Tool::Paint => "paint",
            Tool::Eraser => "eraser",
            Tool::Projection => "projection",
            Tool::PolygonFill => "polygonFill",
            Tool::Smudge => "smudge",
            Tool::Clone => "clone",
            Tool::ColorPicker => "colorPicker",
            Tool::Path => "path",
            Tool::GeometryMask => "geometryMask",
        }
    }
    pub fn parse(s: &str) -> Option<Tool> {
        let n = |x: &str| x.to_ascii_lowercase().replace([' ', '_', '-'], "");
        Tool::ALL.into_iter().find(|t| n(t.id()) == n(s) || n(t.label()) == n(s))
    }
}

/// What the viewport displays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewMode {
    /// The lit material.
    #[default]
    Material,
    /// One channel, unlit.
    Channel(Channel),
    /// A baked mesh map.
    MeshMap(texturecraft_core::BakedMap),
    /// UV checker (texel density and seams).
    UvChecker,
    /// One layer's mask, in grayscale (Alt+click a mask).
    Mask(LayerId),
    /// The surface normal the shading uses, unlit: the mesh normal turned by the Height
    /// channel's slope and the Normal channel ("Normal+height+mesh").
    NormalHeightMesh,
    /// One of the project's image resources shown on the mesh (and in the 2D view) as it is: the
    /// picker's "Other..." (the image is [`Session::view_image`]).
    Image,
}

/// The view picker's entry for [`ViewMode::NormalHeightMesh`]
pub const NORMAL_HEIGHT_MESH_LABEL: &str = "Normal+height+mesh";

impl ViewMode {
    pub fn label(&self) -> String {
        match self {
            ViewMode::Material => "Material".into(),
            ViewMode::Channel(c) => c.label().into(),
            ViewMode::MeshMap(m) => m.id().into(),
            ViewMode::UvChecker => "UV checker".into(),
            ViewMode::Mask(_) => "Layer mask".into(),
            ViewMode::NormalHeightMesh => NORMAL_HEIGHT_MESH_LABEL.into(),
            ViewMode::Image => "Other...".into(),
        }
    }
}

/// A screen-pinned image for the Projection tool (a stencil).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StencilState {
    /// Image resource name.
    pub image: String,
    /// Center in normalized device coordinates (-1..1, y up).
    pub center: [f32; 2],
    /// Height of the image as a fraction of the viewport height (1 = full height).
    pub scale: f32,
    /// Rotation in degrees.
    pub rotation: f32,
    /// Repeat the image across the screen.
    pub tile: bool,
    /// Overlay opacity in the viewport (display only).
    pub opacity: f32,
}

impl StencilState {
    pub fn new(image: &str) -> StencilState {
        StencilState::with_look(image, StencilLook::default())
    }

    /// A stencil of `image` placed as `look` says.
    pub fn with_look(image: &str, look: StencilLook) -> StencilState {
        StencilState { image: image.to_string(), center: look.center, scale: look.scale, rotation: look.rotation, tile: look.tile, opacity: look.opacity }
    }

    pub fn look(&self) -> StencilLook {
        StencilLook { center: self.center, scale: self.scale, rotation: self.rotation, tile: self.tile, opacity: self.opacity }
    }

    /// Back to the placement a new stencil starts with (center, scale and rotation); the image, tiling and opacity stay.
    pub fn reset_placement(&mut self) {
        let start = StencilState::new(&self.image);
        self.center = start.center;
        self.scale = start.scale;
        self.rotation = start.rotation;
    }

    /// Placement for a viewport aspect (width / height): center, half extents in NDC, radians.
    pub fn placement(&self, image_aspect: f32, viewport_aspect: f32) -> ([f32; 2], [f32; 2], f32) {
        let hy = self.scale.max(0.01);
        let hx = hy * image_aspect / viewport_aspect.max(1e-3);
        (self.center, [hx, hy], self.rotation.to_radians())
    }
}

/// Tool settings shared by the UI, CLI and agents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolState {
    pub tool: Tool,
    pub brush: BrushSettings,
    /// Material painted per channel (linear values) and whether each channel is painted.
    pub values: BTreeMap<Channel, [f32; 3]>,
    pub enabled: BTreeSet<Channel>,
    /// The simple material (shelf id) the brush paints the pattern of, with `values` as its plain
    /// stand-in; `None` for plain values (any edit of a value clears it).
    #[serde(default)]
    pub material: Option<String>,
    /// Gray painted into masks.
    pub mask_value: f32,
    pub fill_mode: FillMode,
    /// How a polygon fill box decides which polygons it selects
    #[serde(default)]
    pub fill_hit_test: crate::fill_hit_test::FillHitTest,
    /// Polygon fill boxes in the 3D view select only the polygons the camera sees
    #[serde(default = "default_fill_visible_only")]
    pub fill_visible_only: bool,
    /// Stencil used by the Projection tool.
    #[serde(default)]
    pub stencil: Option<StencilState>,
    /// Placement the next stencil starts with: the last stencil's, kept when the stencil is
    /// cleared or the project changes (its image belongs to the project, the placement does not).
    #[serde(default)]
    pub stencil_look: StencilLook,
    /// Smudge tool: how much of the picked-up values each stamp deposits (0..1).
    #[serde(default = "default_smudge_strength")]
    pub smudge_strength: f32,
    /// Clone tool source (V + click). It belongs to the active texture set: switching the set
    /// (`textureSet.select`, `layer.select` into another set) clears it.
    #[serde(default)]
    pub clone_source: Option<CloneSource>,
    /// Clone tool: keep the offset of the first stroke for the following strokes (aligned, the
    /// default). Off, every stroke starts sampling at the source point.
    #[serde(default = "default_clone_aligned")]
    pub clone_aligned: bool,
    /// What the Clone tool copies: the composite or the selected layer alone.
    #[serde(default)]
    pub clone_from: CloneFrom,
    /// The Path tool's curve being edited.
    #[serde(default)]
    pub path: crate::path_tool::PathState,
}

pub(crate) fn default_smudge_strength() -> f32 {
    0.5
}

fn default_clone_aligned() -> bool {
    true
}

fn default_fill_visible_only() -> bool {
    true
}

/// What the Clone tool copies (the clone source modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CloneFrom {
    /// Everything the texture set shows.
    #[default]
    Composite,
    /// The selected layer only (its content and mask; empty parts copy nothing).
    Layer,
}

/// Where the Clone tool copies from, in texture coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloneSource {
    pub uv: [f32; 2],
    /// Source minus destination, fixed by the first stroke after the source was set and kept
    /// for the following strokes (aligned cloning; never set when the tool is not aligned).
    pub offset: Option<[f32; 2]>,
}

impl Default for ToolState {
    fn default() -> Self {
        let values = Channel::ALL.iter().map(|c| (*c, c.default_value())).collect();
        let mut t = ToolState {
            tool: Tool::Paint,
            brush: BrushSettings::default(),
            values,
            enabled: [Channel::BaseColor, Channel::Metallic, Channel::Roughness, Channel::Height, Channel::Normal].into_iter().collect(),
            material: None,
            mask_value: 1.0,
            fill_mode: FillMode::Triangle,
            fill_hit_test: Default::default(),
            fill_visible_only: default_fill_visible_only(),
            stencil: None,
            stencil_look: StencilLook::default(),
            smudge_strength: default_smudge_strength(),
            clone_source: None,
            clone_aligned: true,
            clone_from: CloneFrom::Composite,
            path: Default::default(),
        };
        t.values.insert(Channel::BaseColor, [0.8, 0.8, 0.8]);
        t.values.insert(Channel::Roughness, [0.3; 3]);
        t
    }
}

/// What is selected in the layer stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StackTarget {
    #[default]
    Content,
    /// The layer's mask (painting adds/uses a paint effect).
    Mask,
    /// A specific effect.
    Effect(LayerId),
}

/// Work left in the viewport after a stroke committed or was cancelled.
pub(crate) struct DisplayStrokeEnd {
    pub id: u64,
    pub ts: usize,
    pub previous_revision: u64,
    pub revision: u64,
    pub tiles: BTreeSet<(u32, u32)>,
}

pub(crate) struct ActiveStroke {
    pub display_id: u64,
    pub display_tiles: BTreeSet<(u32, u32)>,
    /// Gray used for the most recently queued display of a mask stroke.
    pub displayed_mask_value: f32,
    pub ts: usize,
    /// None: the stroke edits the quick mask.
    pub target: Option<PaintTarget>,
    pub stroke: Stroke,
    pub image_alpha: Option<Arc<texturecraft_compose::ImageRes>>,
    pub library_alpha: Option<Arc<texturecraft_paint::alphas::AlphaImage>>,
    pub canvas: StrokeCanvas,
    /// Visible scene captured when a 3D stroke started. UV strokes use the full mesh.
    pub bvh: Arc<Bvh>,
    /// Camera visibility for 3D strokes (None for strokes in the 2D view).
    pub depth: Option<Arc<EyeDepth>>,
    /// 2D view strokes: where the stroke is between samples.
    pub uv: Option<crate::uv_stroke_state::UvStrokeState>,
    pub camera: OrbitCamera,
    pub viewport: Vec2,
    pub values: BTreeMap<Channel, [f32; 3]>,
    pub erase: bool,
    /// Projection tool: the stencil captured with the stroke's camera.
    pub stencil: Option<texturecraft_compose::Stencil>,
    /// History snapshot taken when the stroke began.
    pub before: Document,
    pub label: String,
    /// The commit's destination tiles, copied as the stroke reaches them.
    pub staging: texturecraft_paint::Staging,
    pub device: Option<Box<dyn crate::brush_device::DeviceStroke>>,
    pub device_uv_stamps: Vec<texturecraft_paint::UvStamp>,
    pub(crate) replay: Option<crate::stroke_replay::PaintPlacement>,
}

/// History labels that name a layer, a path number or a smart mask: the UI translates the
/// template and fills the value in (the control channel and `edit.history` keep the English label).
/// `{layer}` is a layer name, `{n}` a path number, `{name}` a smart mask name.
pub const HISTORY_TEMPLATES: &[&str] = &[
    "Paint on {layer}",
    "Erase on {layer}",
    "Clone on {layer}",
    "Smudge on {layer}",
    "Projection on {layer}",
    "Paint mask of {layer}",
    "Erase mask of {layer}",
    "Projection mask of {layer}",
    EDIT_PATH_TEMPLATE,
    APPLY_SMART_MASK_TEMPLATE,
];

pub(crate) const EDIT_PATH_TEMPLATE: &str = "Edit path {n}";
pub(crate) const APPLY_SMART_MASK_TEMPLATE: &str = "Apply smart mask {name}";
pub(crate) const QUICK_MASK_STROKE_LABEL: &str = "Quick mask stroke";

/// `template` with its one `{...}` placeholder replaced by `value`
pub fn history_label(template: &str, value: &str) -> String {
    match (template.find('{'), template.find('}')) {
        (Some(open), Some(close)) if open < close => format!("{}{}{}", &template[..open], value, &template[close + 1..]),
        _ => template.to_string(),
    }
}

/// The history template of a stroke by `tool`, on a layer or (`on_mask`) on its mask
pub fn stroke_template(tool: Tool, on_mask: bool) -> &'static str {
    match (tool, on_mask) {
        (Tool::Eraser, false) => "Erase on {layer}",
        (Tool::Eraser, true) => "Erase mask of {layer}",
        (Tool::Clone, _) => "Clone on {layer}",
        (Tool::Smudge, _) => "Smudge on {layer}",
        (Tool::Projection, false) => "Projection on {layer}",
        (Tool::Projection, true) => "Projection mask of {layer}",
        (_, false) => "Paint on {layer}",
        (_, true) => "Paint mask of {layer}",
    }
}

pub struct Session {
    pub brush_device: Option<crate::brush_device::BrushBackend>,
    pub doc: Document,
    pub mesh: Arc<Mesh>,
    /// Unique per loaded mesh and UV layout (GPU caches compare it).
    pub mesh_id: u64,
    /// Unique per open project: a mesh reimport or UV layout change keeps it, and opening, closing
    /// or creating a project replaces it.
    pub project_id: u64,
    /// The mesh with its own UVs (`mesh` maps UV-tiled sets into their atlas).
    pub(crate) source: Arc<Mesh>,
    pub bvh: Arc<Bvh>,
    /// Original mesh file (saved inside the project); None for built-in meshes.
    pub mesh_source: Option<(String, Vec<u8>)>,
    /// High-definition meshes and cage for baking (saved inside the project).
    pub bake_meshes: crate::BakeMeshes,
    pub caches: Vec<SetCache>,
    pub(crate) undo: Vec<(String, Document)>,
    pub(crate) redo: Vec<(String, Document)>,
    /// Set by [`crate::execute`] from a command's `merge` param: its edits continue the previous
    /// undo step with the same label (a slider drag is one step however many values it applies).
    pub(crate) merge_edits: bool,
    pub tool: ToolState,
    pub active_set: usize,
    /// Last material composited as active and the previous one whose bands may still be warm.
    cache_active: usize,
    cache_previous: Option<usize>,
    /// Selected layer per texture set.
    pub selected: BTreeMap<usize, LayerId>,
    /// Layers selected together with the selected one (Shift+click, Ctrl+click), per texture set.
    /// Counts only while the selected layer is the one they were picked with (see [`Session::selected_layers`]).
    pub(crate) multi: BTreeMap<usize, crate::layer_selection::MultiSelection>,
    pub target: StackTarget,
    pub camera: OrbitCamera,
    pub(crate) selected_camera_name: Option<String>,
    pub view: ViewMode,
    /// The image resource [`ViewMode::Image`] shows
    pub view_image: Option<String>,
    /// The layer shown alone in the viewport (not saved, not in the history; see [`crate::solo`]).
    pub solo: Option<crate::Solo>,
    /// The live bake preview while the app bakes (see [`crate::bake_preview`]).
    pub bake_preview: Option<crate::bake_preview::BakePreview>,
    /// Viewport size (pixels) used by scripted strokes.
    pub viewport: Vec2,
    pub(crate) stroke: Option<ActiveStroke>,
    pub(crate) display_stroke_end: Option<DisplayStrokeEnd>,
    /// Layer clipboard: a document snapshot and the copied layer (or effect).
    pub clipboard: Option<(Document, LayerId)>,
    /// Decoded image resources (rebuilt from `doc.resources` by `Session::sync_images`).
    pub images: texturecraft_compose::ImageStore,
    /// Temporary texel mask limiting paint strokes (not saved, not in the history).
    pub quick_mask: crate::quick_mask::QuickMask,
    /// A path sublayer is being repainted from its paths (the only way to paint into one).
    pub(crate) painting_paths: bool,
    /// Pixels of the paths below the one being edited, so live edits skip repainting them.
    pub(crate) path_cache: Option<crate::path_tool::PathCache>,
    /// A new path's live preview: the path becomes an undo step only when it is applied
    pub(crate) path_draft: Option<crate::path_draft::PathDraft>,
    /// Point changes of the path being edited: (earlier states for undo, undone states for redo)
    pub(crate) path_steps: (Vec<crate::path_tool::PathState>, Vec<crate::path_tool::PathState>),
    pub path: Option<String>,
    /// False in the empty state (the app at start, or after File > Close): the document is a
    /// tiny placeholder that panels do not show. Project-independent commands remain available
    /// (see [`crate::commands::check_enabled`]).
    pub project_open: bool,
    pub modified: bool,
    /// Bumps on every document change.
    pub revision: u64,
    /// Messages for the UI (status bar / toasts).
    pub log: crate::log::SessionLog,
    /// Changes when the session and its log are replaced, including mesh reimport.
    pub log_id: u64,
    /// Reuses the visible geometry and camera depth across 3D strokes.
    pub(crate) stroke_visibility: Option<crate::painting::StrokeVisibility>,
    /// The polygon fill box being dragged
    pub(crate) polygon_fill_drag: Option<crate::polygon_fill_drag::PolygonFillDrag>,
    /// The user's export presets (kept when another project opens).
    pub export_presets: crate::preset_cmds::UserPresets,
    /// The user's brush presets (kept when another project opens).
    pub brush_presets: crate::brush_preset_cmds::UserBrushes,
    /// The user's project templates (kept when another project opens).
    pub templates: crate::template_cmds::UserTemplates,
    /// Backing files in the native template library, keyed by ASCII case-insensitive name.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) template_files: BTreeMap<String, std::path::PathBuf>,
    /// The texture set that composited last (another one starting is when cooks held beyond their budget go).
    pub(crate) last_composited: Option<usize>,
}

impl Session {
    /// A session for `project` on `mesh` (its sets must reference mesh materials), with UV-tiled sets' UVs mapped into their atlas ([`Session::source_mesh`])
    pub fn new(doc: Document, mesh: Mesh, mesh_source: Option<(String, Vec<u8>)>) -> Result<Session> {
        mesh.validate().map_err(|error| EngineError::Other(error.to_string()))?;

        let source = Arc::new(mesh);
        let mesh = tiled_mesh(&source, &doc.project);
        let bvh = Bvh::new(&mesh);
        let mut camera = OrbitCamera { yaw: 0.6, pitch: 0.25, ..OrbitCamera::default() };
        let (bounds_min, bounds_max) = mesh.bounds();
        camera.frame(bounds_min, bounds_max);
        let mut lookup_budget = LOOKUP_BUDGET;
        let caches = doc
            .project
            .texture_sets
            .iter()
            .enumerate()
            .map(|(ts, texture_set)| {
                let cache = SetCache::new(&mesh, texture_set, if ts == 0 { usize::MAX } else { lookup_budget });
                lookup_budget = lookup_budget.saturating_sub(cache.lookup_bytes());
                cache
            })
            .collect();
        let selected = doc.project.texture_sets.iter().enumerate().filter_map(|(index, texture_set)| texture_set.stack.last().map(|id| (index, *id))).collect();

        Ok(Session {
            brush_device: None,
            doc,
            mesh_id: next_mesh_id(),
            project_id: next_mesh_id(),
            source,
            mesh,
            bvh: Arc::new(bvh),
            mesh_source,
            bake_meshes: Default::default(),
            caches,
            undo: Vec::new(),
            redo: Vec::new(),
            merge_edits: false,
            tool: ToolState::default(),
            active_set: 0,
            cache_active: 0,
            cache_previous: None,
            selected,
            multi: BTreeMap::new(),
            target: StackTarget::Content,
            camera,
            selected_camera_name: None,
            view: ViewMode::Material,
            view_image: None,
            solo: None,
            bake_preview: None,
            viewport: Vec2::new(1280.0, 800.0),
            stroke: None,
            display_stroke_end: None,
            clipboard: None,
            images: Default::default(),
            quick_mask: Default::default(),
            painting_paths: false,
            path_cache: None,
            path_draft: None,
            path_steps: Default::default(),
            path: None,
            project_open: true,
            modified: false,
            revision: 1,
            log: crate::log::SessionLog::new(),
            log_id: next_mesh_id(),
            stroke_visibility: None,
            polygon_fill_drag: None,
            export_presets: Default::default(),
            brush_presets: Default::default(),
            templates: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            template_files: Default::default(),
            last_composited: None,
        })
    }

    /// The empty state: no project is open. Holds a tiny placeholder document that the UI never
    /// shows; project-dependent commands are refused.
    pub fn closed() -> Result<Session> {
        // The hidden placeholder uses the minimum supported texture resolution
        let mut session = Session::new_project("", texturecraft_geom::primitives::plane(1.0, 1), "builtin:plane", None, texturecraft_core::MIN_RESOLUTION)?;
        session.project_open = false;

        Ok(session)
    }

    /// Replace this session with `next` (another project), keeping the viewport size, the
    /// user's brush presets, export presets and project templates and the tool settings.
    pub fn switch_to(&mut self, mut next: Session) {
        // Invalidate caches even when both projects were loaded with the same revision
        next.revision = self.revision.wrapping_add(1);

        next.viewport = self.viewport;
        next.export_presets = std::mem::take(&mut self.export_presets);
        next.brush_presets = std::mem::take(&mut self.brush_presets);
        next.templates = std::mem::take(&mut self.templates);
        #[cfg(not(target_arch = "wasm32"))]
        {
            next.template_files = std::mem::take(&mut self.template_files);
        }

        crate::tool_settings::ToolSettings::of(&self.tool).apply(&mut next.tool);
        *self = next;
    }

    /// A new project: one texture set per mesh material, each with an empty paint layer. Sets
    /// whose UVs leave tile 1001 paint UV tiles (see [`Session::new_project_tiled`]).
    pub fn new_project(name: &str, mesh: Mesh, mesh_ref: &str, mesh_source: Option<(String, Vec<u8>)>, resolution: u32) -> Result<Session> {
        Session::new_project_tiled(name, mesh, mesh_ref, mesh_source, resolution, None)
    }

    /// [`Session::new_project`] choosing the UV tile (UDIM) mode: `Some(true)` for every
    /// set, `Some(false)` for none, `None` for the sets whose UVs leave tile 1001.
    pub fn new_project_tiled(
        name: &str,
        mesh: Mesh,
        mesh_ref: &str,
        mesh_source: Option<(String, Vec<u8>)>,
        resolution: u32,
        uv_tiles: Option<bool>,
    ) -> Result<Session> {
        let res = check_resolution(resolution)?;
        let mut project = Project::new(name, mesh_ref);
        let mats = if mesh.materials.is_empty() { vec!["Default".to_string()] } else { mesh.materials.clone() };
        let mut log = crate::log::SessionLog::new();
        for (i, m) in mats.iter().enumerate() {
            let mut ts = TextureSet::new(m.clone(), i, res);
            ts.uv_tiles = detect_uv_tiles(&mesh, i, uv_tiles, &mut log).map_err(|e| EngineError::Other(format!("texture set \"{m}\": {e}")))?;
            check_size(&ts)?;
            let id = project.alloc_id();
            let layer = Layer::new(id, "Layer 1", LayerKind::Paint, &ts.channel_list());
            project.layers.insert(id, layer);
            ts.stack.push(id);
            project.texture_sets.push(ts);
        }
        let mut s = Session::new(
            Document {
                bake_paint: BTreeMap::new(),
                project,
                pixels: PixelStore::default(),
                stroke_replays: Default::default(),
                baked: BTreeMap::new(),
                resources: BTreeMap::new(),
                graphs: Default::default(),
                strokes: BTreeMap::new(),
            },
            mesh,
            mesh_source,
        )?;
        s.log.append(log);
        Ok(s)
    }

    /// The mesh with its own UVs (UV-tiled sets' UVs outside their atlas), for mesh export.
    pub fn source_mesh(&self) -> Mesh {
        (*self.source).clone()
    }

    pub fn project(&self) -> &Project {
        &self.doc.project
    }

    pub fn set_count(&self) -> usize {
        self.doc.project.texture_sets.len()
    }

    /// The active texture set index, clamped.
    pub fn ts(&self) -> usize {
        self.active_set.min(self.set_count().saturating_sub(1))
    }

    pub fn selected_layer(&self) -> Option<LayerId> {
        let ts = self.ts();
        self.selected.get(&ts).copied().filter(|id| self.doc.project.locate(*id).is_some_and(|(t, _, _)| t == ts))
    }

    // -----------------------------------------------------------------------------------------
    // History

    /// Run an undoable edit. On error the document is left untouched.
    pub fn edit<T>(&mut self, label: &str, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
        self.edit_merging(label, self.merge_edits, f)
    }

    /// [`Session::edit`] that, with `merge`, continues the previous undo step when it has the same label (a viewport drag is one step however many updates it makes).
    pub fn edit_merging<T>(&mut self, label: &str, merge: bool, change: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
        self.edit_if_changed(label, merge, |s| change(s).map(|value| (value, true)))
    }

    /// A conditional edit shares the usual rollback and history snapshot with layer reorders.
    pub(crate) fn edit_if_changed<T>(&mut self, label: &str, merge: bool, change: impl FnOnce(&mut Session) -> Result<(T, bool)>) -> Result<T> {
        // A box fill under way becomes its own step first, so this edit isn't written over or folded into it
        self.end_polygon_fill_box();

        let before = self.doc.clone();
        let result = change(self).and_then(|(value, changed)| {
            if changed && let Some(id) = self.doc.project.instance_cycle() {
                return Err(EngineError::Other(format!("instance layer {} would draw itself", id.0)));
            }

            if changed && let Some(error) = crate::lock_cmds::locked_change(&before, &self.doc) {
                return Err(error);
            }

            Ok((value, changed))
        });
        match result {
            Ok((value, changed)) => {
                if changed {
                    self.record(label, before, merge);
                }

                Ok(value)
            }
            Err(error) => {
                self.doc = before;

                // A failed repaint may have cached tiles from the discarded document
                self.path_cache = None;
                Err(error)
            }
        }
    }

    pub(crate) fn push_history(&mut self, label: &str, before: Document) {
        self.record(label, before, false);
    }

    /// A change from `before`: a new undo step `label`, or with `merge` part of the previous one when it has the same label
    fn record(&mut self, label: &str, before: Document, merge: bool) {
        self.invalidate_changed(&before);
        self.finish_record(label, before, merge);
    }

    pub(crate) fn record_resize(&mut self, before: Document, set: usize, mesh: Option<Arc<Mesh>>) {
        self.invalidate_changed_except(&before, Some(set), mesh);
        self.finish_record("Texture set size", before, false);
    }

    fn finish_record(&mut self, label: &str, before: Document, merge: bool) {
        if self.doc.stroke_replays.discard_stale(&self.doc.project, &self.doc.pixels, &self.mesh) {
            self.note_raster_paint();
        }
        self.push_step(label, before, merge);
    }

    pub(crate) fn note_raster_paint(&mut self) {
        self.log.warn("Paint", "This edit uses raster paint. Affected paint will be resampled when the texture resolution changes.");
    }

    /// [`Session::record`] for a change whose tiles the caller already marked for recompositing
    pub(crate) fn push_step(&mut self, label: &str, before: Document, merge: bool) {
        // Another change during a new path's preview makes the path its own step first
        if !self.painting_paths
            && let Some(draft) = self.path_draft.take()
        {
            self.undo.push((crate::path_tool::NEW_PATH_LABEL.to_string(), draft.base));
        }

        // Point changes of an edited path undo only until another step lands
        if !self.painting_paths {
            self.path_steps = Default::default();
        }

        self.modified = true;
        self.revision = self.revision.wrapping_add(1);
        if !(merge && self.undo.last().is_some_and(|(l, _)| l == label)) {
            self.undo.push((label.to_string(), before));
        }
        let over = self.undo.len().saturating_sub(MAX_HISTORY);
        self.undo.drain(..over);
        self.redo.clear();
    }

    /// A change from `other` to the current document that is not an undo step (a path preview)
    pub(crate) fn changed_without_history(&mut self, other: &Document) {
        self.modified = true;
        self.revision = self.revision.wrapping_add(1);
        self.invalidate_changed(other);
    }

    /// Mark dirty every texture set whose composite differs between the previous and current
    /// document (after an edit, undo or redo).
    pub(crate) fn invalidate_changed(&mut self, previous_document: &Document) {
        self.invalidate_changed_except(previous_document, None, None);
    }

    fn invalidate_changed_except(&mut self, previous_document: &Document, prepared_set: Option<usize>, mesh: Option<Arc<Mesh>>) {
        self.follow_uv_layouts(&previous_document.project, prepared_set, mesh);
        self.sync_caches();

        // Removed path effects must release their cached painted tiles
        if self.path_cache.as_ref().is_some_and(|cache| self.paths(cache.eid).is_none()) {
            self.path_cache = None;
        }

        // Removed or replaced resources may change any set's rendered content
        let removed_or_replaced_resource =
            previous_document.resources.iter().any(|(name, previous)| !self.doc.resources.get(name).is_some_and(|current| Arc::ptr_eq(previous, current)));
        let added_resource_names: BTreeSet<_> = self.doc.resources.keys().filter(|name| !previous_document.resources.contains_key(*name)).collect();
        let uses_added_resource = |texture_set_index: usize| {
            use texturecraft_core::{EffectKind, Fill, GraphInput, Source};

            let uses_source = |source: &Source| source.images().into_iter().any(|name| added_resource_names.iter().any(|added| added.as_str() == name));
            let uses_fill =
                |fill: &Fill| fill.sources.values().any(uses_source) || fill.link.as_ref().is_some_and(|link| added_resource_names.contains(&link.package));
            let project = &self.doc.project;
            !added_resource_names.is_empty()
                && project.texture_sets.get(texture_set_index).is_some_and(|set| {
                    project.render_tree(&set.stack).iter().filter_map(|id| project.layers.get(id)).any(|layer| {
                        matches!(&layer.kind, LayerKind::Fill(fill) if uses_fill(fill))
                            || layer
                                .graphs()
                                .into_iter()
                                .flat_map(|graph| graph.graphs())
                                .flat_map(|graph| graph.inputs.values())
                                .any(|input| matches!(input, GraphInput::Image(name) if added_resource_names.contains(name)))
                            || layer.mask.iter().flat_map(|mask| &mask.effects).chain(&layer.content_effects).any(|effect| match &effect.kind {
                                EffectKind::Fill { source, .. } => uses_source(source),
                                EffectKind::Material(fill) => uses_fill(fill),
                                EffectKind::Generator(generator) => generator.inputs.values().any(uses_source),
                                _ => false,
                            })
                    })
                })
        };
        let (empty_baked_maps, empty_images) = (BakedMaps::default(), Default::default());
        let stack_key = |document: &Document, texture_set_index: usize| {
            let stack = document.project.texture_sets.get(texture_set_index).map(|set| set.stack.as_slice()).unwrap_or_default();
            StackKey::new(
                &document.project,
                &document.pixels,
                document.baked.get(&texture_set_index).unwrap_or(&empty_baked_maps),
                &empty_images,
                texture_set_index,
                stack,
            )
        };
        for texture_set_index in 0..self.caches.len() {
            if Some(texture_set_index) == prepared_set {
                continue;
            }
            let before = stack_key(previous_document, texture_set_index);
            let after = stack_key(&self.doc, texture_set_index);
            let resource_changed = removed_or_replaced_resource || uses_added_resource(texture_set_index);
            let Some(cache) = self.caches.get_mut(texture_set_index) else {
                continue;
            };

            if resource_changed {
                cache.invalidate_all();
                continue;
            }

            if before == after {
                continue;
            }

            match before.as_ref().zip(after.as_ref()).and_then(|(before, after)| before.changed_paint(after)) {
                Some(changes) => {
                    cache.material_revision = cache.material_revision.wrapping_add(1);
                    for (target, changed) in changes {
                        let tiles: Vec<_> =
                            changed.into_iter().filter(|&(x, y)| cache.covered_tiles.binary_search_by_key(&(y, x), |&(tx, ty)| (ty, tx)).is_ok()).collect();
                        cache.queue_painted(&self.doc.project, texture_set_index, target, &tiles, |c| &mut c.dirty);
                    }
                }
                None => cache.invalidate_all(),
            }
        }
    }

    /// After the UV tile layout of a texture set changed (UV tiles on or off, tile
    /// resolutions) between `before` and the current project, map the mesh's UVs into the new
    /// atlas and rebuild those sets' caches.
    fn follow_uv_layouts(&mut self, before: &Project, prepared_set: Option<usize>, mesh: Option<Arc<Mesh>>) {
        let layout = |p: &Project, ts: usize| p.texture_sets.get(ts).map(uv_layout);
        let changed: Vec<usize> = (0..self.doc.project.texture_sets.len()).filter(|ts| layout(before, *ts) != layout(&self.doc.project, *ts)).collect();
        if changed.is_empty() && before.texture_sets.len() == self.doc.project.texture_sets.len() {
            return;
        }
        self.mesh = mesh.unwrap_or_else(|| tiled_mesh(&self.source, &self.doc.project));
        self.mesh_id = next_mesh_id();
        self.stroke_visibility = None;
        for ts in changed {
            if Some(ts) == prepared_set {
                continue;
            }
            let lookup_budget = self.lookup_budget_for(ts);
            if let (Some(c), Some(set)) = (self.caches.get_mut(ts), self.doc.project.texture_sets.get(ts)) {
                *c = SetCache::new(&self.mesh, set, lookup_budget);
            }
        }
    }

    /// Mark the document changed and every texture set dirty.
    pub fn touched(&mut self) {
        self.modified = true;
        self.revision = self.revision.wrapping_add(1);
        self.sync_caches();
        for c in &mut self.caches {
            c.invalidate_all();
        }
    }

    /// Make caches agree with the project's texture sets (resolution or channel changes).
    pub fn sync_caches(&mut self) {
        let sets = self.doc.project.texture_sets.clone();
        self.caches.truncate(sets.len());
        for (i, ts) in sets.iter().enumerate() {
            match self.caches.get_mut(i) {
                Some(c) if c.size == ts.size() => {
                    c.invalidate_paint_margin();
                    c.finished_dab = None;
                    let (want, scales) = (ts.channel_list(), ts.block_scales());
                    if c.composite.channels.keys().copied().collect::<Vec<_>>() != want || c.composite.block_scales != scales {
                        c.composite = Composite { block_scales: scales, ..Composite::with_size(&want, c.size.0, c.size.1) };
                        c.upload.extend(texturecraft_compose::all_tiles(c.size.0, c.size.1));
                        c.invalidate_all();
                    }
                }
                _ => {
                    let cache = SetCache::new(&self.mesh, ts, self.lookup_budget_for(i));
                    if i < self.caches.len() {
                        self.caches[i] = cache;
                    } else {
                        self.caches.push(cache);
                    }
                }
            }
        }
    }

    /// Whether a point edit, an uncommitted path preview, or a document edit can be undone
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.path_draft.is_some() || self.path_step_pending(true)
    }

    /// Whether a point edit or document edit can be restored
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty() || self.path_step_pending(false)
    }

    /// Label of the next undo action, giving an active path's point edits and preview priority
    pub fn undo_label(&self) -> Option<&str> {
        if self.path_draft.is_some() || self.path_step_pending(true) {
            return Some(crate::path_tool::NEW_PATH_LABEL);
        }
        self.undo.last().map(|(l, _)| l.as_str())
    }

    /// Label of the next redo action, giving an active path's point history priority
    pub fn redo_label(&self) -> Option<&str> {
        if self.path_step_pending(false) {
            return Some(crate::path_tool::NEW_PATH_LABEL);
        }
        self.redo.last().map(|(l, _)| l.as_str())
    }

    /// The undone steps, the next one to redo first.
    pub fn redo_labels(&self) -> Vec<String> {
        self.redo.iter().rev().map(|(l, _)| l.clone()).collect()
    }
    pub fn history_labels(&self) -> Vec<String> {
        self.undo.iter().map(|(l, _)| l.clone()).collect()
    }

    /// Undo a point edit, cancel an uncommitted path, or restore a document snapshot, in that order
    pub fn undo(&mut self) -> Result<String> {
        // A stroke, a box fill or an edited path under way would otherwise write over the restored document
        self.cancel_stroke();
        self.end_polygon_fill_box();

        // While a path is edited, undo steps back through its point changes first
        if self.step_path(true)? {
            return Ok(crate::path_tool::NEW_PATH_LABEL.to_string());
        }

        // A path being placed isn't a step yet: undo removes it
        if self.path_draft.is_some() {
            self.stop_path_edit();
            return Ok(crate::path_tool::NEW_PATH_LABEL.to_string());
        }

        self.stop_path_edit();

        let (label, restored) = self.undo.pop().ok_or_else(|| EngineError::Other("nothing to undo".into()))?;
        let current = std::mem::replace(&mut self.doc, restored);
        self.after_history(&current);
        self.redo.push((label.clone(), current));
        self.sync_images();
        Ok(label)
    }

    /// Restore a point edit and its pixels before considering document redo
    pub fn redo(&mut self) -> Result<String> {
        // A stroke, a box fill or an edited path under way would otherwise write over the restored document
        self.cancel_stroke();
        self.end_polygon_fill_box();

        if self.step_path(false)? {
            return Ok(crate::path_tool::NEW_PATH_LABEL.to_string());
        }

        self.stop_path_edit();

        let (label, restored) = self.redo.pop().ok_or_else(|| EngineError::Other("nothing to redo".into()))?;
        let current = std::mem::replace(&mut self.doc, restored);
        self.after_history(&current);
        self.undo.push((label.clone(), current));
        self.sync_images();
        Ok(label)
    }

    fn after_history(&mut self, other: &Document) {
        // Pending operations may still hold layer IDs from the discarded snapshot
        self.doc.project.next_id = self.doc.project.next_id.max(other.project.next_id);
        self.modified = true;

        // Layers picked together belong to the stack they were picked in
        self.multi.clear();
        self.revision = self.revision.wrapping_add(1);
        self.invalidate_changed(other);

        // An undone add or duplicate takes the selected effect away
        if let StackTarget::Effect(e) = self.target
            && self.doc.project.effect(e).is_none()
        {
            let p = &other.project;
            let content = p.effect(e).and_then(|(owner, _)| p.layers.get(&owner)).is_some_and(|l| l.content_effects.iter().any(|x| x.id == e));
            self.target = if content { StackTarget::Content } else { StackTarget::Mask };
        }
        let has_mask = self.selected_layer().and_then(|lid| self.doc.project.layer(self.doc.project.mask_owner(lid)).ok()).is_some_and(|l| l.mask.is_some());
        if self.target == StackTarget::Mask && !has_mask {
            self.target = StackTarget::Content;
        }
    }

    // -----------------------------------------------------------------------------------------
    // Compositing

    /// Recomposite dirty tiles of every texture set (with the live stroke preview). Returns
    /// whether anything changed.
    pub fn update(&mut self) -> bool {
        self.update_for(f64::INFINITY, || 0.0)
    }

    /// [`Session::update`] within about `budget_ms` of `clock` (milliseconds): tiles go in
    /// chunks sized from the time the previous chunk took, and what is left stays dirty for the
    /// next call (at least one chunk runs). The UI calls this every frame so a full recomposite
    /// of a large texture set spreads over frames instead of freezing. The active texture set
    /// goes first; settling tiles (see [`SetCache::settle`]) get at most [`SETTLE_BUDGET_MS`]
    /// of a finite budget, after the dirty ones.
    pub fn update_for(&mut self, budget_ms: f64, clock: impl Fn() -> f64) -> bool {
        self.update_for_view(budget_ms, clock, crate::TileDemand::default())
    }

    /// Budgeted display update with the same tile demand used by the viewport uploader.
    pub fn update_for_view(&mut self, budget_ms: f64, clock: impl Fn() -> f64, demand: crate::TileDemand) -> bool {
        self.update_for_view_where(budget_ms, clock, demand, |_| true)
    }

    /// Maintain CPU display caches for the sets the viewport is composing on the CPU.
    pub fn update_for_view_where(&mut self, budget_ms: f64, clock: impl Fn() -> f64, demand: crate::TileDemand, include: impl Fn(usize) -> bool) -> bool {
        let active = self.ts();
        if active != self.cache_active {
            // A third material replaces the previously parked one, even when the
            // interactive slice ends before the compositor visits that texture set.
            if let Some(displaced) = self.cache_previous.filter(|&ts| ts != active)
                && let Some(cache) = self.caches.get_mut(displaced)
            {
                cache.below.clear();
            }
            self.cache_previous = Some(self.cache_active);
            if let Some(cache) = self.caches.get_mut(self.cache_active) {
                cache.below.finish_pass();
            }
            self.cache_active = active;
        }
        let order: Vec<_> = std::iter::once(active).chain((0..self.caches.len()).filter(|&ts| ts != active)).filter(|&ts| include(ts)).collect();
        let changed = self.update_tiles(budget_ms, clock, active, demand, &order);
        self.trim_set_caches(None);
        if let Some(previous) = self.cache_previous.filter(|&ts| self.caches.get(ts).is_some_and(|c| !c.below.is_empty())) {
            // Retaining an idle material spends only the remainder of the normal band
            // budget. The active material always gets first use of that budget.
            let allowance = self.caches.get(active).map_or(0, |c| c.below.limit());
            // Instances in other sets can have live bands of their own.
            let spare = self
                .caches
                .iter()
                .enumerate()
                .filter(|(ts, _)| *ts != previous)
                .fold(allowance, |remaining, (_, cache)| remaining.saturating_sub(cache.below.bytes()));
            if let Some(cache) = self.caches.get_mut(previous)
                && cache.below.bytes() > spare
            {
                cache.below.clear();
            }
        }
        changed
    }

    /// [`Session::update`] for the texture sets `sets` only, in that order (the others keep their dirty tiles): an
    /// export or a windowless render works through a project one set at a time.
    pub fn update_only(&mut self, sets: &[usize]) -> bool {
        let changed = self.update_tiles(f64::INFINITY, || 0.0, self.ts(), crate::TileDemand::default(), sets);
        self.trim_set_caches(None);
        changed
    }

    /// Keeps what the texture sets cache for recompositing (whole-set fields and the stack around the edited layer)
    /// within [`SET_CACHE_BUDGET`] for the whole document: beyond it, the sets that are neither active nor `working`
    /// let theirs go, the largest first. What is let go is computed again when the set next composites, so nothing
    /// changes but the time of that composite. Without this every set of a large project kept its own (up to 2 GiB of
    /// fields each), whether it was ever edited or not.
    fn trim_set_caches(&mut self, working: Option<usize>) {
        let active = self.ts();
        let held = |c: &SetCache| c.below.bytes().saturating_add(c.below.fields.bytes());
        let mut total: usize = self.caches.iter().map(held).sum();
        while total > SET_CACHE_BUDGET {
            let largest = self.caches.iter_mut().enumerate().filter(|(i, c)| *i != active && Some(*i) != working && held(c) > 0).max_by_key(|(_, c)| held(c));
            let Some((_, cache)) = largest else { break };
            total = total.saturating_sub(held(cache));
            cache.below.fields.clear();
            cache.below.clear();
        }
    }

    /// Keeps the texture sets' UV lookups within [`LOOKUP_BUDGET`]: beyond it, the sets that are neither active nor
    /// `working` let theirs go (built again, the same, when next needed).
    pub(crate) fn trim_lookups(&mut self, working: usize) {
        let active = self.ts();
        let mut total: usize = self.caches.iter().map(SetCache::lookup_bytes).sum();
        for (i, cache) in self.caches.iter_mut().enumerate() {
            if total <= LOOKUP_BUDGET {
                break;
            }
            if i != working && i != active {
                total = total.saturating_sub(cache.lookup_bytes());
                cache.release_lookup();
            }
        }
    }

    /// Retain the freshly rasterized lookup for the active set, and for other sets when it fits alongside the current lookups.
    pub(crate) fn lookup_budget_for(&self, working: usize) -> usize {
        if working == self.ts() {
            usize::MAX
        } else {
            let retained = self.caches.iter().enumerate().filter(|(ts, _)| *ts != working).map(|(_, cache)| cache.lookup_bytes()).sum();
            LOOKUP_BUDGET.saturating_sub(retained)
        }
    }

    /// Keeps the layer pixels and baked maps held of a project opened from its file within [`SET_DATA_BUDGET`]: beyond
    /// it, those of the sets that are neither active nor `working` and unchanged since the file was opened let go of
    /// what they read (read again, the same, when next needed).
    fn trim_set_data(&mut self, working: usize) {
        let active = self.ts();
        let pixels = &mut self.doc.pixels;
        let held = pixels.layers.values().chain(pixels.masks.values()).map(|i| i.bytes()).sum::<usize>()
            + self.doc.baked.values().flat_map(|b| b.maps.values()).map(|m| m.bytes()).sum::<usize>();
        if held <= SET_DATA_BUDGET {
            return;
        }
        let project = &self.doc.project;
        let other = |id: LayerId| {
            let owner = project.effect(id).map_or(id, |(owner, _)| owner);
            project.locate(owner).is_some_and(|(ts, _, _)| ts != working && ts != active)
        };
        for ((id, _), img) in pixels.layers.iter_mut() {
            if other(*id) {
                img.release();
            }
        }
        for (id, img) in pixels.masks.iter_mut() {
            if other(*id) {
                img.release();
            }
        }
        for (_, maps) in self.doc.baked.iter_mut().filter(|(ts, _)| **ts != working && **ts != active) {
            for map in maps.maps.values_mut() {
                if matches!(map.as_ref(), texturecraft_compose::Texels::Lazy(_)) && map.bytes() != 0 && Arc::strong_count(map) == 1 {
                    // The graph fingerprint can hold a weak reference to the map.
                    Arc::make_mut(map).release();
                }
            }
        }
    }

    fn update_tiles(&mut self, budget_ms: f64, clock: impl Fn() -> f64, active: usize, demand: crate::TileDemand, order: &[usize]) -> bool {
        let start = clock();
        let initial_chunk_size = if budget_ms.is_finite() { rayon::current_num_threads().clamp(1, CHUNK) } else { CHUNK };

        // The layer being edited: everything else is cached per tile
        let hot = self.stroke.as_ref().and_then(|stroke| stroke.target).map(|target| self.paint_owner(target)).or_else(|| self.selected_layer());
        let stroke_size = self.stroke.as_ref().and_then(|stroke| self.caches.get(stroke.ts)).map(|cache| cache.size);
        let preview = self.stroke.as_ref().and_then(|stroke| {
            Some(StrokePreview {
                target: stroke.target?,
                alpha: stroke.canvas.alpha.clone(),
                opacity: stroke.stroke.settings.opacity,
                erase: stroke.erase,
                values: stroke.values.clone(),
                mask_value: self.tool.mask_value,
                stencil: stroke.stencil.clone(),
                carried_mask: stroke.canvas.pickup.as_ref().and_then(|pickup| pickup.mask_values()).cloned(),
                carried: stroke.canvas.pickup.as_ref().map(|pickup| pickup.values.clone()).unwrap_or_default(),
            })
        });

        // A soloed layer's view of the project; a removed layer ends the solo
        if self.solo.is_some_and(|solo| self.doc.project.locate(solo.layer).is_none()) {
            self.set_solo(None);
        }
        let solo_view = if self.pending_tiles() > 0 { self.solo_view() } else { None };
        let mut changed = false;
        for &ts in order {
            // room for this set's caches before it composites
            if self.caches.get(ts).is_some_and(|c| !c.dirty.is_empty() || !c.settle.is_empty()) {
                self.trim_set_caches(Some(ts));
                // and for its cooks: those the project file keeps come back from it when a set needs them again
                // and for its images: those decoded from the document's resources are decoded again when read
                if self.last_composited.is_some_and(|last| last != ts) {
                    self.doc.graphs.release_reloadable(COOK_BUDGET);
                    self.images.release(IMAGE_BUDGET);
                    self.trim_lookups(ts);
                    self.trim_set_data(ts);
                }
                self.last_composited = Some(ts);
            }
            let Some(cache) = self.caches.get_mut(ts) else { continue };
            if ts == active
                && budget_ms.is_finite()
                && let Some(brush) = demand.brush
                && cache.settle.remove(&brush)
            {
                cache.dirty.insert(brush);
            }
            if Some(ts) == self.cache_previous && cache.dirty.is_empty() && cache.settle.is_empty() && !cache.below.is_empty() {
                continue;
            }
            cache.below.set_hot(hot);
            cache.below.pass = cache.dirty.len() + cache.settle.len();

            // Instances in sets of the stroke's size show the live stroke too
            let preview = preview.as_ref().filter(|_| Some(cache.size) == stroke_size);

            // Reuse the last affordable chunk size for heavy stacks
            let mut chunk = if budget_ms.is_finite() && cache.chunk > 0 { cache.chunk } else { initial_chunk_size };
            loop {
                if cache.dirty.is_empty() {
                    if cache.settle.is_empty() || clock() - start >= budget_ms.min(SETTLE_BUDGET_MS) {
                        break;
                    }

                    // Small chunks: the settle slice of the budget is short
                    cache.dirty.extend(next_chunk(&mut cache.settle, chunk.min(initial_chunk_size)));
                }
                let chunk_started = clock();
                let mut tiles = if ts == active && budget_ms.is_finite() { demand.take(&mut cache.dirty, chunk) } else { Vec::new() };
                if tiles.is_empty() {
                    tiles = next_chunk(&mut cache.dirty, chunk);
                } else if tiles.len() < chunk {
                    tiles.extend(next_chunk(&mut cache.dirty, chunk - tiles.len()));
                }
                for tile in &tiles {
                    cache.settle.remove(tile);
                }

                let empty = BakedMaps::default();
                let gbuf = cache.gbuf().clone();
                let inputs = texturecraft_compose::Inputs {
                    project: solo_view.as_ref().unwrap_or(&self.doc.project),
                    ts,
                    mesh: &self.mesh,
                    gbuf: &gbuf,
                    pixels: &self.doc.pixels,
                    baked: self.doc.baked.get(&ts).unwrap_or(&empty),
                    block_scales: cache.composite.block_scales.clone(),
                    preview,
                    images: &self.images,
                    graphs: &self.doc.graphs,
                    bounds: self.mesh.bounds(),
                };

                // Look ahead so sliced updates filter a run of tiles once
                cache.below.ahead = if budget_ms.is_finite() { tiles_ahead(&cache.dirty, &tiles) } else { Vec::new() };
                texturecraft_compose::composite_tiles_cached(&inputs, &mut cache.composite, &tiles, Some(&mut cache.below));

                // The composited tiles and the neighbours their padding reached change
                let padded = cache.dilation.apply_to_tiles(&mut cache.composite, &tiles);
                cache.upload.extend(tiles.iter().copied().chain(padded));
                cache.revision = cache.revision.wrapping_add(1);
                changed = true;

                let now = clock();
                let remaining_ms = budget_ms - (now - start);

                // Estimate how many tiles fit in the remaining frame budget
                let ms_per_tile = (now - chunk_started) / tiles.len().max(1) as f64;
                if budget_ms.is_finite() && ms_per_tile > 0.0 {
                    // Never above one tile per thread: work can turn heavy from one frame to the next
                    cache.chunk = ((budget_ms / ms_per_tile) as usize).clamp(1, initial_chunk_size);
                }
                if remaining_ms <= 0.0 {
                    return changed;
                }

                let fitting_tiles = if ms_per_tile > 0.0 { (remaining_ms / ms_per_tile) as usize } else { CHUNK };
                if fitting_tiles == 0 {
                    return changed;
                }

                chunk = fitting_tiles.min(CHUNK);
            }
        }

        changed
    }

    /// Tiles still waiting to be composited, over every texture set (settling ones included).
    pub fn pending_tiles(&self) -> usize {
        self.caches.iter().map(|c| c.dirty.len() + c.settle.len()).sum()
    }

    /// The layer a stroke target belongs to (an effect's owner).
    pub(crate) fn paint_owner(&self, t: PaintTarget) -> LayerId {
        let (PaintTarget::Layer(id) | PaintTarget::Mask(id)) = t;
        self.doc.project.effect(id).map_or(id, |(owner, _)| owner)
    }

    /// Mark `tiles` of texture set `ts` dirty after painting `target`, and the same tiles of
    /// other sets of that size drawing it through an instance.
    pub(crate) fn mark_painted(&mut self, ts: usize, target: Option<PaintTarget>, tiles: impl IntoIterator<Item = (u32, u32)>, changed_document: bool) {
        self.mark_tiles(ts, target, tiles, false, changed_document);
    }

    /// [`Session::mark_painted`] for the tiles of a committed stroke: they settle at low
    /// priority (see [`SetCache::settle`]).
    pub(crate) fn mark_settle(&mut self, ts: usize, target: Option<PaintTarget>, tiles: impl IntoIterator<Item = (u32, u32)>) {
        self.mark_tiles(ts, target, tiles, true, true);
    }

    fn mark_tiles(&mut self, ts: usize, target: Option<PaintTarget>, tiles: impl IntoIterator<Item = (u32, u32)>, settle: bool, changed_document: bool) {
        let size = self.caches.get(ts).map(|c| c.size);
        let Some(target) = target else { return };
        let sets = self.doc.project.sets_drawing(self.paint_owner(target));
        let tiles: Vec<(u32, u32)> = tiles.into_iter().collect();
        for (i, c) in self.caches.iter_mut().enumerate() {
            if i != ts && !sets.contains(&i) {
                continue;
            }
            if Some(c.size) == size {
                if changed_document {
                    c.material_revision = c.material_revision.wrapping_add(1);
                }
                c.queue_painted(&self.doc.project, i, target, &tiles, |c| if settle { &mut c.settle } else { &mut c.dirty });
            } else if settle {
                // Resampled instances first see the stroke when its source pixels are committed.
                c.invalidate_all();
            }
        }
    }

    /// Drain tiles needing upload for texture set `ts`.
    pub fn take_uploads(&mut self, ts: usize) -> Vec<(u32, u32)> {
        let Some(cache) = self.caches.get_mut(ts) else { return Vec::new() };
        let tiles = cache.take_upload_batch(usize::MAX, crate::TileDemand::default());
        if let Some(dab) = cache.released_dab_in(&tiles) {
            dab.acknowledge_upload();
        }
        tiles
    }

    /// `f` on the compositor inputs of texture set `ts` (without a live stroke; None for a set
    /// not ready).
    pub(crate) fn with_inputs<R>(&self, ts: usize, f: impl FnOnce(&texturecraft_compose::Inputs) -> Option<R>) -> Option<R> {
        let cache = self.caches.get(ts)?;
        let empty = BakedMaps::default();
        f(&texturecraft_compose::Inputs {
            project: &self.doc.project,
            ts,
            mesh: &self.mesh,
            gbuf: cache.gbuf(),
            pixels: &self.doc.pixels,
            baked: self.doc.baked.get(&ts).unwrap_or(&empty),
            block_scales: cache.composite.block_scales.clone(),
            preview: None,
            images: &self.images,
            graphs: &self.doc.graphs,
            bounds: self.mesh.bounds(),
        })
    }

    /// Grayscale RGBA8 of layer `id`'s mask at `size²` for the mask view (None without a mask).
    pub fn mask_view_rgba(&self, id: LayerId, size: u32) -> Option<Vec<u8>> {
        let (ts, _, _) = self.doc.project.locate(id)?;
        let gray = self.with_inputs(ts, |inp| texturecraft_compose::mask_image(inp, id, size))?;
        Some(gray.iter().flat_map(|g| [*g, *g, *g, 255]).collect())
    }

    /// The image resource of the "Other..." view as `size` x `size` RGBA8 pixels over the UV square (stretched, bilinear,
    /// values as stored; HDR values clamp to 1). Returns None for a missing image, excessive
    /// resolution or allocation failure.
    pub fn image_view_rgba(&self, size: u32) -> Option<Vec<u8>> {
        if size > texturecraft_core::MAX_RESOLUTION {
            return None;
        }

        let image = self.images.get(self.view_image.as_deref()?)?;
        let (width, height) = (image.width.max(1) as usize, image.height.max(1) as usize);
        width.checked_mul(height)?;
        let size = size.max(1) as usize;
        let fallback = [0.0, 0.0, 0.0, 1.0];
        let texel = |x: usize, y: usize| image.rgba.get(y.min(height - 1) * width + x.min(width - 1)).copied().unwrap_or(fallback);
        let mut out = Vec::new();
        out.try_reserve_exact(size * size * 4).ok()?;

        for y in 0..size {
            let v = ((y as f32 + 0.5) / size as f32 * height as f32 - 0.5).max(0.0);
            let (y0, fy) = (v.floor() as usize, v.fract());
            for x in 0..size {
                let u = ((x as f32 + 0.5) / size as f32 * width as f32 - 0.5).max(0.0);
                let (x0, fx) = (u.floor() as usize, u.fract());
                let (a, b, c, d) = (texel(x0, y0), texel(x0 + 1, y0), texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
                for k in 0..3 {
                    let value = (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy;
                    out.push((value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                }
                out.push(255);
            }
        }
        Some(out)
    }

    /// Mask values at texture-set coordinates in request order; None without an enabled mask
    /// Uncovered and out-of-range texels read zero; neighbourhood filters also read surrounding texels
    pub fn mask_at(&self, id: LayerId, texels: &[(u32, u32)]) -> Option<Vec<f32>> {
        let (texture_set, _, _) = self.doc.project.locate(id)?;
        self.with_inputs(texture_set, |inputs| texturecraft_compose::mask_at(inputs, id, texels))
    }

    /// Histogram (`bins` buckets over 0..1) of the values entering effect `effect`: its layer's
    /// mask, or for a content effect the layer's `channel` (None for an unknown effect).
    pub fn effect_histogram(&self, effect: LayerId, channel: Channel, bins: usize) -> Option<Vec<u32>> {
        let (owner, _) = self.doc.project.effect(effect)?;
        let (ts, _, _) = self.doc.project.locate(owner)?;
        self.with_inputs(ts, |inp| texturecraft_compose::effect_histogram(inp, effect, channel, bins))
    }

    pub fn baked(&self, ts: usize) -> Option<&BakedMaps> {
        self.doc.baked.get(&ts)
    }

    /// Bytes held by painted layer pixels, the composites of every texture set and their
    /// hot-layer caches, UV geometry and display padding.
    pub fn memory(&self) -> Memory {
        let composites = self.caches.iter().flat_map(|c| c.composite.channels.values()).map(texturecraft_texel::TileImage::bytes).sum();
        let geometry = self.caches.iter().map(|c| c.lookup_bytes() + c.covered_tiles.capacity() * size_of::<(u32, u32)>()).sum();
        Memory { pixels: self.doc.pixels.bytes(), composites, cache: self.caches.iter().map(|c| c.below.bytes()).sum(), geometry }
    }
}

/// Memory use reported by [`Session::memory`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub pixels: usize,
    pub composites: usize,
    pub cache: usize,
    pub geometry: usize,
}

impl Memory {
    pub fn total(self) -> usize {
        self.pixels.saturating_add(self.composites).saturating_add(self.cache).saturating_add(self.geometry)
    }
}

/// Most tiles per composite chunk (a 1024² region).
const CHUNK: usize = 256;
/// Bytes the texture sets of a document may hold together in recompositing caches (see `Session::trim_set_caches`);
/// one set's whole-set fields alone may reach this ([`texturecraft_compose::FIELD_BUDGET`]).
pub const SET_CACHE_BUDGET: usize = 2 << 30;
/// Bytes of layer pixels and baked maps read from a project's file that a document holds before those of sets not
/// compositing are let go when another set starts compositing (`Session::trim_set_data`).
pub const SET_DATA_BUDGET: usize = 1 << 30;
/// Bytes of UV lookups and padding maps the texture sets hold together before those of sets not compositing are let
/// go when another set starts compositing (`Session::trim_lookups`).
pub const LOOKUP_BUDGET: usize = 512 << 20;
/// Bytes of decoded images a document holds before those that can be decoded again are let go when another texture
/// set starts compositing (`ImageStore::release`).
pub const IMAGE_BUDGET: usize = 512 << 20;
/// Bytes of graph cooks a document holds before those that can be read again are let go when another texture set
/// starts compositing (`GraphStore::release_reloadable`).
pub const COOK_BUDGET: usize = 512 << 20;
/// The most a budgeted update spends on settling tiles (what they show is already right).
pub const SETTLE_BUDGET_MS: f64 = 6.0;

/// Take up to `n` dirty tiles, block by block (16 × 16 tiles) so neighbourhood filters
/// evaluate compact regions.
fn next_chunk(dirty: &mut BTreeSet<(u32, u32)>, max_tiles: usize) -> Vec<(u32, u32)> {
    // A zero-sized request cannot select a pivot
    if max_tiles == 0 {
        return Vec::new();
    }

    if dirty.len() <= max_tiles {
        return std::mem::take(dirty).into_iter().collect();
    }

    let mut tiles: Vec<(u32, u32)> = dirty.iter().copied().collect();

    // Partition by block order so only the requested chunk needs sorting
    tiles.select_nth_unstable_by_key(max_tiles - 1, |&(x, y)| (y / texturecraft_compose::BLOCK, x / texturecraft_compose::BLOCK, y, x));
    tiles.truncate(max_tiles);
    tiles.sort_by_key(|&(x, y)| (y / texturecraft_compose::BLOCK, x / texturecraft_compose::BLOCK, y, x));
    for tile in &tiles {
        dirty.remove(tile);
    }

    tiles
}

/// Dirty tiles a neighbourhood filter runs over ahead of the chunk being composited.
const FILTER_AHEAD: usize = 64;

/// The dirty tiles composited after `chunk` in its 16 × 16 tile blocks, in the order [`next_chunk`] takes them, at most [`FILTER_AHEAD`].
fn tiles_ahead(dirty: &BTreeSet<(u32, u32)>, chunk: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let block = |&(x, y): &(u32, u32)| (y / texturecraft_compose::BLOCK, x / texturecraft_compose::BLOCK);
    let blocks: BTreeSet<(u32, u32)> = chunk.iter().map(block).collect();
    let mut ahead: Vec<(u32, u32)> = dirty.iter().copied().filter(|tile| blocks.contains(&block(tile))).collect();
    ahead.sort_by_key(|tile| (block(tile), tile.1, tile.0));
    ahead.truncate(FILTER_AHEAD);
    ahead
}

/// Valid texture set resolutions: powers of two from 128 to 8192.
pub fn check_resolution(r: u32) -> Result<u32> {
    if (128..=8192).contains(&r) && r.is_power_of_two() {
        Ok(r)
    } else {
        Err(EngineError::Other(format!("resolution {r} must be a power of two between 128 and 8192")))
    }
}

/// A texture set's texels over all its UV tiles must stay within what bakes and caches hold.
pub fn check_size(ts: &TextureSet) -> Result<()> {
    let (w, h) = ts.size();
    if u64::from(w) * u64::from(h) > texturecraft_bake::MAX_TEXELS {
        return Err(EngineError::Other(format!(
            "{} UV tiles at {} exceed {} texels: lower the resolution",
            ts.uv_tiles.len(),
            ts.resolution,
            texturecraft_bake::MAX_TEXELS
        )));
    }
    Ok(())
}

/// UDIM numbers of the UV tiles material `m` of `mesh` paints, ascending: `mode` as in
/// [`Session::new_project_tiled`]. Tiles outside 1001..=2000 are an error in UV tile mode and
/// leave an automatic set classic (with a note in `log`).
pub(crate) fn detect_uv_tiles(mesh: &Mesh, m: usize, mode: Option<bool>, log: &mut crate::log::SessionLog) -> std::result::Result<Vec<u32>, String> {
    if mode == Some(false) {
        return Ok(Vec::new());
    }

    let cells = mesh.uv_tiles(m);

    // Mesh v points down: UDIM row r is v in -r..1-r
    let udims: Option<Vec<u32>> =
        cells.iter().map(|&(u, v)| ((0..=9).contains(&u) && (-99..=0).contains(&v)).then(|| 1001 + u.unsigned_abs() + 10 * v.unsigned_abs())).collect();
    match (mode, udims) {
        (Some(true), Some(t)) if t.is_empty() => Ok(vec![1001]),
        (None, Some(t)) if t == [1001] || t.is_empty() => Ok(Vec::new()),
        (_, Some(mut t)) => {
            t.sort_unstable();
            Ok(t)
        }
        (Some(true), None) => Err("UVs lie outside UDIM tiles 1001..2000".into()),
        (None, None) => {
            log.warn("Importer", format!("material {m}: UVs outside UDIM tiles 1001..2000; painting tile 1001 only"));
            Ok(Vec::new())
        }
        (Some(false), _) => Ok(Vec::new()),
    }
}

/// Where texture set `t`'s UV tiles sit in its atlas (the mesh maps its UVs by this).
pub(crate) fn uv_layout(t: &TextureSet) -> (usize, Vec<(u32, f32)>) {
    (t.material, t.uv_tiles.iter().map(|u| (*u, t.tile_scale(*u))).collect())
}

pub(crate) fn next_mesh_id() -> u64 {
    static NEXT_MESH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT_MESH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// `source` with the UVs of each UV-tiled set's material mapped into its atlas (shared when no
/// set is UV-tiled).
pub(crate) fn tiled_mesh(source: &Arc<Mesh>, project: &Project) -> Arc<Mesh> {
    let tiled: Vec<_> = project.texture_sets.iter().filter_map(|ts| Some((ts, ts.tile_grid()?))).collect();
    if tiled.is_empty() {
        return source.clone();
    }
    let mut mesh = (**source).clone();
    let mut done = BTreeSet::new();
    for (ts, grid) in tiled {
        if done.insert(ts.material) {
            mesh.map_material_uvs_with(ts.material, |c, uv| Vec2::from(ts.to_atlas(&grid, c.to_array(), uv.to_array())));
        }
    }
    Arc::new(mesh)
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    #[test]
    fn a_sets_uv_lookup_can_keep_the_first_raster_or_rebuild_it_on_demand() {
        let mut s = Session::new_project("lookup", texturecraft_geom::primitives::plane(1.0, 1), "builtin:plane", None, 128).unwrap();
        let cold = SetCache::new(&s.mesh, &s.doc.project.texture_sets[0], 0);
        assert_eq!(cold.lookup_bytes(), 0, "a lookup outside the budget waits until needed");
        let built = s.caches[0].gbuf().clone();
        assert!(s.caches[0].lookup_bytes() > 0, "the active set keeps its first raster");
        assert!(!s.caches[0].covered_tiles.is_empty());
        s.update();
        assert!(Arc::ptr_eq(&built, s.caches[0].gbuf()), "updates use the first raster");
        assert_eq!((&cold.gbuf().tri, &cold.gbuf().bary), (&built.tri, &built.bary));
        let padding = s.caches[0].dilation().src.clone();
        s.caches[0].release_lookup();
        assert_eq!(s.caches[0].lookup_bytes(), 0);
        assert_eq!((&s.caches[0].gbuf().tri, &s.caches[0].gbuf().bary), (&built.tri, &built.bary));
        assert_eq!(s.caches[0].dilation().src, padding);
    }
}
