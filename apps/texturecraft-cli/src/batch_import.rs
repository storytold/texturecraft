//! `texturecraft-cli import <folder> <out folder>`: every project file of the reference app below a folder converted
//! one by one (or a few at a time with `--jobs`), each saved under the same relative path in the out folder. One
//! result line per project as it finishes, a total at the end, and a failing exit code when any project failed.
//!
//! With `--jobs` above 1 the projects run on a small pool that also keeps to a memory budget: converting a project
//! holds about twice its file size in memory, so a job reserves its estimate first and waits until it fits next to
//! the running ones (a job larger than the whole budget runs alone).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Instant;

use serde_json::Value;
use texturecraft_engine::core::PROJECT_EXT as TCRAFT_EXT;
use texturecraft_engine::import_cmds::PROJECT_EXT;

/// Folder depth searched for project files (a guard against links that loop).
const MAX_DEPTH: usize = 16;
/// Memory one project's job is assumed to need beyond twice its file size.
const BASE_BYTES: u64 = 256 << 20;
/// Most jobs at once (`--jobs`): each conversion already runs on several threads.
pub const MAX_JOBS: usize = 16;

/// The project files below `dir` (links to files included), sorted, at most [`MAX_DEPTH`] folders down; folders that
/// cannot be listed are skipped.
pub fn project_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(list) = std::fs::read_dir(&d) else { continue };
        for e in list.flatten() {
            let path = e.path();
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() && depth < MAX_DEPTH {
                stack.push((path, depth + 1));
            // a link to a file counts as the file; links to folders are not followed (they can loop)
            } else if !kind.is_dir() && path.is_file() && path.extension().is_some_and(|x| x.eq_ignore_ascii_case(PROJECT_EXT)) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Where the project `file` found below `dir` is saved below `out`: the same relative path, as a TextureCraft project.
pub fn out_path(dir: &Path, out: &Path, file: &Path) -> PathBuf {
    let relative = file.strip_prefix(dir).unwrap_or(file);
    out.join(relative).with_extension(TCRAFT_EXT)
}

/// Bytes reserved by running jobs; [`Budget::reserve`] waits until a new job fits (or nothing else runs).
pub struct Budget {
    limit: u64,
    used: Mutex<u64>,
    freed: Condvar,
}

impl Budget {
    pub fn new(limit: u64) -> Budget {
        Budget { limit, used: Mutex::new(0), freed: Condvar::new() }
    }

    /// Waits until `bytes` fit; returns what was reserved (never more than the whole budget).
    pub fn reserve(&self, bytes: u64) -> u64 {
        let want = bytes.min(self.limit);
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        while *used > 0 && used.saturating_add(want) > self.limit {
            used = self.freed.wait(used).unwrap_or_else(PoisonError::into_inner);
        }
        *used = used.saturating_add(want);
        want
    }

    pub fn release(&self, bytes: u64) {
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        *used = used.saturating_sub(bytes);
        self.freed.notify_all();
    }
}

/// The memory a project's conversion is assumed to need: twice its file size plus a base.
pub fn estimate(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len()).saturating_mul(2).saturating_add(BASE_BYTES)
}

/// 60 % of the memory available now.
pub fn default_budget() -> u64 {
    let available = sysinfo::System::new()
        .map(|mut sys| {
            sys.refresh_memory();
            match sys.available_memory() {
                0 => sys.total_memory(),
                n => n,
            }
        })
        .unwrap_or(8 << 30);
    available / 10 * 6
}

/// Runs `work` on every item on up to `jobs` threads, each holding its item's `weight` of `budget` while it runs;
/// `done` is called with each result as it comes (in finishing order). Results come back in the order of `items`.
pub fn run_pool<T: Sync, R: Send>(
    items: &[T],
    jobs: usize,
    budget: &Budget,
    weight: impl Fn(&T) -> u64 + Sync,
    work: impl Fn(&T) -> R + Sync,
    done: impl Fn(&T, &R) + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|scope| {
        for _ in 0..jobs.clamp(1, MAX_JOBS).min(items.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else { break };
                    let reserved = budget.reserve(weight(item));
                    let result = work(item);
                    budget.release(reserved);
                    done(item, &result);
                    results.lock().unwrap_or_else(PoisonError::into_inner).push((index, result));
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_else(PoisonError::into_inner);
    results.sort_by_key(|(i, _)| *i);
    results.into_iter().map(|(_, r)| r).collect()
}

/// One project's result: the saved file and the seconds it took, or the error.
pub struct Converted {
    pub out: PathBuf,
    pub seconds: f64,
    pub result: Result<Value, String>,
}

/// Converts every project below `dir` into `out` with `convert` (file, out path → the import's summary), printing a
/// line per project. Err when there is nothing to convert or any project failed.
pub fn convert_folder(dir: &Path, out: &Path, jobs: usize, convert: impl Fn(&Path, &Path) -> Result<Value, String> + Sync) -> Result<(), String> {
    let files = project_files(dir);
    if files.is_empty() {
        return Err(format!("no .{PROJECT_EXT} project files below {}", dir.display()));
    }
    let budget = Budget::new(default_budget());
    let started = Instant::now();
    eprintln!("converting {} projects, {} at a time", files.len(), jobs.clamp(1, MAX_JOBS).min(files.len()));
    let results = run_pool(
        &files,
        jobs,
        &budget,
        |f| estimate(f),
        |f| {
            let target = out_path(dir, out, f);
            let t = Instant::now();
            let result = target
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .map_err(|e| format!("cannot make the folder for {}: {e}", target.display()))
                .and_then(|()| convert(f, &target));
            Converted { out: target, seconds: t.elapsed().as_secs_f64(), result }
        },
        |f, c| println!("{}", line(f, c)),
    );
    let failed = results.iter().filter(|c| c.result.is_err()).count();
    println!("{} converted, {failed} failed, {:.1} s", results.len() - failed, started.elapsed().as_secs_f64());
    if failed == 0 { Ok(()) } else { Err(format!("{failed} of {} projects failed", results.len())) }
}

/// `ok  12.3 s  <file> -> <out>  (<sets> texture sets, <locked> locked layers)` or `FAILED  …  <error>`.
fn line(file: &Path, c: &Converted) -> String {
    match &c.result {
        Ok(summary) => {
            let sets = summary.get("sets").and_then(Value::as_array);
            let locked: u64 = sets.into_iter().flatten().filter_map(|s| s.get("locked").and_then(Value::as_u64)).sum();
            format!(
                "ok      {:7.1} s  {} -> {}  ({} texture sets, {locked} locked layers)",
                c.seconds,
                file.display(),
                c.out.display(),
                sets.map_or(0, Vec::len)
            )
        }
        Err(e) => format!("FAILED  {:7.1} s  {}  {}", c.seconds, file.display(), e.replace('\n', " ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use std::time::Duration;

    fn folder(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tc-batch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        d
    }

    #[test]
    fn project_files_are_found_below_the_folder_and_saved_alongside() {
        let d = folder("find");
        let upper = PROJECT_EXT.to_ascii_uppercase();
        for f in
            [format!("one.{upper}"), format!("a/two.{PROJECT_EXT}"), format!("a/b/three.{PROJECT_EXT}"), "a/notes.txt".into(), format!("a/b/x.{TCRAFT_EXT}")]
        {
            std::fs::write(d.join(f), b"").unwrap();
        }
        let found: Vec<PathBuf> = project_files(&d).iter().map(|p| p.strip_prefix(&d).unwrap().to_path_buf()).collect();
        let want: Vec<PathBuf> =
            [format!("a/b/three.{PROJECT_EXT}"), format!("a/two.{PROJECT_EXT}"), format!("one.{upper}")].iter().map(PathBuf::from).collect();
        assert_eq!(found, want);
        let out = out_path(&d, Path::new("/out"), &d.join(&want[0]));
        assert_eq!(out, Path::new("/out/a/b/three").with_extension(TCRAFT_EXT));
        assert!(project_files(&d.join("nothing here")).is_empty());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.join(&want[1]), d.join(format!("linked.{PROJECT_EXT}"))).unwrap();
            std::os::unix::fs::symlink(&d, d.join("a/loop")).unwrap();
            assert_eq!(project_files(&d).len(), 4, "a linked file counts, a linked folder is not walked");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn results_keep_input_order() {
        let items: Vec<u64> = (0..40).collect();
        let out = run_pool(
            &items,
            8,
            &Budget::new(u64::MAX),
            |_| 1,
            |&n| {
                std::thread::sleep(Duration::from_micros((40 - n) * 50));
                n * 3
            },
            |_, _| {},
        );
        assert_eq!(out, items.iter().map(|n| n * 3).collect::<Vec<_>>());
    }

    #[test]
    fn running_jobs_stay_within_the_budget_and_a_huge_one_runs_alone() {
        let (running, peak) = (AtomicU64::new(0), AtomicU64::new(0));
        let items: Vec<u64> = vec![4, 4, 4, 3, 3, 3, 2, 2, 25, 1, 1, 1];
        run_pool(
            &items,
            6,
            &Budget::new(10),
            |&w| w,
            |&w| {
                let w = w.min(10);
                let now = running.fetch_add(w, Ordering::SeqCst) + w;
                peak.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(5));
                running.fetch_sub(w, Ordering::SeqCst);
            },
            |_, _| {},
        );
        assert!(peak.load(Ordering::SeqCst) <= 10, "{}", peak.load(Ordering::SeqCst));
    }

    #[test]
    fn a_failure_is_reported_and_fails_the_run_but_stops_nothing() {
        let d = folder("fail");
        let names = ["a", "b", "c"].map(|n| format!("{n}.{PROJECT_EXT}"));
        for n in &names {
            std::fs::write(d.join(n), b"").unwrap();
        }
        let out = d.join("out");
        let seen = Mutex::new(Vec::new());
        let r = convert_folder(&d, &out, 2, |f, target| {
            seen.lock().unwrap().push(f.to_path_buf());
            if f.ends_with(&names[1]) { Err("broken".into()) } else { Ok(serde_json::json!({"sets": [{"locked": 2}]})) }
                .inspect(|_| assert!(target.parent().unwrap().is_dir()))
        });
        assert!(r.is_err_and(|e| e.contains("1 of 3")));
        assert_eq!(seen.lock().unwrap().len(), 3, "the others still ran");
        assert!(convert_folder(&d.join("out"), &out, 1, |_, _| Ok(Value::Null)).is_err(), "no projects is an error");
        let ok = Converted { out: "/o/x.tcraft".into(), seconds: 1.0, result: Ok(serde_json::json!({"sets": [{"locked": 2}, {"locked": 1}]})) };
        assert!(line(Path::new("/i/x"), &ok).contains("(2 texture sets, 3 locked layers)"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
