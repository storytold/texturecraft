use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use texturecraft_engine::Session;
use texturecraft_engine::compose::{BakedMaps, Inputs, ProjectorDepthCache, StackKey, eval::composite_linear_tiles};
use texturecraft_engine::core::LayerId;
use texturecraft_render::material_underlay::MaterialUnderlay;

use super::{CompositeKey, LitEnvironment};

pub(super) struct Underlay {
    layer: LayerId,
    mesh: u64,
    revision: u64,
    environment: LitEnvironment,
    key: StackKey,
    cancel: Arc<AtomicBool>,
    pending: Option<mpsc::Receiver<Arc<MaterialUnderlay>>>,
    material: Option<Arc<MaterialUnderlay>>,
}

impl Underlay {
    pub(super) fn matches_selection(&self, session: &Session, layer: Option<LayerId>) -> bool {
        layer == Some(self.layer) && session.mesh_id == self.mesh
    }

    pub(super) fn matches_environment(&self, session: &Session) -> bool {
        self.environment.matches(session)
    }

    pub(super) fn pending(&self) -> bool {
        self.pending.is_some()
    }

    #[cfg(test)]
    pub(super) fn ready(&self) -> Option<Arc<MaterialUnderlay>> {
        self.material.clone()
    }

    pub(super) fn material(
        state: &mut Option<Self>,
        session: &Session,
        ts: usize,
        (layer, first): (LayerId, usize),
        key: CompositeKey,
        budget: u64,
    ) -> Option<Arc<MaterialUnderlay>> {
        let project = &session.doc.project;
        let cache = session.caches.get(ts)?;
        let set = project.texture_set(ts).ok()?;
        let same_selection = state.as_ref().is_some_and(|state| state.layer == layer && state.mesh == key.mesh);
        if !same_selection || state.as_ref().is_some_and(|state| state.revision != key.revision || !state.matches_environment(session)) {
            let empty = BakedMaps::default();
            let baked = session.doc.baked.get(&ts).unwrap_or(&empty);
            let below = set.stack.get(..first)?;
            let stack_key = StackKey::new(project, &session.doc.pixels, baked, &session.images, ts, below)?;
            if let Some(previous) = state.as_mut().filter(|state| same_selection && state.key == stack_key) {
                previous.revision = key.revision;
                previous.environment = LitEnvironment::of(session);
            } else {
                let mut lower = project.clone();
                lower.texture_set_mut(ts).ok()?.stack.truncate(first);
                let mesh = session.mesh.clone();
                let gbuf = cache.gbuf.clone();
                let tiles = cache.covered_tiles.clone();
                let pixels = session.doc.pixels.clone();
                let baked = baked.clone();
                let images = session.images.clone();
                let block_scales = set.block_scales();
                let size = cache.size;
                let (send, receive) = mpsc::sync_channel(1);
                let cancel = Arc::new(AtomicBool::new(false));
                let worker_cancel = cancel.clone();
                *state = Some(Self {
                    layer,
                    mesh: key.mesh,
                    revision: key.revision,
                    environment: LitEnvironment::of(session),
                    key: stack_key,
                    cancel,
                    pending: Some(receive),
                    material: None,
                });
                rayon::spawn(move || {
                    if worker_cancel.load(Ordering::Acquire) {
                        return;
                    }

                    let Some(mut material) = MaterialUnderlay::new(size, budget) else { return };
                    let inputs = Inputs {
                        project: &lower,
                        ts,
                        mesh: &mesh,
                        gbuf: &gbuf,
                        pixels: &pixels,
                        baked: &baked,
                        block_scales,
                        preview: None,
                        images: &images,
                        bounds: mesh.bounds(),
                    };
                    // Compute all tiles in one pass so spatial effects retain their field plans
                    composite_linear_tiles(
                        &inputs,
                        &ProjectorDepthCache::default(),
                        &tiles,
                        || worker_cancel.load(Ordering::Relaxed),
                        |tile, channels, data| {
                            if !worker_cancel.load(Ordering::Relaxed) && !material.record_tile(tile, channels, data) {
                                worker_cancel.store(true, Ordering::Relaxed);
                            }
                        },
                    );
                    if !worker_cancel.load(Ordering::Acquire) {
                        let _ = send.send(Arc::new(material));
                    }
                });
            }
        }

        let state = state.as_mut()?;
        if let Some(pending) = &state.pending {
            match pending.try_recv() {
                Ok(material) => {
                    state.material = Some(material);
                    state.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => state.pending = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        state.material.clone()
    }
}

impl Drop for Underlay {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}
