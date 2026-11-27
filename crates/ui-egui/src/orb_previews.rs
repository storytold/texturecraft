//! Material orb pictures, cached by look and rendered off the UI thread: a quick low-resolution picture first, then the full one
//! (the web, which has no threads, renders the full picture at once)

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex};

use egui::{ColorImage, Context, Id, TextureHandle, TextureOptions};

mod orb_job;
#[cfg(not(target_arch = "wasm32"))]
mod orb_result;
mod picture;
#[cfg(not(target_arch = "wasm32"))]
mod worker;

pub(crate) use orb_job::OrbJob;
#[cfg(not(target_arch = "wasm32"))]
use orb_result::OrbResult;
use picture::Picture;
#[cfg(not(target_arch = "wasm32"))]
use worker::Worker;

/// Pictures kept; the least recently drawn go first (a list of every plain material holds about fifty)
const MAX_PICTURES: usize = 160;

/// Most places that keep their last drawn picture (an orb place outlives its look, so places can outnumber pictures)
const MAX_SHOWN_PLACES: usize = MAX_PICTURES * 2;

/// The quick first picture is this many times smaller than the full one, and pictures this small skip it
const QUICK_DIVISOR: usize = 4;
const QUICK_BELOW_PX: usize = 48;

/// Every orb picture of one egui context
#[derive(Default)]
pub struct OrbPreviews {
    pictures: HashMap<u64, Picture>,
    next_request: u64,
    /// The picture last drawn at each place and the pass it was drawn in, shown while that place's new one renders
    shown: HashMap<Id, (TextureHandle, u64)>,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<Worker>,
}

type Shared = Arc<Mutex<OrbPreviews>>;

impl OrbPreviews {
    /// The picture to draw at `place` for the look `key` (see [`OrbJob::key_of`]): its own once rendered, else the last one drawn there (None before the first); `job` builds the render only when the look is new
    pub(crate) fn texture(ctx: &Context, place: Id, key: u64, job: impl FnOnce() -> OrbJob) -> Option<TextureHandle> {
        let shared = ctx.data_mut(|data| data.get_temp_mut_or_default::<Shared>(Id::unique("orb_previews")).clone());
        let mut previews = shared.lock().ok()?;
        previews.take_results(ctx);
        let pass = ctx.cumulative_pass_nr();
        let request = previews.next_request;
        if let Entry::Vacant(slot) = previews.pictures.entry(key) {
            let job = OrbJob { request, ..job() };
            slot.insert(Picture { request, texture: None, full: false, drawn: pass, _images: job.material.image_handles() });
            #[cfg(not(target_arch = "wasm32"))]
            let job = OrbJob { place, ..job };
            previews.next_request = request.wrapping_add(1);
            previews.start(ctx, job);
        }
        let ready = previews.pictures.get_mut(&key).and_then(|picture| {
            picture.drawn = pass;
            picture.texture.clone()
        });
        let on_screen = match ready {
            Some(texture) => {
                previews.shown.insert(place, (texture.clone(), pass));
                Some(texture)
            }
            None => previews.shown.get_mut(&place).map(|(texture, drawn)| {
                *drawn = pass;
                texture.clone()
            }),
        };

        // A new look or a new place may pass the cap
        previews.forget_old();
        on_screen
    }

    /// Whether every orb asked for has its full picture (tests and captures wait for this)
    pub fn settled(ctx: &Context) -> bool {
        let shared = ctx.data_mut(|data| data.get_temp_mut_or_default::<Shared>(Id::unique("orb_previews")).clone());
        let Ok(mut previews) = shared.lock() else { return true };
        previews.take_results(ctx);
        previews.pictures.values().all(|picture| picture.full)
    }

    /// The size in pixels of the picture last drawn at `place` (tests)
    #[cfg(test)]
    pub(crate) fn shown_size(ctx: &Context, place: &str) -> Option<[usize; 2]> {
        let shared = ctx.data_mut(|data| data.get_temp_mut_or_default::<Shared>(Id::unique("orb_previews")).clone());
        let previews = shared.lock().ok()?;
        previews.shown.get(&Id::unique(("material_orb", place))).map(|(texture, _)| texture.size())
    }

    fn store(&mut self, ctx: &Context, key: u64, request: u64, full: bool, image: ColorImage) {
        if let Some(picture) = self.pictures.get_mut(&key)
            && picture.request == request
            && !picture.full
        {
            picture.texture = Some(ctx.load_texture("material-orb", image, TextureOptions::LINEAR));
            picture.full = full;
        }
    }

    /// Drop the least recently drawn pictures past [`MAX_PICTURES`] and places' last shown pictures past [`MAX_SHOWN_PLACES`]
    fn forget_old(&mut self) {
        while self.pictures.len() > MAX_PICTURES {
            let Some(oldest) = self.pictures.iter().min_by_key(|(_, picture)| picture.drawn).map(|(key, _)| *key) else { break };
            self.pictures.remove(&oldest);
        }

        // A place no longer drawn (a deleted layer) must not keep its texture alive
        while self.shown.len() > MAX_SHOWN_PLACES {
            let Some(oldest) = self.shown.iter().min_by_key(|(_, (_, drawn))| *drawn).map(|(place, _)| *place) else { break };
            self.shown.remove(&oldest);
        }
    }

    /// The web renders the full picture at once
    #[cfg(target_arch = "wasm32")]
    fn start(&mut self, ctx: &Context, job: OrbJob) {
        let job = OrbJob { quick: false, ..job };
        self.store(ctx, job.key, job.request, true, job.picture());
    }

    #[cfg(target_arch = "wasm32")]
    fn take_results(&mut self, _ctx: &Context) {}

    #[cfg(not(target_arch = "wasm32"))]
    fn start(&mut self, ctx: &Context, job: OrbJob) {
        let key = job.key;
        let worker = match self.worker.take() {
            Some(worker) => worker,
            None => match Worker::start() {
                Some(worker) => worker,
                None => {
                    // No thread: render here rather than show nothing
                    let job = OrbJob { quick: false, ..job };
                    self.store(ctx, key, job.request, true, job.picture());
                    return;
                }
            },
        };
        match worker.jobs.send((job, ctx.clone())) {
            Ok(()) => self.worker = Some(worker),
            Err(std::sync::mpsc::SendError((job, _))) => {
                let job = OrbJob { quick: false, ..job };
                self.store(ctx, key, job.request, true, job.picture());
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn take_results(&mut self, ctx: &Context) {
        let Some(worker) = &self.worker else { return };
        let results: Vec<OrbResult> = worker.results.try_iter().collect();
        for result in results {
            match result {
                OrbResult::Picture { key, request, full, image } => self.store(ctx, key, request, full, image),
                OrbResult::Dropped { key, request } => {
                    if let Entry::Occupied(entry) = self.pictures.entry(key)
                        && entry.get().request == request
                    {
                        entry.remove();
                    }
                }
            }
        }
    }
}

/// Keep only the newest job per place (others than [`Id::NULL`]); returns the keys of the jobs dropped
#[cfg(not(target_arch = "wasm32"))]
fn newest_per_place(waiting: &mut Vec<OrbJob>) -> Vec<(u64, u64)> {
    let mut seen = std::collections::HashSet::new();
    let mut dropped = Vec::new();
    let mut kept: Vec<OrbJob> = Vec::with_capacity(waiting.len());
    for job in waiting.drain(..).rev() {
        if job.place == Id::NULL || seen.insert(job.place) {
            kept.push(job);
        } else {
            dropped.push((job.key, job.request));
        }
    }
    kept.reverse();
    *waiting = kept;
    dropped
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use egui::Color32;
    use texturecraft_engine::compose::ImageRes;
    use texturecraft_engine::core::{Channel, Fill, Source};

    use super::*;
    use crate::orb_material::OrbMaterial;

    fn copper(roughness: f32) -> OrbMaterial {
        OrbMaterial::uniform([0.8, 0.35, 0.18], 1.0, roughness)
    }

    /// One UI pass; nothing draws the textures it makes, so their updates are dropped
    fn run(ctx: &Context, ui: impl FnMut(&Context)) {
        let mut output = ctx.run_pass(Default::default(), ui);
        output.textures_delta.clear();
    }

    /// Draw `job` at `place` every pass until it is full or 20 s pass; returns the sizes shown on the way
    fn draw_until_settled(ctx: &Context, place: Id, job: &OrbJob) -> Vec<[usize; 2]> {
        let start = Instant::now();
        let mut sizes = Vec::new();
        while start.elapsed() < Duration::from_secs(20) {
            let mut size = None;
            run(ctx, |ctx| size = OrbPreviews::texture(ctx, place, job.key, || job.clone()).map(|texture| texture.size()));
            if let Some(size) = size
                && sizes.last() != Some(&size)
            {
                sizes.push(size);
            }
            if OrbPreviews::settled(ctx) {
                run(ctx, |ctx| size = OrbPreviews::texture(ctx, place, job.key, || job.clone()).map(|texture| texture.size()));
                if let Some(size) = size
                    && sizes.last() != Some(&size)
                {
                    sizes.push(size);
                }
                break;
            }

            std::thread::sleep(Duration::from_millis(5));
        }
        sizes
    }

    #[test]
    fn an_orb_renders_off_the_ui_thread_quick_first_then_full_and_is_cached_by_its_look() {
        let ctx = Context::default();
        let place = Id::unique("orb");
        let job = OrbJob::new(copper(0.3), 200, Color32::BLACK);
        let mut first = None;
        run(&ctx, |ctx| first = OrbPreviews::texture(ctx, place, job.key, || job.clone()));

        assert!(first.is_none(), "the first pass does not wait for the render");

        let sizes = draw_until_settled(&ctx, place, &job);

        // Both pictures can arrive between frames, so only the full picture is guaranteed to appear
        let quick_then_full: &[[usize; 2]] = &[[50, 50], [200, 200]];
        assert!(sizes == quick_then_full || sizes == quick_then_full[1..], "the quick picture, if shown, then the full one: {sizes:?}");
        assert!(OrbPreviews::settled(&ctx));

        // The same look again: drawn at once from the cache, nothing new to render
        let mut again = None;
        run(&ctx, |ctx| again = OrbPreviews::texture(ctx, place, job.key, || job.clone()));

        assert_eq!(again.map(|texture| texture.size()), Some([200, 200]));
        assert!(OrbPreviews::settled(&ctx), "a cached look renders nothing");

        // A new look keeps the old picture on screen until its own arrives
        let rougher = OrbJob::new(copper(0.8), 200, Color32::BLACK);
        let mut meanwhile = None;
        run(&ctx, |ctx| meanwhile = OrbPreviews::texture(ctx, place, rougher.key, || rougher.clone()));

        assert!(meanwhile.is_some(), "the last picture stays while the new one renders");
        assert!(!OrbPreviews::settled(&ctx), "the new look is rendering");

        draw_until_settled(&ctx, place, &rougher);

        assert!(OrbPreviews::settled(&ctx));
    }

    /// A dropped context lets go of its orb pictures and closes its worker's job channel: the idle worker holds no context
    #[test]
    fn a_dropped_context_frees_its_previews_while_the_worker_idles() {
        // Long enough for the worker to finish its last job on a busy machine
        const ATTEMPTS: usize = 2000;
        const WAIT: Duration = Duration::from_millis(5);
        let ctx = Context::default();
        let job = OrbJob::new(copper(0.3), 64, Color32::BLACK);
        draw_until_settled(&ctx, Id::unique("orb"), &job);
        let shared = ctx.data(|data| data.get_temp::<Shared>(Id::unique("orb_previews"))).unwrap();
        let previews = Arc::downgrade(&shared);
        drop(shared);

        drop(ctx);

        let freed = (0..ATTEMPTS).any(|_| {
            let freed = previews.upgrade().is_none();
            if !freed {
                std::thread::sleep(WAIT);
            }
            freed
        });
        assert!(freed, "the worker kept the dropped context and its previews alive");
    }

    #[test]
    fn a_newer_look_for_the_same_place_replaces_one_still_waiting() {
        let job = |roughness: f32| OrbJob { place: Id::unique("orb"), ..OrbJob::new(copper(roughness), 64, Color32::BLACK) };
        let other = OrbJob { place: Id::unique("other"), ..OrbJob::new(copper(0.5), 64, Color32::BLACK) };
        let mut waiting = vec![job(0.1), other.clone(), job(0.2), job(0.3)];
        let dropped = newest_per_place(&mut waiting);
        assert_eq!(dropped.len(), 2);
        assert_eq!(waiting.iter().map(|job| job.key).collect::<Vec<_>>(), vec![other.key, job(0.3).key]);
    }

    #[test]
    fn a_delayed_drop_keeps_the_replacement_render_for_an_evicted_look() {
        let ctx = Context::default();
        let (jobs, submitted) = std::sync::mpsc::channel();
        let (completed, results) = std::sync::mpsc::channel();
        let previews = OrbPreviews { worker: Some(Worker { jobs, results }), ..Default::default() };
        ctx.data_mut(|data| data.insert_temp(Id::unique("orb_previews"), Arc::new(Mutex::new(previews))));
        let first = OrbJob::new(copper(0.0), 16, Color32::BLACK);
        let place = Id::unique("changing material");
        let request = |job: &OrbJob| {
            run(&ctx, |ctx| {
                OrbPreviews::texture(ctx, place, job.key, || job.clone());
            })
        };

        request(&first);
        let (old_job, _) = submitted.try_recv().unwrap();
        for index in 1..=MAX_PICTURES {
            let other = OrbJob::new(copper(index as f32 / MAX_PICTURES as f32), 16, Color32::BLACK);
            request(&other);
            submitted.try_recv().unwrap();
        }
        request(&first);
        let (replacement, _) = submitted.try_recv().expect("eviction requested a new render");

        completed.send(OrbResult::Dropped { key: old_job.key, request: old_job.request }).unwrap();
        completed
            .send(OrbResult::Picture { key: replacement.key, request: replacement.request, full: true, image: ColorImage::filled([16, 16], Color32::WHITE) })
            .unwrap();
        let texture = OrbPreviews::texture(&ctx, place, first.key, || first.clone());

        assert!(texture.is_some(), "the new render should be visible after the old job was dropped");
        assert!(submitted.try_recv().is_err(), "the completed look needs no third render");
    }

    #[test]
    fn places_no_longer_drawn_let_go_of_their_last_picture_past_the_cap() {
        let ctx = Context::default();
        let texture = ctx.load_texture("orb", ColorImage::filled([1, 1], Color32::BLACK), TextureOptions::LINEAR);
        let mut previews = OrbPreviews::default();
        let places = MAX_SHOWN_PLACES as u64 + 10;
        for pass in 0..places {
            previews.shown.insert(Id::unique(("orb", pass)), (texture.clone(), pass));
        }

        previews.forget_old();

        assert_eq!(previews.shown.len(), MAX_SHOWN_PLACES, "the cap holds for places too");
        assert!(!previews.shown.contains_key(&Id::unique(("orb", 0_u64))), "the least recently drawn place goes first");
        assert!(previews.shown.contains_key(&Id::unique(("orb", places - 1))), "the latest drawn place stays");
    }

    /// An orb of a fill whose base colour reads `image` under one name, the name a re-import keeps
    fn image_orb(image: Arc<ImageRes>) -> OrbJob {
        let fill = Fill { sources: BTreeMap::from([(Channel::BaseColor, Source::image(REIMPORTED))]), ..Fill::default() };
        let images = HashMap::from([(REIMPORTED.to_owned(), image)]);
        OrbJob::new(OrbMaterial::of_fill(&fill, &[Channel::BaseColor], &images), 64, Color32::BLACK)
    }

    /// An image name imported twice
    const REIMPORTED: &str = "grain";

    #[test]
    fn a_new_image_under_a_cached_ones_name_and_size_gets_its_own_picture() {
        let ctx = Context::default();
        let place = Id::unique("orb");
        let image = Arc::new(ImageRes::new(2, 2, vec![[0.1; 4]; 4], false));
        let first = image_orb(Arc::clone(&image));
        draw_until_settled(&ctx, place, &first);
        let first_key = first.key;
        drop(first);

        // The cached picture holds the image weakly, so its address stays taken however the allocator reuses blocks
        assert!(Arc::weak_count(&image) >= 1, "the cached picture keeps the image's address");

        drop(image);

        // Same name and size: the freed image's address must not come back to the cached picture's key
        let second = image_orb(Arc::new(ImageRes::new(2, 2, vec![[0.9; 4]; 4], false)));
        assert_ne!(second.key, first_key, "the new image's orb is drawn anew");
    }

    #[test]
    fn small_orbs_skip_the_quick_picture() {
        assert!(!OrbJob::new(copper(0.3), 24, Color32::BLACK).quick);
        assert!(OrbJob::new(copper(0.3), 96, Color32::BLACK).quick);
    }
}
