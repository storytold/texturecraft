//! Filling a shared slot from threads that run parallel work, without ever waiting where waiting could hang.
//!
//! A rayon thread that waits for parallel work runs other pending work meanwhile. So a thread can hold, further down
//! its stack, a piece of the work that fills a slot (or be filling it itself) when it asks for that slot: waiting there
//! would never end. Which thread holds what is not known, so the rule is: **a pool thread never waits for a slot
//! another fill has claimed; it computes its own copy.** Threads outside any pool wait as usual.
//!
//! Graph renders run their parallel work on the runtime's own pool ([`super::GraphStore`]'s), never on the pool the
//! compositor runs on, so no thread of the compositor's pool can hold a piece of a render: others may wait for a
//! render safely ([`Slots::get_or_fill_own_pool`]). Only the claiming thread itself asking again gets its own copy.
//!
//! A pool thread that waits for another pool (`install`) runs pending work of its own pool meanwhile, and that work
//! may ask for the slot the thread is filling: it would fill it again, one copy deeper on the same stack for every
//! piece it picks up. A compositor tile asking for a graph's cook is such a piece (Doll.spp: 320 nested renders of
//! one graph, 16 GB, then a stack overflow). So a fill that waits for its own pool runs through [`wait_outside`],
//! which waits on a plain thread and so picks up nothing.
//!
//! Copies cost time, not correctness: fills are deterministic.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;

/// Values by key, each filled once; see the module docs for when a thread waits.
pub struct Slots<K, V> {
    state: Mutex<State<K, V>>,
    filled: Condvar,
}

struct State<K, V> {
    done: HashMap<K, V>,
    /// Keys being filled, with the filling thread.
    claimed: HashMap<K, ThreadId>,
}

impl<K, V> Default for Slots<K, V> {
    fn default() -> Self {
        Slots { state: Mutex::new(State { done: HashMap::new(), claimed: HashMap::new() }), filled: Condvar::new() }
    }
}

impl<K: Hash + Eq + Clone, V: Clone> Slots<K, V> {
    fn lock(&self) -> MutexGuard<'_, State<K, V>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The value of `key` if it is filled.
    pub fn get(&self, key: &K) -> Option<V> {
        self.lock().done.get(key).cloned()
    }

    /// Puts a value in (replacing one there).
    pub fn insert(&self, key: K, value: V) {
        self.lock().done.insert(key, value);
        self.filled.notify_all();
    }

    pub fn remove(&self, key: &K) -> Option<V> {
        self.lock().done.remove(key)
    }

    /// Every filled value.
    pub fn values(&self) -> Vec<(K, V)> {
        self.lock().done.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn retain(&self, keep: impl Fn(&K) -> bool) {
        self.lock().done.retain(|k, _| keep(k));
    }

    /// `key`'s value, filled with `fill` on first use. A pool thread finding the key claimed by another fill computes its
    /// own copy instead of waiting (for fills whose parallel work may run on the caller's pool).
    pub fn get_or_fill(&self, key: &K, fill: impl FnOnce() -> V) -> V {
        self.fill(key, fill, false)
    }

    /// As [`Slots::get_or_fill`], for fills that run their parallel work on a pool of their own (graph renders): others
    /// wait for the fill; only the claiming thread asking again computes its own copy.
    pub fn get_or_fill_own_pool(&self, key: &K, fill: impl FnOnce() -> V) -> V {
        self.fill(key, fill, true)
    }

    fn fill(&self, key: &K, fill: impl FnOnce() -> V, others_wait: bool) -> V {
        let me = std::thread::current().id();
        let in_pool = rayon::current_thread_index().is_some();
        let mut state = self.lock();
        loop {
            if let Some(v) = state.done.get(key) {
                return v.clone();
            }
            match state.claimed.get(key).copied() {
                None => break,
                Some(claimant) if claimant == me || (in_pool && !others_wait) => {
                    drop(state);
                    return fill();
                }
                Some(_) => state = self.filled.wait(state).unwrap_or_else(PoisonError::into_inner),
            }
        }
        state.claimed.insert(key.clone(), me);
        drop(state);
        let release = Release { slots: self, key: Some(key.clone()) };
        let v = fill();
        let mut state = self.lock();
        state.done.entry(key.clone()).or_insert_with(|| v.clone());
        drop(state);
        drop(release);
        v
    }
}

/// Gives a claimed key back when its fill ends, also by a panic, and wakes the waiting threads.
struct Release<'s, K: Hash + Eq, V> {
    slots: &'s Slots<K, V>,
    key: Option<K>,
}

impl<K: Hash + Eq, V> Drop for Release<'_, K, V> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            let mut state = self.slots.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.claimed.remove(&key);
        }
        self.slots.filled.notify_all();
    }
}

/// Stack of the thread [`wait_outside`] starts: the size of a main thread's, as renders on the main thread have.
const OUTSIDE_STACK: usize = 8 << 20;

/// Runs `f`, which waits for work on `pool`, so that the waiting thread runs no other pending work: on a thread of
/// any other pool, `f` runs on a plain thread of its own while the caller blocks (see the module docs). Elsewhere, and
/// where no thread can be started, `f` runs here.
pub fn wait_outside<T: Send>(pool: Option<&rayon::ThreadPool>, f: impl FnOnce() -> Result<T, String> + Send) -> Result<T, String> {
    let foreign = rayon::current_thread_index().is_some() && pool.is_some_and(|p| p.current_thread_index().is_none());
    if !foreign {
        return f();
    }
    let job = Mutex::new(Some(f));
    let run = || job.lock().unwrap_or_else(PoisonError::into_inner).take().map(|f| f());
    let ran = std::thread::scope(|scope| {
        let started = std::thread::Builder::new().name("texturecraft-render".into()).stack_size(OUTSIDE_STACK).spawn_scoped(scope, run);
        started.ok().map(|h| h.join().unwrap_or_else(|_| Some(Err("the render stopped with an internal error".to_string()))))
    });
    match ran.flatten() {
        Some(r) => r,
        // the thread could not start: run here
        None => run().unwrap_or_else(|| Err("the render did not run".to_string())),
    }
}

/// A shareable [`Slots`].
pub type SharedSlots<K, V> = Arc<Slots<K, V>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_fill_is_kept() {
        let slots: Slots<u32, u32> = Slots::default();
        assert_eq!(slots.get_or_fill(&1, || 7), 7);
        assert_eq!(slots.get_or_fill(&1, || 8), 7);
        assert_eq!(slots.get(&1), Some(7));
    }

    #[test]
    fn a_pool_thread_asking_for_a_slot_it_fills_gets_its_own_copy() {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let slots: Slots<u32, u32> = Slots::default();
        let inner = pool.install(|| slots.get_or_fill(&1, || slots.get_or_fill(&1, || 3) + 1));
        assert_eq!(inner, 4);
        assert_eq!(slots.get(&1), Some(4));
    }

    /// The pattern that hung the first graph sources: thread A claims the slot and waits for a piece of its fill that
    /// thread B runs; B, inside that piece, asks for the slot. B waiting would hang both, so B computes its own copy.
    #[test]
    fn a_pool_thread_never_waits_for_a_fill_held_by_work_below_it() {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
        let slots: Slots<u32, u32> = Slots::default();
        let (done, finished) = std::sync::mpsc::channel();
        let slots = Arc::new(slots);
        let s = slots.clone();
        // a watchdog: the test fails instead of hanging
        std::thread::spawn(move || {
            let v = pool.install(|| {
                s.get_or_fill(&1, || {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let rx = Mutex::new(rx);
                    let s = &s;
                    rayon::scope(|scope| {
                        scope.spawn(move |_| {
                            let _ = tx.send(s.get_or_fill(&1, || 5));
                        });
                        // block outside rayon: the piece can only run on the other thread
                        rx.lock().unwrap().recv().unwrap()
                    })
                })
            });
            let _ = done.send(v);
        });
        let v = finished.recv_timeout(std::time::Duration::from_secs(20)).expect("a pool thread waited for a fill held below it");
        assert_eq!(v, 5);
        assert_eq!(slots.get(&1), Some(5));
    }

    /// Doll.spp: a compositor thread filling a graph's cook waits for the render's pool and meanwhile runs other tiles,
    /// which ask for the same cook and fill it again, one copy deeper each. Waiting outside, it takes no other tile.
    #[test]
    fn a_fill_waiting_for_its_own_pool_takes_no_other_work() {
        use rayon::prelude::*;
        let compositor = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let render = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
        let slots: Slots<u32, u32> = Slots::default();
        let fills = std::sync::atomic::AtomicUsize::new(0);
        let got: Vec<u32> = compositor.install(|| {
            (0..64)
                .into_par_iter()
                .map(|_| {
                    slots.get_or_fill_own_pool(&1, || {
                        fills.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let rendered = wait_outside(Some(&render), || {
                            Ok(render.install(|| {
                                std::thread::sleep(std::time::Duration::from_millis(20));
                                7
                            }))
                        });
                        rendered.unwrap()
                    })
                })
                .collect()
        });
        assert!(got.iter().all(|v| *v == 7));
        assert_eq!(fills.into_inner(), 1, "the filling thread took other tiles while it waited and filled again");
    }

    #[test]
    fn work_on_the_render_pool_itself_runs_in_place() {
        let render = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let caller = std::thread::current().id();
        let here = render.install(|| {
            let me = std::thread::current().id();
            wait_outside(Some(&render), || Ok(std::thread::current().id() == me)).unwrap()
        });
        assert!(here);
        assert_eq!(wait_outside(Some(&render), || Ok(std::thread::current().id())).unwrap(), caller, "outside any pool");
    }

    #[test]
    fn threads_outside_a_pool_wait_for_the_fill() {
        let slots = Arc::new(Slots::<u32, u32>::default());
        let (started, go) = std::sync::mpsc::channel();
        let s = slots.clone();
        let filler = std::thread::spawn(move || {
            s.get_or_fill_own_pool(&1, || {
                let _ = started.send(());
                std::thread::sleep(std::time::Duration::from_millis(50));
                9
            })
        });
        go.recv().unwrap();
        let fills = std::sync::atomic::AtomicUsize::new(0);
        let v = slots.get_or_fill_own_pool(&1, || {
            fills.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            10
        });
        assert_eq!((v, filler.join().unwrap()), (9, 9));
        assert_eq!(fills.into_inner(), 0, "the second asker waited instead of filling again");
    }

    #[test]
    fn a_panicking_fill_gives_the_slot_back() {
        let slots: Slots<u32, u32> = Slots::default();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| slots.get_or_fill(&1, || panic!("fill fails"))));
        assert!(r.is_err());
        assert_eq!(slots.get_or_fill(&1, || 2), 2);
    }
}
