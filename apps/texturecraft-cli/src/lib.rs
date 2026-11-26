//! Headless TextureCraft command line: inspect, script, bake, export and render projects, and
//! serve agents over MCP or JSON lines. Never opens a window. `run` is the whole program, so it can
//! be tested in-process as well as through the binary; `texturecraft-cli --help` lists the
//! subcommands. The conventions are RFC-0018's: exit 0, 1 (a command failed) or 2 (a usage error),
//! results on stdout, `error: <message>` on stderr.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod args;
mod error;

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use clap::CommandFactory;
use serde_json::{Value, json};
use texturecraft_automation::security::{client_token, server_token};
use texturecraft_automation::{AuthorizedWorkspace, BridgeClient, Filesystem, Headless, RenderOptions, TexturecraftMcp, rpc};
use texturecraft_engine::formats::{MeshReport, import_mesh, import_mesh_bytes, mesh_report};

use args::{
    BakeArgs, BatchArgs, Cli, Command, CommandsArgs, ExportArgs, InfoArgs, McpArgs, NewArgs, RenderArgs, RootArgs, RunArgs, ServeArgs, is_project, parse,
};
use error::{CliError, unicode};

/// Writes a line. A closed pipe (`texturecraft-cli info x | head`) is not worth an error: the rest
/// of the output is dropped.
macro_rules! outln {
    ($to:expr, $($arg:tt)*) => {
        drop(writeln!($to, $($arg)*))
    };
}

/// The version `--version` prints after the program's name: `<version> (<commit> <date>)`.
fn version() -> String {
    let sha = env!("TEXTURECRAFT_BUILD_SHA");
    let short = sha.get(..7).unwrap_or(sha);
    let date = env!("TEXTURECRAFT_BUILD_DATE");
    match (short.is_empty(), date.is_empty()) {
        (true, true) => texturecraft_engine::VERSION.to_string(),
        (true, false) => format!("{} ({date})", texturecraft_engine::VERSION),
        _ => format!("{} ({short} {date})", texturecraft_engine::VERSION),
    }
}

/// The command line's clap definition.
pub fn command() -> clap::Command {
    Cli::command().version(version())
}

/// The desktop app's Nightly launch options. The CLI never takes them: a Nightly install ships the
/// GUI only, and a script that passes them to the CLI has the wrong program.
fn launch_profile_argument(args: &[OsString]) -> Option<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .find(|arg| ["--launch-profile", "--profile-root"].iter().any(|flag| arg == flag || arg.starts_with(&format!("{flag}="))))
}

/// Run the CLI on its arguments (without the program name), writing results to `out` and
/// diagnostics to `err`. Returns the process exit code: 0 on success, 1 on failure, 2 for a usage
/// error. Help and the version go to `out`, exit 0. `mcp` and `serve` speak on the process's own
/// stdin and stdout.
pub fn run<I, T>(args: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    if let Some(flag) = launch_profile_argument(&args) {
        outln!(err, "error: `{flag}` is a TextureCraft desktop launch option, not a CLI argument");
        return 2;
    }
    // Help and the version (`help`, `-h`, `<subcommand> --help`, `-V`) come back as errors too.
    let cli = match parse(&args) {
        Ok(cli) => cli,
        Err(e) => {
            let to: &mut dyn Write = if e.use_stderr() { err } else { out };
            let _ = write!(to, "{}", e.render());
            return e.exit_code();
        }
    };
    match dispatch(cli.command, out, err) {
        Ok(()) => 0,
        Err(e) => {
            outln!(err, "error: {e}");
            1
        }
    }
}

fn dispatch(command: Command, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), CliError> {
    match command {
        Command::Info(a) => info(&a, out),
        Command::Commands(a) => commands(&a, out),
        Command::Run(a) => run_command(&a, out, err),
        Command::Batch(a) => batch(&a, out, err),
        Command::Bake(a) => bake(&a, out, err),
        Command::Export(a) => export(&a, out, err),
        Command::Render(a) => render(&a, err),
        Command::New(a) => new(&a, err),
        Command::Mcp(a) => mcp(&a),
        Command::Serve(a) => serve(&a, err),
    }
}

fn print(out: &mut dyn Write, v: &Value) {
    outln!(out, "{}", serde_json::to_string_pretty(v).unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}")));
}

/// A trusted local session on `input`: a project, a mesh file or `builtin:<id>`.
fn open(input: &Path, resolution: Option<u32>) -> Result<Headless, CliError> {
    let mut h = Headless::new(Headless::default_session()?, Filesystem::TrustedLocal);
    match input.to_str().and_then(|text| text.strip_prefix("builtin:")) {
        Some(id) => h.command_run("project.new", json!({"mesh": id, "resolution": resolution.unwrap_or(1024), "name": id})).map(|_| ())?,
        None if is_project(input) => h.open(unicode(input)?).map(|_| ())?,
        None => h.import_mesh(unicode(input)?, resolution, None).map(|_| ())?,
    }
    Ok(h)
}

fn save(h: &mut Headless, out: &Path, err: &mut dyn Write) -> Result<(), CliError> {
    let r = h.save(Some(unicode(out)?))?;
    outln!(err, "saved {}", r["path"].as_str().unwrap_or_default());
    Ok(())
}

fn report_json(r: &MeshReport) -> Value {
    json!({
        "triangles": r.triangles, "vertices": r.vertices, "materials": r.materials, "uvsGenerated": r.uvs_generated,
        "uvOutOfRange": r.uv_out_of_range, "overlapRatio": r.overlap_ratio, "zeroAreaUvTriangles": r.zero_area_uv_triangles, "warnings": r.warnings,
    })
}

fn info(a: &InfoArgs, out: &mut dyn Write) -> Result<(), CliError> {
    let file = unicode(&a.input)?;
    if let Some(id) = file.strip_prefix("builtin:") {
        let m = texturecraft_engine::geom::primitives::builtin(id).ok_or_else(|| CliError::UnknownBuiltin(id.to_string()))?;
        let report = mesh_report(&m);
        print(
            out,
            &json!({
                "file": file, "kind": "mesh", "triangles": m.triangle_count(), "vertices": m.vertex_count(),
                "materials": m.materials, "textureSets": m.materials, "uv": report_json(&report),
            }),
        );
        return Ok(());
    }
    if !is_project(&a.input) {
        let m = import_mesh(&a.input).map_err(|e| CliError::Read { path: a.input.clone(), message: e.to_string() })?;
        print(
            out,
            &json!({
                "file": file, "kind": "mesh", "triangles": m.mesh.triangle_count(), "vertices": m.mesh.vertex_count(),
                "materials": m.mesh.materials, "textureSets": m.mesh.materials, "uv": report_json(&m.report),
            }),
        );
        return Ok(());
    }
    let h = open(&a.input, None)?;
    let mut v = h.inspect();
    v["file"] = json!(file);
    v["kind"] = json!("project");
    // The embedded mesh is re-imported for its UV report (built-in meshes have none).
    v["uv"] = match &h.session.mesh_source {
        Some((ext, bytes)) => {
            let m = import_mesh_bytes(&format!("mesh.{ext}"), bytes).map_err(|e| CliError::Read { path: a.input.clone(), message: e.to_string() })?;
            report_json(&m.report)
        }
        None => Value::Null,
    };
    print(out, &v);
    Ok(())
}

fn commands(a: &CommandsArgs, out: &mut dyn Write) -> Result<(), CliError> {
    let h = open(Path::new("builtin:cube"), Some(128))?;
    let list = h.command_list(a.filter.as_deref());
    if a.json {
        print(out, &list);
        return Ok(());
    }
    for c in list.as_array().into_iter().flatten() {
        let f = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        outln!(out, "{:<28} {:<36} {}", f("id"), f("label"), f("params"));
    }
    Ok(())
}

fn run_command(a: &RunArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), CliError> {
    let mut h = open(&a.input, None)?;
    print(out, &h.command_run(&a.id, a.params.clone().unwrap_or_else(|| json!({})))?);
    match &a.output {
        Some(path) => save(&mut h, path, err),
        None => Ok(()),
    }
}

fn batch(a: &BatchArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), CliError> {
    let text = std::fs::read_to_string(&a.script).map_err(|e| CliError::Read { path: a.script.clone(), message: e.to_string() })?;
    let script: Value = serde_json::from_str(&text).map_err(|source| CliError::Script { path: a.script.clone(), source })?;
    let steps = script.as_array().or_else(|| script.get("steps").and_then(Value::as_array)).ok_or(CliError::ScriptShape)?;
    let path_field = |field: &'static str| -> Result<Option<&str>, CliError> {
        match script.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(path)) => Ok(Some(path.as_str())),
            Some(_) => Err(CliError::ScriptField(field)),
        }
    };
    let input = path_field("open")?.unwrap_or("builtin:material_ball");
    let output = path_field("save")?;

    let mut h = open(Path::new(input), None)?;
    let reply = h.batch(steps);
    print(out, &reply);
    if let Some(failure) = reply.get("failed").filter(|failure| !failure.is_null()) {
        return Err(CliError::BatchStopped {
            index: failure["index"].to_string(),
            id: failure["id"].as_str().unwrap_or_default().to_string(),
            message: failure["error"].as_str().unwrap_or_default().to_string(),
        });
    }
    match output {
        Some(path) => save(&mut h, Path::new(path), err),
        None => Ok(()),
    }
}

fn bake(a: &BakeArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), CliError> {
    // `check` has seen to it: a project without -o is saved in place (--in-place), a mesh has -o.
    let target = a.output.as_deref().unwrap_or(&a.input);
    let mut p = json!({"ts": "all"});
    if let Some(r) = a.resolution {
        p["resolution"] = json!(r);
    }
    if !a.maps.is_empty() {
        p["maps"] = json!(a.maps.iter().map(|m| m.trim()).filter(|m| !m.is_empty()).collect::<Vec<_>>());
    }
    let mut h = open(&a.input, None)?;
    h.command_run("bake.run", p)?;
    save(&mut h, target, err)?;
    print(out, &h.inspect()["textureSets"]);
    Ok(())
}

fn export(a: &ExportArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result<(), CliError> {
    let mut p = json!({"dir": unicode(&a.dir)?});
    for (key, value) in [("preset", &a.preset), ("format", &a.format), ("padding", &a.padding)] {
        if let Some(v) = value {
            p[key] = json!(v);
        }
    }
    if let Some(v) = a.size {
        p["size"] = json!(v);
    }
    let mut h = open(&a.input, None)?;
    let r = h.export(&p)?;
    for w in r["warnings"].as_array().into_iter().flatten() {
        outln!(err, "warning: {}", w.as_str().unwrap_or_default());
    }
    for f in r["files"].as_array().into_iter().flatten() {
        outln!(out, "{}", f.as_str().unwrap_or_default());
    }
    Ok(())
}

fn render(a: &RenderArgs, err: &mut dyn Write) -> Result<(), CliError> {
    let options = RenderOptions {
        view: a.view.clone(),
        width: a.size.map(|s| s.width),
        height: a.size.map(|s| s.height),
        yaw: a.yaw,
        pitch: a.pitch,
        samples: a.samples,
        bounces: a.bounces,
        cpu: a.cpu,
    };
    let mut h = open(&a.input, None)?;
    if let Some(name) = &a.camera {
        h.command_run("camera.select", json!({ "name": name }))?;
    }
    let t0 = std::time::Instant::now();
    let r = h.render_png(&options)?;
    let secs = t0.elapsed().as_secs_f64();
    std::fs::write(&a.output, &r.png).map_err(|source| CliError::Write { path: a.output.clone(), source })?;
    outln!(err, "wrote {} ({}x{}, {}, {secs:.2} s)", a.output.display(), r.width, r.height, r.by);
    Ok(())
}

fn new(a: &NewArgs, err: &mut dyn Write) -> Result<(), CliError> {
    if is_project(&a.input) {
        return Err(CliError::AlreadyProject(a.input.clone()));
    }
    let mut h = open(&a.input, a.resolution)?;
    if a.sample {
        let bake_res = h.session.doc.project.texture_sets.first().map_or(1024, |t| t.resolution.min(1024));
        h.command_run("bake.run", json!({"ts": "all", "resolution": bake_res}))?;
        for ts in 0..h.session.set_count() {
            h.command_run("textureSet.select", json!({"ts": ts}))?;
            h.command_run("material.apply", json!({"material": "steel_painted_worn"}))?;
        }
        h.command_run("textureSet.select", json!({"ts": 0}))?;
    }
    save(&mut h, &a.output, err)
}

/// File access for servers: none without roots, else only below them
fn workspace(roots: &RootArgs) -> Result<Filesystem, CliError> {
    if roots.read_root.is_none() && roots.write_root.is_none() {
        return Ok(Filesystem::Denied);
    }
    Ok(Filesystem::Workspace(AuthorizedWorkspace::new(roots.read_root.as_deref(), roots.write_root.as_deref())?))
}

fn mcp(a: &McpArgs) -> Result<(), CliError> {
    let server = match a.bridge {
        Some(port) => TexturecraftMcp::bridge(BridgeClient::new(port, client_token(port, a.token.clone())?)),
        None => TexturecraftMcp::headless(Headless::new(Headless::default_session()?, workspace(&a.roots)?)),
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(CliError::Serve)?;
    rt.block_on(server.serve_stdio()).map_err(|e| CliError::Automation(texturecraft_automation::AutomationError::Other(e.to_string())))
}

fn serve(a: &ServeArgs, err: &mut dyn Write) -> Result<(), CliError> {
    let h = Arc::new(Mutex::new(Headless::new(Headless::default_session()?, workspace(&a.roots)?)));
    match a.port {
        None => rpc::serve_lines(&h, std::io::stdin().lock(), std::io::stdout().lock()).map_err(CliError::Serve)?,
        Some(port) => {
            // Bind first: a port in use must not replace its server's token file.
            let listener = rpc::bind_tcp(port)?;
            let addr = listener.local_addr().map_err(CliError::Serve)?;
            let (token, file) = server_token(addr.port())?;
            match file {
                Some(f) => outln!(err, "texturecraft-cli: serving on {addr}; token in {}", f.display()),
                None => outln!(err, "texturecraft-cli: serving on {addr}; token from TEXTURECRAFT_CONTROL_TOKEN"),
            }
            rpc::serve_listener(listener, h, token)?;
        }
    }
    Ok(())
}
