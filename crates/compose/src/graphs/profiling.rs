use std::any::Any;
#[cfg(not(target_arch = "wasm32"))]
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use texturecraft_core::GraphRef;
use texturecraft_graph::graph::Node;

use super::CookTrace;
#[cfg(not(target_arch = "wasm32"))]
use super::lock;

/// CPU work observed during one imported graph evaluation with an accelerator available.
#[derive(Debug, serde::Serialize)]
pub struct GraphCpuReport {
    pub graph: String,
    pub graph_key: String,
    pub source_package: String,
    pub resolution: usize,
    pub settings_key: String,
    pub nodes: Vec<GraphCpuNode>,
}

#[derive(Debug, serde::Serialize)]
pub struct GraphCpuNode {
    pub id: u32,
    pub kind: String,
    pub size: [usize; 2],
    pub variant: String,
    pub self_time_us: u64,
}

pub(super) struct CookProfiler {
    emit: CookTrace,
    graph: String,
    #[cfg(not(target_arch = "wasm32"))]
    timings: Arc<Mutex<Timings>>,
    #[cfg(not(target_arch = "wasm32"))]
    cpu_report: Option<GraphCpuReport>,
}

impl CookProfiler {
    pub(super) fn new(emit: CookTrace, graph: String, source: &GraphRef, resolution: usize, gpu_available: bool) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let cpu_report = source.source_package.as_ref().filter(|_| gpu_available && emit.cpu_report.is_some()).map(|package| GraphCpuReport {
            graph: graph.clone(),
            graph_key: source.key.clone(),
            source_package: super::trace_name(package).to_string(),
            resolution,
            settings_key: texturecraft_core::text_key(&format!("{:?}/{}", source.params, source.seed)),
            nodes: Vec::new(),
        });
        #[cfg(target_arch = "wasm32")]
        let _ = (source, resolution, gpu_available);
        Self {
            emit,
            graph,
            #[cfg(not(target_arch = "wasm32"))]
            timings: Arc::default(),
            #[cfg(not(target_arch = "wasm32"))]
            cpu_report,
        }
    }

    fn accelerated(&self, id: u32, kind: &str, resident: bool) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(node) = lock(&self.timings).active.last_mut().filter(|node| node.id == id && node.kind == kind) {
            node.gpu = true;
        }
        let backend = if resident { "GPU-resident" } else { "GPU" };
        (self.emit.line)(format!("{backend} result for node #{id} {kind} in {:?}", self.graph));
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
        (self.emit.line)(format!("GPU graph readback for node #{id} in {:?}: {bytes} bytes", self.graph));
    }

    fn node_variant(&self, id: u32, node: &Node, (width, height): (usize, usize)) -> Option<Box<dyn Any + Send>> {
        let kind = node.kind.as_str();
        let description = format!("#{id} {kind} ({width}×{height}) in {:?}", self.graph);
        (self.emit.line)(format!("Node {description}"));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let variant = (self.cpu_report.is_some() && !matches!(kind, "input" | "input_value" | "bitmap" | "output" | "dot")).then(|| variant_key(node));
            lock(&self.timings).active.push(ActiveNode { id, kind: kind.into(), size: [width, height], variant, gpu: false, children: Duration::ZERO });
            Some(Box::new(NodeTime { emit: self.emit.line.clone(), description, started: Instant::now(), timings: self.timings.clone() }))
        }
        #[cfg(target_arch = "wasm32")]
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn variant_key(node: &Node) -> String {
    let mut hash = texturecraft_core::StableHash::new();
    hash.str(&node.kind);
    for (name, &value) in &node.nums {
        hash.str(name).f32(value);
    }
    for (name, value) in &node.strs {
        hash.str(name).str(value);
    }
    for (name, &source) in &node.ins {
        hash.str(name).u64(u64::from(source));
        hash.str(node.in_out.get(name).map_or("", String::as_str));
    }
    format!("{:032x}", hash.finish())
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for CookProfiler {
    fn drop(&mut self) {
        let mut timings = lock(&self.timings);
        if timings.cpu.is_empty() && timings.gpu_count == 0 {
            return;
        }
        let mut cpu: Vec<_> = timings.cpu.iter().collect();
        cpu.sort_unstable_by_key(|(_, (_, time))| std::cmp::Reverse(*time));
        let cpu_count: usize = cpu.iter().map(|(_, (count, _))| count).sum();
        let cpu_time: Duration = cpu.iter().map(|(_, (_, time))| *time).sum();
        let hottest = cpu.iter().take(6).map(|(kind, (count, time))| format!("{kind} {}ms ({count})", time.as_millis())).collect::<Vec<_>>().join(", ");
        (self.emit.line)(format!(
            "Graph {:?} backend totals: CPU {cpu_count} nodes, {} ms; GPU path {} nodes, {} ms including host work; {} readbacks, {} bytes; CPU by kind: {hottest}",
            self.graph,
            cpu_time.as_millis(),
            timings.gpu_count,
            timings.gpu_time.as_millis(),
            timings.readbacks,
            timings.readback_bytes,
        ));
        if let Some(mut report) = self.cpu_report.take()
            && let Some(emit) = &self.emit.cpu_report
            && !timings.cpu_nodes.is_empty()
        {
            report.nodes = std::mem::take(&mut timings.cpu_nodes);
            drop(timings);
            emit(report);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Timings {
    active: Vec<ActiveNode>,
    cpu: BTreeMap<String, (usize, Duration)>,
    cpu_nodes: Vec<GraphCpuNode>,
    gpu_count: usize,
    gpu_time: Duration,
    readbacks: usize,
    readback_bytes: usize,
}

#[cfg(not(target_arch = "wasm32"))]
struct ActiveNode {
    id: u32,
    kind: String,
    size: [usize; 2],
    variant: Option<String>,
    gpu: bool,
    children: Duration,
}

#[cfg(not(target_arch = "wasm32"))]
struct NodeTime {
    emit: Arc<dyn Fn(String) + Send + Sync>,
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
                let (count, time) = timings.cpu.entry(node.kind.clone()).or_default();
                *count += 1;
                *time += own_time;
                if let Some(variant) = node.variant {
                    timings.cpu_nodes.push(GraphCpuNode {
                        id: node.id,
                        kind: node.kind,
                        size: node.size,
                        variant,
                        self_time_us: u64::try_from(own_time.as_micros()).unwrap_or(u64::MAX),
                    });
                }
            }
            node.gpu
        };
        if elapsed.as_millis() >= 100 {
            let backend = if gpu { "GPU" } else { "CPU" };
            (self.emit)(format!("Slow node {}: {} ms ({backend} path)", self.description, elapsed.as_millis()));
        }
    }
}
