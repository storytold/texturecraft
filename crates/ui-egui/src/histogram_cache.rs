//! The histograms the levels and compare mask screens show, cached by document revision

use std::sync::Arc;

use egui::{Id, Ui};

/// Buckets of a properties screen's histogram, over 0..1
pub(crate) const HISTOGRAM_BUCKETS: usize = 64;

/// The histogram cached under `key` for `revision`, recomputed by `count` once no button is held; one entry per `owner`, so a hidden effect's is dropped
pub(crate) fn cached_histogram<T: Send + Sync + 'static>(ui: &Ui, owner: &str, key: Id, revision: u64, count: impl FnOnce() -> Option<T>) -> Option<Arc<T>> {
    let owner = ui.make_persistent_id(owner);
    ui.data_mut(|data| {
        let previous = std::mem::replace(data.get_temp_mut_or(owner, key), key);
        if previous != key {
            data.remove::<(u64, Arc<T>)>(previous);
        }
    });

    match ui.data(|data| data.get_temp::<(u64, Arc<T>)>(key)) {
        Some((cached, histogram)) if cached == revision || ui.input(|input| input.pointer.any_down()) => Some(histogram),
        _ => count().map(|histogram| {
            let histogram = Arc::new(histogram);
            ui.data_mut(|data| data.insert_temp(key, (revision, histogram.clone())));
            histogram
        }),
    }
}
