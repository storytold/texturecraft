//! Asynchronous layer-panel thumbnails.
//!
//! The panel asks for small previews of layer contents, masks and effect stages
//! ([`ThumbTarget`]). The work never runs on the UI thread and never touches the main composite:
//!
//! * A **snapshot** ([`ThumbSnapshot`]) is an immutable copy of what the compositor reads (the
//!   project, copy-on-write pixel tiles, baked maps, images and the mesh) plus a 64 px G-buffer.
//!   It is rebuilt only when the document really changed, at most every [`DEBOUNCE`] seconds
//!   (a slider drag renders at that pace, not once per frame).
//! * A **worker thread** evaluates one target at a time from a snapshot with
//!   [`texturecraft_compose::thumb::render`]. On wasm32 (no threads) one job runs per
//!   [`ThumbService::poll`] instead.
//! * Results are kept in a small LRU **cache** ([`CACHE_ENTRIES`]). A result computed from an
//!   older snapshot than the current one is dropped, so a slow job can never overwrite a newer
//!   thumbnail. An entry that is out of date keeps showing until its replacement arrives.
//! * Only targets requested during the current frame are rendered: a row that scrolled away
//!   before its turn is dropped from the queue.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use texturecraft_compose::{BakedMaps, ImageStore, Inputs, PixelStore, THUMB_TEXELS, ThumbImage, ThumbTarget, thumb};
use texturecraft_core::Project;
use texturecraft_geom::glam::Vec3;
use texturecraft_geom::{Mesh, UvGBuffer};

use crate::Session;

/// Thumbnails kept.
pub const CACHE_ENTRIES: usize = 96;
/// Seconds between checks for a changed document while it keeps changing.
pub const DEBOUNCE: f64 = 0.12;
/// Jobs handed to the worker at once (the rest wait here, where they can still be dropped).
const IN_FLIGHT: usize = 2;

/// An immutable copy of everything the thumbnails of one texture set read.
pub struct ThumbSnapshot {
    ts: usize,
    mesh_id: u64,
    project: Project,
    pixels: PixelStore,
    baked: BakedMaps,
    images: ImageStore,
    mesh: Arc<Mesh>,
    gbuf: Arc<UvGBuffer>,
    bounds: (Vec3, Vec3),
}

impl ThumbSnapshot {
    /// Snapshot texture set `ts` of `s`, with the G-buffer from `gbufs` (built on a miss).
    pub fn new(s: &Session, ts: usize, gbufs: &mut GbufCache) -> Option<ThumbSnapshot> {
        let material = s.doc.project.texture_set(ts).ok()?.material;
        let gbuf = gbufs.get(&s.mesh, s.mesh_id, material);
        Some(ThumbSnapshot {
            ts,
            mesh_id: s.mesh_id,
            project: s.doc.project.clone(),
            pixels: s.doc.pixels.clone(),
            baked: s.doc.baked.get(&ts).cloned().unwrap_or_default(),
            images: s.images.clone(),
            mesh: s.mesh.clone(),
            gbuf,
            bounds: s.mesh.bounds(),
        })
    }

    /// Whether `s` would give the same thumbnails as this snapshot.
    fn matches(&self, s: &Session, ts: usize) -> bool {
        let same_pixels = self.pixels.layers.len() == s.doc.pixels.layers.len()
            && self.pixels.masks.len() == s.doc.pixels.masks.len()
            && self.pixels.layers.iter().all(|(k, v)| s.doc.pixels.layers.get(k) == Some(v))
            && self.pixels.masks.iter().all(|(k, v)| s.doc.pixels.masks.get(k) == Some(v));
        let none = BakedMaps::default();
        let baked = s.doc.baked.get(&ts).unwrap_or(&none);
        self.ts == ts
            && self.mesh_id == s.mesh_id
            && self.project == s.doc.project
            && same_pixels
            && self.baked == *baked
            && self.images.len() == s.images.len()
            && self.images.iter().all(|(k, v)| s.images.get(k).is_some_and(|o| Arc::ptr_eq(o, v)))
    }

    /// Render `target`: the work of one job.
    pub fn render(&self, target: ThumbTarget) -> Option<ThumbImage> {
        let inp = Inputs {
            project: &self.project,
            ts: self.ts,
            mesh: &self.mesh,
            gbuf: &self.gbuf,
            pixels: &self.pixels,
            baked: &self.baked,
            preview: None,
            images: &self.images,
            bounds: self.bounds,
        };
        thumb::render(&inp, target)
    }

    /// Whether the layer or effect `target` shows still exists in the snapshot's project.
    fn has(&self, target: ThumbTarget) -> bool {
        has_target(&self.project, target)
    }
}

fn has_target(p: &Project, target: ThumbTarget) -> bool {
    match target {
        ThumbTarget::Content { layer, .. } | ThumbTarget::Mask { layer } => p.layers.contains_key(&layer),
        ThumbTarget::Stage { effect, .. } => p.effect(effect).is_some(),
    }
}

/// The 64 px G-buffers built so far, per mesh and material (rasterizing is the one part of a
/// snapshot that is not just copying handles).
#[derive(Default)]
pub struct GbufCache {
    map: HashMap<(u64, usize), Arc<UvGBuffer>>,
}

impl GbufCache {
    fn get(&mut self, mesh: &Mesh, mesh_id: u64, material: usize) -> Arc<UvGBuffer> {
        if self.map.len() > 16 {
            self.map.clear();
        }
        self.map.entry((mesh_id, material)).or_insert_with(|| Arc::new(UvGBuffer::new(mesh, material, THUMB_TEXELS, THUMB_TEXELS))).clone()
    }
}

struct Entry {
    /// None: the target has nothing to show (a layer without that channel).
    image: Option<Arc<ThumbImage>>,
    /// Snapshot generation it was rendered from.
    generation: u64,
    /// Changes whenever `image` does (views key their GPU textures on it).
    version: u64,
    used: u64,
}

/// What the panel draws for one target.
#[derive(Clone)]
pub struct ThumbView {
    pub image: Option<Arc<ThumbImage>>,
    /// Identifies `image`; changes when a new picture arrives.
    pub version: u64,
    /// A newer picture is on its way.
    pub pending: bool,
}

struct Job {
    snapshot: Arc<ThumbSnapshot>,
    generation: u64,
    target: ThumbTarget,
}

struct Done {
    generation: u64,
    target: ThumbTarget,
    image: Option<ThumbImage>,
}

#[cfg(not(target_arch = "wasm32"))]
struct Worker {
    tx: std::sync::mpsc::Sender<Job>,
    rx: std::sync::mpsc::Receiver<Done>,
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_worker() -> Option<Worker> {
    let (tx, jobs) = std::sync::mpsc::channel::<Job>();
    let (done_tx, rx) = std::sync::mpsc::channel::<Done>();
    std::thread::Builder::new()
        .name("layer-thumbs".into())
        .spawn(move || {
            while let Ok(job) = jobs.recv() {
                let image = job.snapshot.render(job.target);
                if done_tx.send(Done { generation: job.generation, target: job.target, image }).is_err() {
                    break;
                }
            }
        })
        .ok()?;
    Some(Worker { tx, rx })
}

/// The thumbnail cache and its worker. One lives in the UI; see the module documentation.
pub struct ThumbService {
    snapshot: Option<Arc<ThumbSnapshot>>,
    /// Bumps whenever the snapshot's content changes; entries and jobs carry it.
    generation: u64,
    /// The session revision the snapshot was last compared with.
    seen: Option<u64>,
    last_check: f64,
    cache: HashMap<ThumbTarget, Entry>,
    tick: u64,
    versions: u64,
    queue: VecDeque<ThumbTarget>,
    in_flight: HashSet<ThumbTarget>,
    wanted: HashSet<ThumbTarget>,
    gbufs: GbufCache,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<Worker>,
    #[cfg(not(target_arch = "wasm32"))]
    spawned: bool,
    /// Jobs run inline (wasm32, or no worker thread could start).
    inline: VecDeque<Job>,
    /// Pictures computed (tests, benchmarks).
    pub rendered: u64,
}

impl Default for ThumbService {
    fn default() -> Self {
        ThumbService {
            snapshot: None,
            generation: 0,
            seen: None,
            last_check: f64::NEG_INFINITY,
            cache: HashMap::new(),
            tick: 0,
            versions: 0,
            queue: VecDeque::new(),
            in_flight: HashSet::new(),
            wanted: HashSet::new(),
            gbufs: GbufCache::default(),
            #[cfg(not(target_arch = "wasm32"))]
            worker: None,
            #[cfg(not(target_arch = "wasm32"))]
            spawned: false,
            inline: VecDeque::new(),
            rendered: 0,
        }
    }
}

impl ThumbService {
    pub fn new() -> ThumbService {
        ThumbService::default()
    }

    /// Start of a frame: forget what the last frame asked for and notice document changes.
    pub fn begin_frame(&mut self, s: &Session, now: f64) {
        self.wanted.clear();
        let ts = s.ts();
        if self.seen == Some(s.revision) && self.snapshot.as_ref().is_some_and(|sn| sn.ts == ts && sn.mesh_id == s.mesh_id) {
            return;
        }
        // While the document keeps changing, compare at the debounce pace (the first look is immediate).
        if self.snapshot.is_some() && now - self.last_check < DEBOUNCE {
            return;
        }
        self.last_check = now;
        if self.snapshot.as_ref().is_some_and(|sn| sn.matches(s, ts)) {
            self.seen = Some(s.revision);
            return;
        }
        self.snapshot = ThumbSnapshot::new(s, ts, &mut self.gbufs).map(Arc::new);
        self.seen = Some(s.revision);
        self.generation += 1;
        // Jobs of the old snapshot are not worth running (those in flight are dropped on arrival).
        self.queue.clear();
        self.inline.clear();
        self.in_flight.clear();
        if let Some(sn) = &self.snapshot {
            self.cache.retain(|t, _| sn.has(*t));
        } else {
            self.cache.clear();
        }
    }

    /// Whether the displayed pictures wait for a document comparison that is still debounced.
    fn checked(&self, s: &Session) -> bool {
        self.seen == Some(s.revision)
    }

    /// The picture for `target` (call for the rows on screen only). Queues work when the
    /// picture is missing or out of date.
    pub fn get(&mut self, s: &Session, target: ThumbTarget) -> ThumbView {
        self.wanted.insert(target);
        self.tick += 1;
        let fresh = self.cache.get(&target).is_some_and(|e| e.generation == self.generation);
        let checked = self.checked(s);
        if !fresh && checked && self.snapshot.is_some() && !self.in_flight.contains(&target) && !self.queue.contains(&target) {
            self.queue.push_back(target);
        }
        let tick = self.tick;
        match self.cache.get_mut(&target) {
            Some(e) => {
                e.used = tick;
                ThumbView { image: e.image.clone(), version: e.version, pending: !(fresh && checked) }
            }
            None => ThumbView { image: None, version: 0, pending: true },
        }
    }

    /// End of a frame: take finished pictures, drop queued rows nobody asked for, hand the worker
    /// its next jobs. Returns whether more work is outstanding (ask for another frame).
    pub fn poll(&mut self, s: &Session) -> bool {
        self.collect();
        let wanted = &self.wanted;
        self.queue.retain(|t| wanted.contains(t));
        self.dispatch();
        self.trim();
        !self.queue.is_empty() || !self.in_flight.is_empty() || !self.inline.is_empty() || !self.checked(s)
    }

    fn dispatch(&mut self) {
        let Some(snapshot) = self.snapshot.clone() else { return };
        while self.in_flight.len() + self.inline.len() < IN_FLIGHT {
            let Some(target) = self.queue.pop_front() else { break };
            let job = Job { snapshot: snapshot.clone(), generation: self.generation, target };
            self.in_flight.insert(target);
            if !self.send(job) {
                break;
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn send(&mut self, job: Job) -> bool {
        if !self.spawned {
            self.spawned = true;
            self.worker = spawn_worker();
        }
        match &self.worker {
            Some(w) => match w.tx.send(job) {
                Ok(()) => true,
                Err(e) => {
                    // The worker is gone: run this and the rest here.
                    self.worker = None;
                    self.inline.push_back(e.0);
                    true
                }
            },
            None => {
                self.inline.push_back(job);
                true
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn send(&mut self, job: Job) -> bool {
        self.inline.push_back(job);
        true
    }

    fn collect(&mut self) {
        let mut done: Vec<Done> = Vec::new();
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(w) = &self.worker {
            while let Ok(d) = w.rx.try_recv() {
                done.push(d);
            }
        }
        // Without a thread one job runs per frame, so the interface keeps its pace.
        if let Some(job) = self.inline.pop_front() {
            let image = job.snapshot.render(job.target);
            done.push(Done { generation: job.generation, target: job.target, image });
        }
        for d in done {
            self.rendered += 1;
            // A picture of an older snapshot never replaces a newer one.
            if d.generation != self.generation {
                continue;
            }
            self.in_flight.remove(&d.target);
            if !self.snapshot.as_ref().is_some_and(|sn| sn.has(d.target)) {
                continue;
            }
            self.versions += 1;
            let entry = Entry { image: d.image.map(Arc::new), generation: d.generation, version: self.versions, used: self.tick };
            self.cache.insert(d.target, entry);
        }
    }

    fn trim(&mut self) {
        while self.cache.len() > CACHE_ENTRIES {
            let Some(oldest) = self.cache.iter().filter(|(t, _)| !self.wanted.contains(t)).min_by_key(|(_, e)| e.used).map(|(t, _)| *t) else { break };
            self.cache.remove(&oldest);
        }
    }

    /// Entries held (tests).
    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    /// Whether `target` has a picture of the current snapshot (tests).
    pub fn is_fresh(&self, target: ThumbTarget) -> bool {
        self.cache.get(&target).is_some_and(|e| e.generation == self.generation)
    }

    /// Whether any work is queued or running (tests).
    pub fn busy(&self) -> bool {
        !self.queue.is_empty() || !self.in_flight.is_empty() || !self.inline.is_empty()
    }
}
