//! What a finished bake tells: how many maps it made, how long it took and which sets failed.

use crate::i18n::{tr, trf};

/// The tally of the bake running now (the clock is the app's frame time, in seconds)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakeReport {
    pub started: f64,
    /// Maps stored into texture sets so far
    pub maps: usize,
    /// Texture sets whose bake failed
    pub failed: Vec<usize>,
}

impl BakeReport {
    pub fn started(now: f64) -> Self {
        BakeReport { started: now, ..Default::default() }
    }

    /// The toast text, the Log line and whether the toast is an error (some set failed)
    pub fn summary(&self, now: f64) -> (String, String, bool) {
        let time = duration_text((now - self.started).max(0.0));
        let line = if self.failed.is_empty() {
            format!("Bake finished: {} maps in {time}", self.maps)
        } else {
            format!("Bake finished: {} maps in {time}, {} failed", self.maps, self.failed.len())
        };
        let text = if self.failed.is_empty() {
            trf("Baked {maps} maps in {time}", &[("maps", &self.maps), ("time", &time)])
        } else {
            let failed = self.failed.len();
            format!(
                "{} {}",
                trf("Baked {maps} maps in {time}, {failed} failed.", &[("maps", &self.maps), ("time", &time), ("failed", &failed)]),
                tr("Failures are listed in the Log.")
            )
        };
        (text, line, !self.failed.is_empty())
    }
}

/// "850 ms", "12.3 s" or "2 min 5 s"
fn duration_text(seconds: f64) -> String {
    if seconds < 1.0 {
        format!("{} ms", (seconds * 1000.0).round() as u32)
    } else if seconds < 60.0 {
        format!("{seconds:.1} s")
    } else {
        let whole = seconds.round() as u64;
        format!("{} min {} s", whole / 60, whole % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_names_maps_and_time() {
        let report = BakeReport { started: 10.0, maps: 6, failed: vec![] };
        let (text, line, failed) = report.summary(22.34);
        assert_eq!(text, "Baked 6 maps in 12.3 s");
        assert_eq!(line, "Bake finished: 6 maps in 12.3 s");
        assert!(!failed);
    }

    #[test]
    fn failures_are_counted_and_point_to_the_log() {
        let report = BakeReport { started: 0.0, maps: 3, failed: vec![1, 2] };
        let (text, _, failed) = report.summary(125.0);
        assert_eq!(text, "Baked 3 maps in 2 min 5 s, 2 failed. Failures are listed in the Log.");
        assert!(failed);
    }

    #[test]
    fn short_times_show_milliseconds() {
        assert_eq!(duration_text(0.85), "850 ms");
    }
}
