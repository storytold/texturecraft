//! `texturecraft-cli graph`: renders a graph text file to PNGs with the graph runtime, on the CPU.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use texturecraft_graph::context::{Cancel, EvalContext, Files};
use texturecraft_graph::graph::Graph;
use texturecraft_graph::png;

use crate::{Args, R, s};

/// Most `--param` flags one call takes.
const MAX_PARAMS: usize = 4096;

/// `name=v` or `name=v,v,...` as the name and its numbers.
fn param(text: &str) -> Result<(&str, Vec<f32>), String> {
    let (name, values) = text.rsplit_once('=').ok_or_else(|| format!("--param `{text}`: want <name>=<value>"))?;
    let values =
        values.split(',').map(|v| v.trim().parse::<f32>().map_err(|_| format!("--param `{text}`: `{v}` is not a number"))).collect::<Result<Vec<_>, _>>()?;
    Ok((name, values))
}

/// Output names as file names: anything but letters, digits, `-` and `_` becomes `_`.
fn file_stem(name: &str) -> String {
    let stem: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if stem.is_empty() { "output".to_string() } else { stem }
}

pub fn render(a: &Args) -> R {
    let (input, dir) = (a.pos(0, "<graph.tcgraph>")?, a.pos(1, "<out dir>")?);
    let size = a.num::<usize>("--size")?;
    if size.is_some_and(|s| s == 0 || s > 8192) {
        return Err("--size must be 1-8192".into());
    }
    let depth = match a.get("--depth") {
        None | Some("8") => 8,
        Some("16") => 16,
        Some(d) => return Err(format!("--depth must be 8 or 16, not `{d}`")),
    };
    let timeout = a.num::<f64>("--timeout")?;
    if timeout.is_some_and(|t| !t.is_finite() || t <= 0.0) {
        return Err("--timeout must be a positive number of seconds".into());
    }
    let params = a.all("--param");
    if params.len() > MAX_PARAMS {
        return Err(format!("at most {MAX_PARAMS} --param flags"));
    }

    // read as the runtime's own loader does: live links resolved next to the file, legacy outputs as output nodes
    let (mut g, issues) = Graph::load_file(Path::new(input)).map_err(|e| format!("{input}: {e}"))?;
    for i in &issues {
        eprintln!("warning: {i}");
    }
    for p in params {
        let (name, values) = param(p)?;
        g.set_param(name, &values)?;
    }
    if let Some(seed) = a.num::<u32>("--seed")? {
        g.seed_offset = seed;
    }
    let outputs = a.all("--output");
    if !outputs.is_empty() {
        g.keep_outputs(&outputs)?;
    }
    g.validate().map_err(s)?;

    let cancel = Cancel::new();
    let ec = EvalContext { cancel: cancel.clone(), files: Files::disk(), ..EvalContext::default() };
    // the time limit is the host's: a timer sets the cancel flag, the runtime never reads a clock
    let (done, stop) = mpsc::channel::<()>();
    let timer = timeout.map(|t| {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            if let Err(mpsc::RecvTimeoutError::Timeout) = stop.recv_timeout(Duration::from_secs_f64(t)) {
                cancel.cancel();
            }
        })
    });
    let t0 = std::time::Instant::now();
    let result = g.evaluate_in(&ec, size, &[]);
    drop(done);
    if let Some(t) = timer {
        let _ = t.join();
    }
    let out = result
        .map_err(|e| if cancel.is_cancelled() { format!("cancelled after the --timeout of {} s", timeout.unwrap_or_default()) } else { e.to_string() })?;
    let secs = t0.elapsed().as_secs_f64();

    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {dir}: {e}"))?;
    for (name, t) in &out {
        let path = Path::new(dir).join(format!("{}.png", file_stem(name)));
        std::fs::write(&path, png::encode(t.w, t.h, &t.px, depth)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!("{}", path.display());
    }
    eprintln!("rendered {} output(s) in {secs:.2} s", out.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_and_file_names() {
        assert_eq!(param("level=0.5").unwrap(), ("level", vec![0.5]));
        assert_eq!(param("a=b=1,2").unwrap(), ("a=b", vec![1.0, 2.0]));
        assert!(param("level").is_err());
        assert!(param("level=x").is_err());
        assert_eq!(file_stem("base color/1"), "base_color_1");
        assert_eq!(file_stem(""), "output");
    }
}
