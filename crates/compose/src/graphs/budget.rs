//! The render budget: one limit, shared by every graph render of the process (imports, the app's composites,
//! exports), on the memory renders running at the same time may take. A render waits until the renders already running
//! leave room for its estimated peak ([`estimate`]); a render alone always runs, however large. Nested graphs render
//! inside their parent's share.
//!
//! The limit is a setting ([`set_limit`]); by default half the machine's memory ([`default_limit`]), so that on a
//! 16 GB machine two of the heaviest decompiled mask generators (3–4 GB each) run at once and a third waits.

use std::sync::{Condvar, Mutex, OnceLock, PoisonError};

use texturecraft_graph::graph::Graph;

/// Bytes per pixel of a node image (f32 RGBA).
const PIXEL_BYTES: u64 = 16;
/// Node images alive at a render's peak: a node's two inputs and its output.
const IMAGES_AT_PEAK: u64 = 3;
/// Memory assumed when the machine's cannot be read.
const ASSUMED_MEMORY: u64 = 16 << 30;
/// Smallest limit (a smaller setting is raised to it).
const MIN_LIMIT: u64 = 1 << 30;
/// Largest node side the runtime makes.
const MAX_SIDE: u64 = 8192;

struct Budget {
    /// (limit, bytes taken by running renders, renders running)
    state: Mutex<(u64, u64, usize)>,
    freed: Condvar,
}

fn budget() -> &'static Budget {
    static BUDGET: OnceLock<Budget> = OnceLock::new();
    BUDGET.get_or_init(|| Budget { state: Mutex::new((default_limit(), 0, 0)), freed: Condvar::new() })
}

/// Default share of the machine's memory renders running at once may take, in percent.
pub const DEFAULT_PERCENT: u8 = 50;

/// [`DEFAULT_PERCENT`] of the machine's memory (at least [`MIN_LIMIT`]).
pub fn default_limit() -> u64 {
    limit_for_percent(DEFAULT_PERCENT)
}

/// The machine's memory, where it can be read.
pub fn total_memory() -> Option<u64> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut sys = sysinfo::System::new().ok()?;
        sys.refresh_memory();
        Some(sys.total_memory()).filter(|m| *m > 0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

/// The limit for `percent` (1 to 100) of the machine's memory.
pub fn limit_for_percent(percent: u8) -> u64 {
    (total_memory().unwrap_or(ASSUMED_MEMORY) / 100 * u64::from(percent.clamp(1, 100))).max(MIN_LIMIT)
}

/// Sets the budget's limit in bytes (`None`: the default); renders waiting are let through when it rises.
pub fn set_limit(bytes: Option<u64>) {
    let b = budget();
    let mut s = b.state.lock().unwrap_or_else(PoisonError::into_inner);
    s.0 = bytes.unwrap_or_else(default_limit).max(MIN_LIMIT);
    drop(s);
    b.freed.notify_all();
}

/// The budget's limit in bytes.
pub fn limit() -> u64 {
    budget().state.lock().unwrap_or_else(PoisonError::into_inner).0
}

thread_local! {
    /// This thread runs a render holding a share: a render it starts inside runs within that share (waiting for room
    /// would wait for itself).
    static HOLDING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A running render's share of the budget, given back when dropped.
pub struct Permit {
    bytes: u64,
    /// Whether it took a share (a render inside another's share takes none).
    counted: bool,
}

impl Drop for Permit {
    fn drop(&mut self) {
        if !self.counted {
            return;
        }
        HOLDING.with(|h| h.set(false));
        let b = budget();
        let mut s = b.state.lock().unwrap_or_else(PoisonError::into_inner);
        s.1 = s.1.saturating_sub(self.bytes);
        s.2 = s.2.saturating_sub(1);
        drop(s);
        b.freed.notify_all();
    }
}

/// Waits until a render estimated at `bytes` fits beside the renders running (or none runs), and takes its share.
pub fn acquire(bytes: u64) -> Permit {
    if HOLDING.with(std::cell::Cell::get) {
        return Permit { bytes: 0, counted: false };
    }
    let b = budget();
    let mut s = b.state.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        let share = bytes.min(s.0);
        if s.2 == 0 || s.1.saturating_add(share) <= s.0 {
            s.1 = s.1.saturating_add(share);
            s.2 += 1;
            HOLDING.with(|h| h.set(true));
            return Permit { bytes: share, counted: true };
        }
        s = b.freed.wait(s).unwrap_or_else(PoisonError::into_inner);
    }
}

/// A render's estimated peak at `side`: three node images of its largest node (nodes of a fixed size included, which
/// decompiled graphs have up to 8192²), f32 RGBA.
pub fn estimate(graph: &Graph, side: usize) -> u64 {
    let side = (side as u64).clamp(1, MAX_SIDE);
    let nominal = (graph.size as u64).max(1);
    let axis = |e: f32| 1u64 << (e.round().clamp(0.0, 13.0) as u32);
    let largest = graph
        .nodes
        .values()
        .map(|n| {
            if let Some(x) = n.nums.get("osize_log2_x") {
                return axis(*x) * axis(n.nums.get("osize_log2_y").copied().unwrap_or(*x));
            }
            let mode = n.nums.get("osize_mode").copied().unwrap_or(0.0).round() as i32;
            let rel = n.nums.get("osize_rel").copied().unwrap_or(0.0).round().clamp(-12.0, 12.0) as i32;
            let w = match mode {
                2 => (axis(n.nums.get("osize_abs").copied().unwrap_or(9.0).clamp(4.0, 13.0)) * side / nominal).clamp(1, MAX_SIDE),
                _ if rel > 0 => (side << rel.min(13)).min(MAX_SIDE),
                _ => side,
            };
            w * w
        })
        .fold(side * side, u64::max);
    largest * PIXEL_BYTES * IMAGES_AT_PEAK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_follow_the_largest_node() {
        let g = Graph::parse("size 64\nnode 1 color r=1 g=1 b=1\noutput o 1\n").unwrap();
        assert_eq!(estimate(&g, 1024), 1024 * 1024 * 16 * 3);
        let g = Graph::parse("size 64\nnode 1 color r=1 g=1 b=1\nnode 2 levels_mid in=@1 osize_log2_x=13 osize_log2_y=13\noutput o 2\n").unwrap();
        assert_eq!(estimate(&g, 1024), 8192 * 8192 * 16 * 3);
    }

    #[test]
    fn renders_wait_for_room_and_a_lone_render_always_runs() {
        // (the budget is process-wide: this test is the only one changing it)
        set_limit(Some(4 << 30));
        let big = acquire(16 << 30);
        assert_eq!(big.bytes, 4 << 30, "a render alone runs, its share capped at the limit");
        let waiting = std::thread::spawn(|| {
            let p = acquire(2 << 30);
            p.bytes
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!waiting.is_finished(), "no room beside the big render");
        drop(big);
        assert_eq!(waiting.join().unwrap(), 2 << 30);
        // a render started inside another's share takes none
        let a = acquire(2 << 30);
        let inner = acquire(2 << 30);
        assert!(!inner.counted && a.counted);
        drop((inner, a));
        set_limit(None);
        assert!(limit() >= MIN_LIMIT);
    }
}
