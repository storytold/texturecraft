//! What a finished bake tells: how many maps it made, how long it took and which sets failed.

use crate::i18n::{tr, trf};

/// The tally of the bake running now (the clock is the app's frame time, in seconds)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakeReport {
    pub started_at_seconds: f64,
    /// Maps stored into texture sets so far
    pub map_count: usize,
    /// Number of texture sets whose bake failed
    pub failed_set_count: usize,
}

impl BakeReport {
    pub fn started(now: f64) -> Self {
        BakeReport { started_at_seconds: now, ..Default::default() }
    }

    /// The toast text, the Log line and whether the toast is an error (some set failed)
    pub fn summary(&self, now: f64) -> (String, String, bool) {
        let time = duration_text((now - self.started_at_seconds).max(0.0));
        let line = if self.failed_set_count == 0 {
            format!("Bake finished: {} maps in {time}", self.map_count)
        } else {
            format!("Bake finished: {} maps in {time}, {} failed", self.map_count, self.failed_set_count)
        };
        let text = if self.failed_set_count == 0 {
            trf("Baked {maps} maps in {time}", &[("maps", &self.map_count), ("time", &time)])
        } else {
            let failed = self.failed_set_count;
            format!(
                "{} {}",
                trf("Baked {maps} maps in {time}, {failed} failed.", &[("maps", &self.map_count), ("time", &time), ("failed", &failed)]),
                tr("Failures are listed in the Log.")
            )
        };
        (text, line, self.failed_set_count > 0)
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
        let report = BakeReport { started_at_seconds: 10.0, map_count: 6, failed_set_count: 0 };

        let (text, line, failed) = report.summary(22.34);

        assert_eq!(text, "Baked 6 maps in 12.3 s");
        assert_eq!(line, "Bake finished: 6 maps in 12.3 s");
        assert!(!failed);
    }

    #[test]
    fn failures_are_counted_and_point_to_the_log() {
        let report = BakeReport { started_at_seconds: 0.0, map_count: 3, failed_set_count: 2 };

        let (text, _, failed) = report.summary(125.0);

        assert_eq!(text, "Baked 3 maps in 2 min 5 s, 2 failed. Failures are listed in the Log.");
        assert!(failed);
    }

    #[test]
    fn short_times_show_milliseconds() {
        assert_eq!(duration_text(0.85), "850 ms");
    }
}
