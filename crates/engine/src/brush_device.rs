//! A desktop renderer can supply stroke alpha without making the engine depend on wgpu.

use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Weak},
};

use texturecraft_compose::{PaintTarget, Stencil};
use texturecraft_core::Channel;
use texturecraft_geom::{Bvh, Mesh, UvGBuffer};
use texturecraft_paint::{BrushSettings, EyeDepth, Stamp, UvStamp};
use texturecraft_texel::TileImage;

pub type BrushTiles = BTreeSet<(u32, u32)>;
/// Stencil samples by document tile, in row-major linear RGBA.
pub type ProjectedTiles = BTreeMap<(u32, u32), Vec<[f32; 4]>>;

pub struct BrushBackend {
    pub device: Arc<dyn BrushDevice>,
    pub surface: Weak<UvGBuffer>,
}

pub struct BrushSurface {
    pub mesh: Arc<Mesh>,
    pub bvh: Arc<Bvh>,
    pub uv: Arc<UvGBuffer>,
    pub material: usize,
    pub depth: Option<Arc<EyeDepth>>,
    pub settings: BrushSettings,
    pub stencil: Option<Stencil>,
    pub limit: Option<(TileImage, bool)>,
}

pub struct DeviceStrokePreview<'a> {
    pub ts: usize,
    pub target: PaintTarget,
    pub values: &'a BTreeMap<Channel, [f32; 3]>,
    pub mask_value: f32,
    pub opacity: f32,
    /// The Eraser: the stroke scales the target's paint and coverage down instead of painting
    pub erase: bool,
    pub stroke: &'a dyn DeviceStroke,
}

pub trait BrushDevice: Send + Sync {
    fn start(&self, surface: BrushSurface) -> std::result::Result<Box<dyn DeviceStroke>, String>;
}

pub trait DeviceStroke: Send + Sync {
    fn append_world(&mut self, stamps: &[Stamp]) -> std::result::Result<BrushTiles, String>;
    fn append_uv(&mut self, stamps: &[UvStamp]) -> std::result::Result<BrushTiles, String>;
    fn read_alpha(&self, tiles: &BrushTiles) -> std::result::Result<TileImage, String>;
    fn read_projection(&self, tiles: &BrushTiles) -> std::result::Result<ProjectedTiles, String>;
    fn as_any(&self) -> &dyn Any;
}
