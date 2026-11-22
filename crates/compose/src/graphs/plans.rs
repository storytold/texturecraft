//! Reuse the exact same validated material program when another project references it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use texturecraft_graph::graph::PreparedGraph;

use super::{budget, lock};

const MAX_PLANS: usize = 64;

#[derive(Default)]
struct Plans {
    entries: VecDeque<Entry>,
    source_bytes: usize,
}

struct Entry {
    key: String,
    text: Arc<str>,
    plan: Arc<PreparedGraph>,
}

pub(super) fn get(key: &str, text: &Arc<str>) -> Result<Arc<PreparedGraph>, String> {
    static PLANS: OnceLock<Mutex<Plans>> = OnceLock::new();
    let plans = PLANS.get_or_init(Mutex::default);
    if let Some(plan) = lock(plans).get(key, text) {
        return Ok(plan);
    }
    let plan = Arc::new(PreparedGraph::parse(text).map_err(|e| e.to_string())?);
    let mut cache = lock(plans);
    if let Some(existing) = cache.get(key, text) {
        return Ok(existing);
    }
    let budget = (budget::limit() / 256).clamp(4 << 20, 64 << 20) as usize;
    if text.len() <= budget {
        while cache.entries.len() >= MAX_PLANS || cache.source_bytes + text.len() > budget {
            let Some(oldest) = cache.entries.pop_front() else { break };
            cache.source_bytes -= oldest.text.len();
        }
        cache.source_bytes += text.len();
        cache.entries.push_back(Entry { key: key.to_owned(), text: text.clone(), plan: plan.clone() });
    }
    Ok(plan)
}

impl Plans {
    fn get(&mut self, key: &str, text: &Arc<str>) -> Option<Arc<PreparedGraph>> {
        let index = self.entries.iter().position(|entry| entry.key == key && (Arc::ptr_eq(&entry.text, text) || entry.text == *text))?;
        let entry = self.entries.remove(index)?;
        let plan = entry.plan.clone();
        self.entries.push_back(entry);
        Some(plan)
    }
}
