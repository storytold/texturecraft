use std::any::Any;
#[cfg(not(target_arch = "wasm32"))]
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::CookTrace;
#[cfg(not(target_arch = "wasm32"))]
use super::lock;

pub(super) struct CookProfiler {
    emit: CookTrace,
    graph: String,
    #[cfg(not(target_arch = "wasm32"))]
    timings: Arc<Mutex<Timings>>,
}

impl CookProfiler {
    pub(super) fn new(emit: CookTrace, graph: String) -> Self {
        Self {
            emit,
            graph,
            #[cfg(not(target_arch = "wasm32"))]
            timings: Arc::default(),
        }
    }

    fn accelerated(&self, id: u32, kind: &str, resident: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(node) = lock(&self.timings).active.last_mut().filter(|node| node.id == id && node.kind == kind) {
            node.gpu = true;
        }
        let backend = if resident { "GPU-resident" } else { "GPU" };
        (self.emit)(format!("{backend} result for node #{id} {kind} in {:?}", self.graph));
    }
}

impl texturecraft_graph::profile::Profiler for CookProfiler {
    fn enabled(&self) -> bool {
        false
    }

    fn span(&self, _: &str) -> Box<dyn Any + Send> {
        Box::new(())
    }

    fn accelerated_node(&self, id: u32, kind: &str) {
        self.accelerated(id, kind, false);
    }

    fn resident_node(&self, id: u32, kind: &str) {
        self.accelerated(id, kind, true);
    }

    fn resident_readback(&self, id: u32, bytes: usize) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut timings = lock(&self.timings);
            timings.readbacks += 1;
            timings.readback_bytes = timings.readback_bytes.saturating_add(bytes);
        }
        (self.emit)(format!("GPU graph readback for node #{id} in {:?}: {bytes} bytes", self.graph));
    }

    fn node(&self, id: u32, kind: &str, (width, height): (usize, usize)) -> Option<Box<dyn Any + Send>> {
        let description = format!("#{id} {kind} ({width}×{height}) in {:?}", self.graph);
        (self.emit)(format!("Node {description}"));
        #[cfg(not(target_arch = "wasm32"))]
        {
            lock(&self.timings).active.push(ActiveNode { id, kind: kind.into(), gpu: false, children: Duration::ZERO });
            Some(Box::new(NodeTime { emit: self.emit.clone(), description, started: Instant::now(), timings: self.timings.clone() }))
        }
        #[cfg(target_arch = "wasm32")]
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for CookProfiler {
    fn drop(&mut self) {
        let timings = lock(&self.timings);
        if timings.cpu.is_empty() && timings.gpu_count == 0 {
            return;
        }
        let mut cpu: Vec<_> = timings.cpu.iter().collect();
        cpu.sort_unstable_by_key(|(_, (_, time))| std::cmp::Reverse(*time));
        let cpu_count: usize = cpu.iter().map(|(_, (count, _))| count).sum();
        let cpu_time: Duration = cpu.iter().map(|(_, (_, time))| *time).sum();
        let hottest = cpu.iter().take(6).map(|(kind, (count, time))| format!("{kind} {}ms ({count})", time.as_millis())).collect::<Vec<_>>().join(", ");
        (self.emit)(format!(
            "Graph {:?} backend totals: CPU {cpu_count} nodes, {} ms; GPU path {} nodes, {} ms including host work; {} readbacks, {} bytes; CPU by kind: {hottest}",
            self.graph,
            cpu_time.as_millis(),
            timings.gpu_count,
            timings.gpu_time.as_millis(),
            timings.readbacks,
            timings.readback_bytes,
        ));
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Timings {
    active: Vec<ActiveNode>,
    cpu: BTreeMap<String, (usize, Duration)>,
    gpu_count: usize,
    gpu_time: Duration,
    readbacks: usize,
    readback_bytes: usize,
}

#[cfg(not(target_arch = "wasm32"))]
struct ActiveNode {
    id: u32,
    kind: String,
    gpu: bool,
    children: Duration,
}

#[cfg(not(target_arch = "wasm32"))]
struct NodeTime {
    emit: CookTrace,
    description: String,
    started: Instant,
    timings: Arc<Mutex<Timings>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for NodeTime {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        let gpu = {
            let mut timings = lock(&self.timings);
            let Some(node) = timings.active.pop() else { return };
            if let Some(parent) = timings.active.last_mut() {
                parent.children += elapsed;
            }
            let own_time = elapsed.saturating_sub(node.children);
            if node.gpu {
                timings.gpu_count += 1;
                timings.gpu_time += own_time;
            } else {
                let (count, time) = timings.cpu.entry(node.kind).or_default();
                *count += 1;
                *time += own_time;
            }
            node.gpu
        };
        if elapsed.as_millis() >= 100 {
            let backend = if gpu { "GPU" } else { "CPU" };
            (self.emit)(format!("Slow node {}: {} ms ({backend} path)", self.description, elapsed.as_millis()));
        }
    }
}
