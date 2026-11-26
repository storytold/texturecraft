//! Progress inside a long step: work whose end is only estimated (graph cooks, laid strokes) is watched from a second
//! thread, which reports how far it has come while the step's own thread works.

/// How often a watched step reports.
pub const WATCH_PERIOD: std::time::Duration = std::time::Duration::from_millis(250);

/// Runs `work` on this thread while `poll` is called every `period` on another one (and once more when `work` is done),
/// so that a step whose work cannot report by itself still shows progress. Where no thread can be started (the web
/// build) `work` just runs.
pub fn watching<R>(period: std::time::Duration, poll: &(dyn Fn() + Sync), work: impl FnOnce() -> R) -> R {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let done = std::sync::atomic::AtomicBool::new(false);
        let out = std::thread::scope(|scope| {
            let done = &done;
            let watcher = std::thread::Builder::new().name("texturecraft-import-progress".into()).spawn_scoped(scope, move || {
                while !done.load(std::sync::atomic::Ordering::Acquire) {
                    std::thread::park_timeout(period);
                    poll();
                }
            });
            // Stops the watcher when `work` unwinds too, so the scope can join it
            let _stop = watcher.as_ref().ok().map(|w| StopWatcher { done, watcher: w.thread().clone() });
            work()
        });
        poll();
        out
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = period;
        let out = work();
        poll();
        out
    }
}

/// Tells the watcher thread to finish when dropped.
#[cfg(not(target_arch = "wasm32"))]
struct StopWatcher<'a> {
    done: &'a std::sync::atomic::AtomicBool,
    watcher: std::thread::Thread,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for StopWatcher<'_> {
    fn drop(&mut self) {
        self.done.store(true, std::sync::atomic::Ordering::Release);
        self.watcher.unpark();
    }
}

/// The share of a step done when `done` of an estimated `expected` units are: in proportion up to 80 %, then nearing
/// 100 % without reaching it, so that an estimate too low keeps the bar moving instead of stopping at the end.
pub fn share(done: f32, expected: f32) -> f32 {
    const LINEAR: f32 = 0.8;
    if !(done.is_finite() && expected.is_finite()) || done <= 0.0 {
        return 0.0;
    }
    let r = done / expected.max(1.0);
    if r <= LINEAR { r } else { (LINEAR + (1.0 - LINEAR) * (1.0 - (-(r - LINEAR) / (1.0 - LINEAR)).exp())).min(0.999) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn share_grows_in_proportion_then_nears_one_without_reaching_it() {
        assert_eq!(share(0.0, 10.0), 0.0);
        assert!((share(4.0, 10.0) - 0.4).abs() < 1e-6);
        assert!((share(8.0, 10.0) - 0.8).abs() < 1e-6);
        let mut last = 0.0;
        for i in 0..200 {
            let s = share(i as f32 * 0.25, 10.0);
            assert!(s >= last && s < 1.0, "{i}: {s}");
            last = s;
        }
        assert!(share(30.0, 10.0) > 0.99 && share(1e6, 1.0) < 1.0);
        assert_eq!(share(f32::NAN, 10.0), 0.0);
        assert!(share(5.0, 0.0) < 1.0);
    }

    #[test]
    fn watching_polls_while_the_work_runs_and_once_after_it() {
        let seen = Mutex::new(Vec::new());
        let counter = std::sync::atomic::AtomicUsize::new(0);
        let poll = || seen.lock().unwrap().push(counter.load(std::sync::atomic::Ordering::Relaxed));
        let out = watching(std::time::Duration::from_millis(5), &poll, || {
            for _ in 0..10 {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            42
        });
        assert_eq!(out, 42);
        let seen = seen.into_inner().unwrap();
        assert!(seen.len() >= 3, "polled while the work ran: {seen:?}");
        assert_eq!(seen.last(), Some(&10), "the last poll sees the work finished");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]));
    }
}
