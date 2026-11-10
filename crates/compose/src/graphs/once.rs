//! Filling a shared slot from threads that run parallel work, without ever waiting where waiting could hang.
//!
//! A rayon thread that waits for parallel work runs other pending work meanwhile. So a thread can hold, further down
//! its stack, a piece of the work that fills a slot (or be filling it itself) when it asks for that slot: waiting there
//! would never end. Which thread holds what is not known, so the rule is: **a pool thread never waits for a slot
//! another fill has claimed; it computes its own copy.** Threads outside any pool wait as usual.
//!
//! Graph renders run their parallel work on the runtime's own pool ([`super::GraphStore`]'s), never on the pool the
//! compositor runs on, so no thread of the compositor's pool can hold a piece of a render: others may wait for a
//! render safely ([`Slots::get_or_fill_own_pool`]). Only the claiming thread itself (which runs other work of its own
//! pool while it waits for the render's pool) gets its own copy.
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
