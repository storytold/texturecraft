use std::time::{Duration, Instant};

use serde_json::{Value, json};

#[path = "row.rs"]
mod row;

use row::Row;

pub(super) fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn scale() -> f64 {
    std::env::var("TC_PERF_SCALE").ok().and_then(|value| value.parse::<f64>().ok()).filter(|value| value.is_finite() && *value > 0.0).unwrap_or(1.0)
}

pub(super) struct Report {
    pub(super) rows: Vec<Row>,
    test: &'static str,
}

impl Report {
    pub(super) fn new(test: &'static str) -> Self {
        println!("\n=== {test} ({} threads, {} build)", rayon::current_num_threads(), if cfg!(debug_assertions) { "debug" } else { "release" });

        Self { test, rows: Vec::new() }
    }

    /// Record a stage; `budget_ms` is scaled by `TC_PERF_SCALE`.
    pub(super) fn add(&mut self, stage: impl Into<String>, took: Duration, budget_ms: Option<f64>, note: impl Into<String>) {
        let row = Row { stage: stage.into(), took, budget: budget_ms.map(|budget| Duration::from_secs_f64(budget * scale() / 1000.0)), note: note.into() };
        let budget = row.budget.map_or(String::new(), |limit| format!(" / {:>9.1} ms{}", ms(limit), if row.took > limit { "  OVER" } else { "" }));
        println!("  {:<58} {:>9.1} ms{budget}  {}", row.stage, ms(row.took), row.note);

        self.rows.push(row);
    }

    /// Time `action` as one stage.
    pub(super) fn time<T>(&mut self, stage: impl Into<String>, budget_ms: Option<f64>, action: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let result = action();
        self.add(stage, start.elapsed(), budget_ms, "");

        result
    }

    /// Write the JSON report and fail on blown budgets (release builds only).
    pub(super) fn finish(self) {
        if let Ok(output_dir) = std::env::var("TC_PERF_OUT") {
            let rows: Vec<Value> =
                self.rows.iter().map(|row| json!({"stage": row.stage, "ms": ms(row.took), "budgetMs": row.budget.map(ms), "note": row.note})).collect();
            let document = json!({"test": self.test, "threads": rayon::current_num_threads(), "release": !cfg!(debug_assertions), "stages": rows});

            // Resolve relative output paths from the workspace root
            let output_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(output_dir);
            std::fs::create_dir_all(&output_dir).unwrap();
            let path = output_dir.join(format!("{}.json", self.test));
            std::fs::write(&path, serde_json::to_string_pretty(&document).unwrap()).unwrap();
            println!("  report: {}", path.display());
        }

        let over: Vec<String> = self
            .rows
            .iter()
            .filter_map(|row| row.budget.filter(|limit| row.took > *limit).map(|limit| format!("{}: {:.1} ms > {:.1} ms", row.stage, ms(row.took), ms(limit))))
            .collect();
        if over.is_empty() {
            return;
        }

        if cfg!(debug_assertions) || std::env::var("TC_PERF_BUDGETS").is_ok_and(|value| value == "off") {
            println!("  ({} stages over budget; not failing: {})", over.len(), if cfg!(debug_assertions) { "debug build" } else { "TC_PERF_BUDGETS=off" });
            return;
        }

        panic!("{}: {} stages over budget:\n  {}", self.test, over.len(), over.join("\n  "));
    }
}
