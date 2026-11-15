//! Runs `file.importProject` as the UI does (an import task with progress, then opened) and prints every progress
//! report with the time it came. With `--cancel-in <step id> <ms>` the import is cancelled that long after the step
//! first shows (`reading`, `converting`, `replaying`, `cooking`, `saving`) and the time it took to stop is printed.
//! A line is printed per step change and per 1 % the bar moves. With `--view` the run also composites the converted
//! project's view, as the desktop app's import does.
//!
//! `cargo run --release -p texturecraft-engine --features dev-tools --example import_progress -- <project file> [--cancel-in cooking 2000] [--view]`

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use texturecraft_engine::import_cmds::{ImportProgress, ImportTask, Step};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(file) = args.get(1) else {
        eprintln!("usage: import_progress <project file> [--cancel-in <step id> <ms>]");
        std::process::exit(2);
    };
    let view = args.iter().any(|a| a == "--view");
    let cancel_in = match args.get(2).map(String::as_str) {
        Some("--cancel-in") => match (args.get(3), args.get(4).and_then(|m| m.parse::<u64>().ok())) {
            (Some(step), Some(ms)) => Some((step.clone(), Duration::from_millis(ms))),
            _ => {
                eprintln!("--cancel-in needs a step id and milliseconds");
                std::process::exit(2);
            }
        },
        _ => None,
    };
    let t0 = Instant::now();
    // when each step id first showed
    let first: Arc<Mutex<Vec<(String, Instant)>>> = Arc::default();
    let seen = first.clone();
    // the last step and fraction printed: a line per step change and per 1 % moved
    let last = Mutex::new(None::<(Step, f32)>);
    let progress = Arc::new(ImportProgress::with_listener(move |f, step| {
        let mut last = last.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if last.is_none_or(|(s, at)| s != step || f >= at + 0.01) {
            println!("{:>8.2} s  {:>5.1}%  {step}", t0.elapsed().as_secs_f64(), f * 100.0);
            *last = Some((step, f));
        }
        let mut seen = seen.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !seen.iter().any(|(id, _)| id == step.id()) {
            seen.push((step.id().to_string(), Instant::now()));
        }
    }));
    let cancelled_at: Arc<Mutex<Option<Instant>>> = Arc::default();
    let watcher = cancel_in.map(|(step, after)| {
        let (progress, first, at) = (progress.clone(), first.clone(), cancelled_at.clone());
        std::thread::spawn(move || {
            loop {
                let since = first.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().find(|(id, _)| *id == step).map(|(_, t)| *t);
                if let Some(t) = since
                    && t.elapsed() >= after
                {
                    println!("{:>8.2} s  cancel ({} ms into {step})", t0.elapsed().as_secs_f64(), after.as_millis());
                    *at.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
                    progress.cancel();
                    return;
                }
                if progress.fraction() >= 1.0 {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    });
    let result = ImportTask::path(file.clone()).with_view(view).run(&progress);
    let end = Instant::now();
    if let Some(w) = watcher {
        let _ = w.join();
    }
    match result {
        Ok(out) => {
            let mut session = match texturecraft_engine::sample_session(256, 64) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };
            match session.apply_import(out) {
                Ok(reply) => {
                    let sets = reply.pointer("/import/sets").and_then(|s| s.as_array()).map_or(0, Vec::len);
                    let layers: u64 = reply.pointer("/import/sets").and_then(|s| s.as_array()).into_iter().flatten().filter_map(|s| s["layers"].as_u64()).sum();
                    println!(
                        "{:>8.2} s  opened \"{}\": {sets} texture sets, {layers} layers, {} graph renders, {} tiles left to composite",
                        t0.elapsed().as_secs_f64(),
                        session.project().name,
                        session.doc.graphs.renders(),
                        session.pending_tiles()
                    );
                }
                Err(e) => println!("{:>8.2} s  not opened: {e}", t0.elapsed().as_secs_f64()),
            }
        }
        Err(e) => {
            let stop = cancelled_at.lock().unwrap_or_else(std::sync::PoisonError::into_inner).map(|t| end.duration_since(t));
            println!("{:>8.2} s  failed in step \"{}\": {e}", t0.elapsed().as_secs_f64(), progress.step());
            if let Some(stop) = stop {
                println!("stopped {:.0} ms after the cancel", stop.as_secs_f64() * 1000.0);
            }
        }
    }
}
