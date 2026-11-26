//! Padding of the viewport composite. Uniform UV coverage has no pixels to copy.

use std::collections::BTreeSet;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use texturecraft_compose::{Composite, pad_tiles};
use texturecraft_geom::{Mesh, NO_TRI, UvGBuffer};
use texturecraft_texel::DilationMap;

use crate::session::DISPLAY_PADDING;

/// The set's UV surface lookup, built on first use and shared with display padding.
pub struct UvLookup(Arc<UvLookupInner>);

enum UvLookupInner {
    Ready(Arc<UvGBuffer>),
    Lazy { mesh: Arc<Mesh>, material: usize, size: (u32, u32), value: OnceLock<Arc<UvGBuffer>> },
}

impl UvLookup {
    pub(crate) fn lazy(mesh: Arc<Mesh>, material: usize, size: (u32, u32), seed: Option<Arc<UvGBuffer>>) -> Self {
        let value = seed.map(OnceLock::from).unwrap_or_default();
        Self(Arc::new(UvLookupInner::Lazy { mesh, material, size, value }))
    }

    fn share(&self) -> Self {
        Self(Arc::clone(&self.0))
    }

    pub(crate) fn evicted(&self) -> Self {
        match self.0.as_ref() {
            UvLookupInner::Ready(_) => self.share(),
            UvLookupInner::Lazy { mesh, material, size, .. } => Self::lazy(Arc::clone(mesh), *material, *size, None),
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        let gbuf = match self.0.as_ref() {
            UvLookupInner::Ready(value) => Some(value),
            UvLookupInner::Lazy { value, .. } => value.get(),
        };
        gbuf.map_or(0, |g| Self::raster_bytes(g))
    }

    pub(crate) fn raster_bytes(gbuf: &UvGBuffer) -> usize {
        gbuf.tri.capacity() * size_of::<u32>() + gbuf.bary.capacity() * size_of::<[f32; 2]>()
    }
}

impl Deref for UvLookup {
    type Target = Arc<UvGBuffer>;

    fn deref(&self) -> &Self::Target {
        match self.0.as_ref() {
            UvLookupInner::Ready(gbuf) => gbuf,
            UvLookupInner::Lazy { mesh, material, size: (w, h), value } => value.get_or_init(|| Arc::new(UvGBuffer::new(mesh, *material, *w, *h))),
        }
    }
}

/// The viewport's padding map. Reading the dense map explicitly also works for uniform UVs.
pub struct DisplayPadding {
    dense: Arc<OnceLock<Arc<DilationMap>>>,
    size: (u32, u32),
    mixed: Option<UvLookup>,
    block: u32,
    #[cfg(not(target_arch = "wasm32"))]
    worker_started: bool,
    /// All texels covered or all uncovered. Mixed coverage needs the ordinary dilation map.
    uniform: Option<bool>,
}

impl DisplayPadding {
    pub(crate) fn new(gbuf: &Arc<UvGBuffer>, has_coverage: bool, block: u32) -> Self {
        let lookup = UvLookup(Arc::new(UvLookupInner::Ready(Arc::clone(gbuf))));
        Self::with_lookup(&lookup, gbuf, has_coverage, block)
    }

    pub(crate) fn with_lookup(lookup: &UvLookup, coverage: &UvGBuffer, has_coverage: bool, block: u32) -> Self {
        let size = (coverage.width, coverage.height);
        let uniform = if !has_coverage {
            Some(false)
        } else if !coverage.tri.contains(&NO_TRI) {
            Some(true)
        } else {
            None
        };
        let mixed = uniform.is_none().then(|| lookup.share());
        Self {
            dense: Arc::default(),
            size,
            mixed,
            block,
            #[cfg(not(target_arch = "wasm32"))]
            worker_started: false,
            uniform,
        }
    }

    pub(crate) fn apply_to_tiles(&self, output: &mut Composite, tiles: &[(u32, u32)]) -> BTreeSet<(u32, u32)> {
        if self.uniform.is_some() { BTreeSet::new() } else { pad_tiles(output, self, tiles) }
    }

    /// The same nearest covered source as the dense map, without constructing it for one pick.
    pub(crate) fn source_for_sample(&self, x: u32, y: u32) -> Option<(u32, u32)> {
        if let Some(dense) = self.dense.get() {
            return dense.source(x, y);
        }
        let gbuf = self.mixed.as_ref()?;
        let block = if self.block == 0 { u32::MAX } else { self.block };
        let bounds = |pos: u32, size: u32| {
            let start = pos / block * block;
            let last = start.saturating_add(block - 1).min(size - 1);
            pos.saturating_sub(DISPLAY_PADDING).max(start)..=pos.saturating_add(DISPLAY_PADDING).min(last)
        };
        let columns = bounds(x, self.size.0);
        bounds(y, self.size.1)
            .flat_map(|sy| columns.clone().map(move |sx| (sx, sy)))
            .filter(|&(sx, sy)| gbuf.tri[(sy * gbuf.width + sx) as usize] != NO_TRI)
            .min_by_key(|&(sx, sy)| (sx.abs_diff(x).max(sy.abs_diff(y)), sy, sx))
    }

    /// Whether every texel has a surface sample and the display needs no padding.
    pub fn fully_covered(&self) -> bool {
        self.uniform == Some(true)
    }

    /// Share the UV coverage so GPU display can calculate padding without the CPU map.
    pub fn mixed_coverage(&self) -> Option<Arc<UvGBuffer>> {
        self.mixed.as_ref().map(|lookup| Arc::clone(lookup))
    }

    /// Whether applying CPU viewport padding can proceed without waiting for the dense map.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn is_ready(&self) -> bool {
        self.mixed.is_none() || self.dense.get().is_some()
    }

    /// Start the dense CPU fallback on a worker so the viewport can keep drawing.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn prepare_on_worker(&mut self) -> bool {
        if self.is_ready() {
            return true;
        }
        let Some(gbuf) = self.mixed.as_ref() else { return true };
        if !self.worker_started {
            let (gbuf, dense, block) = (Arc::clone(gbuf), self.dense.clone(), self.block);
            self.worker_started = std::thread::Builder::new()
                .name("texturecraft-padding".into())
                .spawn(move || {
                    dense.get_or_init(|| mixed_sources(&gbuf, block));
                })
                .is_ok();
        }
        // If creating a thread failed, let the caller prepare the map synchronously.
        !self.worker_started
    }

    pub(crate) fn bytes(&self) -> usize {
        self.dense.get().map_or(0, |map| map.src.capacity() * size_of::<u32>())
    }

    pub(crate) fn evict(&mut self, lookup: &UvLookup) {
        if self.mixed.is_some() {
            self.mixed = Some(lookup.share());
        }
        self.dense = Arc::default();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.worker_started = false;
        }
    }
}

impl Deref for DisplayPadding {
    type Target = Arc<DilationMap>;

    fn deref(&self) -> &Self::Target {
        self.dense.get_or_init(|| {
            let (width, height) = self.size;
            if let Some(gbuf) = &self.mixed {
                return mixed_sources(gbuf, self.block);
            }
            let n = width as usize * height as usize;
            let src = if self.uniform == Some(true) { (0..n as u32).collect() } else { vec![texturecraft_texel::dilate::NONE; n] };
            Arc::new(DilationMap { width, height, src })
        })
    }
}

fn mixed_sources(gbuf: &UvGBuffer, block: u32) -> Arc<DilationMap> {
    Arc::new(DilationMap::in_blocks(&gbuf.coverage_mask(), gbuf.width, gbuf.height, Some(DISPLAY_PADDING), block))
}
