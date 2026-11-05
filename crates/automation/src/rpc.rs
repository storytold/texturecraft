//! Headless JSON-lines server with the app's control-channel envelope:
//! `{"id","method","params"}` → `{"id","ok":true,"result"}` | `{"id","ok":false,"error"}`.
//! `texturecraft-cli serve` runs it on stdio or on a loopback TCP port (first request `auth`).
//!
//! Methods: the app's `ping`, `command.run {id, params?}`, `command.list {filter?}`,
//! `session.inspect`, plus `command.batch {steps}`, `project.open {path}`,
//! `project.save {path?}`, `mesh.import {path, resolution?, name?}`, `mesh.reimport {path?,
//! preserveStrokes?}`, `mesh.export {path, format?}`, `bake.mesh.add {path, cage?}`,
//! `resource.import {path}`, `material.save {path, layer?, name?}`, `material.load {path}`
//! (workspace-scoped versions of the engine commands),
//! `textures.export {dir, …}`, `render {view?, width?, height?, preset?, scale?, transparent?, yaw?,
//! pitch?, samples?, bounces?, cpu?}` (PNG base64, the
//! shape of the app's `viewport.screenshot`) and `methods`.

use std::io::{BufRead, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use serde_json::{Value, json};

use crate::budgets::json_text;
use crate::headless::RenderOptions;
use crate::security::{
    AUTH_TIMEOUT, AuthDeadline, ConnectionLimiter, LineRead, MAX_BATCH_STEPS, MAX_CONNECTIONS, MAX_REQUEST_BYTES, read_bounded_line, token_matches,
};
use crate::{AutomationError, Headless, bad};

pub const METHODS: &[&str] = &[
    "ping",
    "methods",
    "session.inspect",
    "command.list",
    "command.run",
    "command.batch",
    "project.open",
    "project.save",
    "mesh.import",
    "mesh.reimport",
    "mesh.export",
    "bake.mesh.add",
    "resource.import",
    "material.save",
    "material.load",
    "textures.export",
    "render",
];

fn str_of<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}

fn u32_of(p: &Value, k: &str) -> Result<Option<u32>, AutomationError> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().and_then(|v| u32::try_from(v).ok()).map(Some).ok_or_else(|| bad(format!("`{k}` must be a whole number"))),
    }
}

fn f32_of(p: &Value, k: &str) -> Result<Option<f32>, AutomationError> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().map(|v| Some(v as f32)).ok_or_else(|| bad(format!("`{k}` must be a number"))),
    }
}

fn bool_of(p: &Value, k: &str) -> Result<bool, AutomationError> {
    match p.get(k) {
        None | Some(Value::Null) => Ok(false),
        Some(v) => v.as_bool().ok_or_else(|| bad(format!("`{k}` must be true or false"))),
    }
}

fn required<'a>(p: &'a Value, k: &str) -> Result<&'a str, AutomationError> {
    str_of(p, k).ok_or_else(|| bad(format!("`{k}` is required")))
}

impl Headless {
    /// Dispatch one request. Unknown methods and bad params are errors, never panics.
    pub fn handle(&mut self, method: &str, p: &Value) -> Result<Value, AutomationError> {
        match method {
            "ping" => Ok(json!("pong")),
            "methods" => Ok(json!(METHODS)),
            "session.inspect" => Ok(self.inspect()),
            "command.list" => Ok(self.command_list(str_of(p, "filter"))),
            "command.run" => self.command_run(required(p, "id")?, p.get("params").cloned().unwrap_or(Value::Null)),
            "command.batch" => {
                let steps = p.get("steps").and_then(Value::as_array).ok_or_else(|| bad("`steps` must be an array of {id, params}"))?;
                if steps.len() > MAX_BATCH_STEPS {
                    return Err(bad(format!("at most {MAX_BATCH_STEPS} steps per batch")));
                }
                Ok(self.batch(steps))
            }
            "project.open" => self.open(required(p, "path")?),
            "project.save" => self.save(str_of(p, "path")),
            "mesh.import" => self.import_mesh(required(p, "path")?, u32_of(p, "resolution")?, str_of(p, "name")),
            "mesh.reimport" => self.reimport_mesh(str_of(p, "path"), bool_of(p, "preserveStrokes")?),
            "mesh.export" => self.export_mesh(required(p, "path")?, str_of(p, "format")),
            "bake.mesh.add" => self.add_bake_mesh(required(p, "path")?, bool_of(p, "cage")?),
            "resource.import" => self.import_resource(required(p, "path")?),
            "material.save" => self.material_save(p),
            "material.load" => self.material_load(required(p, "path")?),
            "textures.export" => self.export(p),
            "render" => {
                let o = RenderOptions {
                    view: str_of(p, "view").map(str::to_string),
                    width: u32_of(p, "width")?,
                    height: u32_of(p, "height")?,
                    yaw: f32_of(p, "yaw")?,
                    pitch: f32_of(p, "pitch")?,
                    samples: u32_of(p, "samples")?,
                    bounces: u32_of(p, "bounces")?,
                    cpu: bool_of(p, "cpu")?,
                    preset: str_of(p, "preset").map(str::to_string),
                    scale: u32_of(p, "scale")?,
                    transparent: bool_of(p, "transparent")?,
                };
                let r = self.render_png(&o)?;
                Ok(json!({ "width": r.width, "height": r.height, "by": r.by, "png": base64::engine::general_purpose::STANDARD.encode(r.png) }))
            }
            m if m.starts_with("ui.") || m == "viewport.screenshot" => {
                Err(bad(format!("`{m}` needs the running app (texturecraft --control <port>); headless renders use `render`")))
            }
            other => Err(bad(format!("unknown method `{other}` (see `methods`)"))),
        }
    }
}

/// Answer one request line. A panic inside a command becomes an error reply (edits run on a
/// snapshot, so the session stays consistent).
pub fn respond(h: &Mutex<Headless>, line: &str) -> Value {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}),
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = req.get("method").and_then(Value::as_str) else {
        return json!({"id": id, "ok": false, "error": "`method` is required"});
    };
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.lock().unwrap_or_else(PoisonError::into_inner).handle(method, &params)))
        .unwrap_or_else(|_| Err(AutomationError::Other(format!("internal error in `{method}`"))));
    match r {
        Ok(v) => json!({"id": id, "ok": true, "result": v}),
        Err(e) => json!({"id": id, "ok": false, "error": e.to_string()}),
    }
}

/// Write one reply line; an oversized reply becomes an error with the same id.
fn write_reply(w: &mut impl Write, reply: &Value) -> std::io::Result<()> {
    let text = json_text(reply).unwrap_or_else(|e| json!({"id": reply.get("id"), "ok": false, "error": e.to_string()}).to_string());
    writeln!(w, "{text}")?;
    w.flush()
}

/// Serve JSON lines from `r` to `w` until EOF (stdio mode: no authentication).
pub fn serve_lines(h: &Mutex<Headless>, mut r: impl BufRead, mut w: impl Write) -> std::io::Result<()> {
    let mut line = String::new();
    loop {
        let reply = match read_bounded_line(&mut r, &mut line) {
            Ok(LineRead::Eof) => return Ok(()),
            Ok(LineRead::TooLong) => json!({"id": null, "ok": false, "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes")}),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => json!({"id": null, "ok": false, "error": "request is not UTF-8"}),
            Err(e) => return Err(e),
            Ok(LineRead::Line) if line.trim().is_empty() => continue,
            Ok(LineRead::Line) => respond(h, &line),
        };
        write_reply(&mut w, &reply)?;
    }
}

/// One TCP connection: the first request must be `auth {token}`, as on the app's channel.
fn serve_connection(h: &Mutex<Headless>, stream: std::net::TcpStream, token: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(600)))?;
    let mut r = std::io::BufReader::new(stream.try_clone()?);
    let mut w = stream;
    let mut line = String::new();
    let mut authed = false;
    let mut deadline = Some(AuthDeadline::start(&w, AUTH_TIMEOUT)?);
    loop {
        match read_bounded_line(&mut r, &mut line)? {
            LineRead::Eof => return Ok(()),
            LineRead::TooLong => return write_reply(&mut w, &json!({"id": null, "ok": false, "error": "request too large"})),
            LineRead::Line if line.trim().is_empty() => continue,
            LineRead::Line => {}
        }
        if authed {
            write_reply(&mut w, &respond(h, &line))?;
            continue;
        }
        let req: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        let supplied = req.get("params").and_then(|p| p.get("token")).and_then(Value::as_str).unwrap_or_default();
        authed = req.get("method").and_then(Value::as_str) == Some("auth") && token_matches(token, supplied);
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        if !authed {
            return write_reply(&mut w, &json!({"id": id, "ok": false, "error": "authenticate first: {\"method\":\"auth\",\"params\":{\"token\":…}}"}));
        }
        drop(deadline.take());
        write_reply(&mut w, &json!({"id": id, "ok": true}))?;
    }
}

/// Listen on `127.0.0.1:port` (0 picks a port).
pub fn bind_tcp(port: u16) -> Result<TcpListener, AutomationError> {
    TcpListener::bind(("127.0.0.1", port)).map_err(|e| AutomationError::Io(format!("cannot listen on 127.0.0.1:{port}: {e}")))
}

/// Serve authenticated JSON lines on `127.0.0.1:port` (0 picks a port) with one shared
/// session. `ready` gets the bound address.
pub fn serve_tcp(port: u16, h: Arc<Mutex<Headless>>, token: String, ready: impl FnOnce(std::net::SocketAddr)) -> Result<(), AutomationError> {
    let listener = bind_tcp(port)?;
    ready(listener.local_addr().map_err(|e| AutomationError::Io(e.to_string()))?);
    serve_listener(listener, h, token)
}

/// Serve authenticated JSON lines on a bound listener (see [`bind_tcp`]) with one shared session.
pub fn serve_listener(listener: TcpListener, h: Arc<Mutex<Headless>>, token: String) -> Result<(), AutomationError> {
    let limiter = ConnectionLimiter::default();
    let token = Arc::new(token);
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let Some(permit) = limiter.try_acquire(MAX_CONNECTIONS) else {
            let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "too many connections"}));
            continue;
        };
        let (h, token) = (h.clone(), token.clone());
        std::thread::spawn(move || {
            let _permit = permit;
            let _ = serve_connection(&h, stream, &token);
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headless::Filesystem;

    fn session() -> Mutex<Headless> {
        let s = texturecraft_engine::Session::new_project("t", texturecraft_engine::geom::primitives::cube(2.0), "builtin:cube", None, 128).unwrap();
        Mutex::new(Headless::new(s, Filesystem::Denied))
    }

    #[test]
    fn lines_round_trip_and_errors() {
        let h = session();
        let input = concat!(
            r#"{"id":1,"method":"command.run","params":{"id":"layer.add.fill","params":{"color":[1,0,0]}}}"#,
            "\n\n",
            r#"{"id":2,"method":"session.inspect"}"#,
            "\n",
            r#"{"id":3,"method":"nope"}"#,
            "\n",
            "not json\n",
            r#"{"id":4,"method":"command.run","params":{"id":"file.open","params":{"path":"/etc/passwd"}}}"#,
            "\n",
            r#"{"id":5,"method":"project.open","params":{"path":"x.tcraft"}}"#,
            "\n",
            r#"{"id":6,"method":"command.batch","params":{"steps":[{"id":"layer.add.paint"},{"id":"nope"},{"id":"layer.add.paint"}]}}"#,
            "\n",
        );
        let mut out = Vec::new();
        serve_lines(&h, input.as_bytes(), &mut out).unwrap();
        let r: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(r.len(), 7);
        assert_eq!(r[0]["ok"], true);
        assert_eq!(r[1]["result"]["layers"][0]["kind"], "fill");
        assert_eq!(r[2]["ok"], false);
        assert_eq!(r[3]["id"], Value::Null);
        assert!(r[4]["error"].as_str().unwrap().contains("disabled for automation"), "{}", r[4]);
        assert!(r[5]["error"].as_str().unwrap().contains("not granted"), "{}", r[5]);
        assert_eq!(r[6]["result"]["completed"], 1);
        assert_eq!(r[6]["result"]["failed"]["index"], 1);
    }

    #[test]
    fn tcp_requires_the_token() {
        use std::io::{BufRead, BufReader};
        let h = Arc::new(session());
        let (tx, rx) = std::sync::mpsc::channel();
        let token = "0123456789abcdef0123".to_string();
        std::thread::spawn(move || serve_tcp(0, h, token, |a| tx.send(a).unwrap()));
        let addr = rx.recv().unwrap();
        let talk = |lines: &[&str]| {
            let s = std::net::TcpStream::connect(addr).unwrap();
            let mut w = s.try_clone().unwrap();
            let mut r = BufReader::new(s);
            lines
                .iter()
                .map(|l| {
                    writeln!(w, "{l}").unwrap();
                    let mut reply = String::new();
                    r.read_line(&mut reply).unwrap();
                    serde_json::from_str::<Value>(&reply).unwrap()
                })
                .collect::<Vec<_>>()
        };
        let denied = talk(&[r#"{"id":1,"method":"session.inspect"}"#]);
        assert_eq!(denied[0]["ok"], false);
        let ok = talk(&[r#"{"id":1,"method":"auth","params":{"token":"0123456789abcdef0123"}}"#, r#"{"id":2,"method":"ping"}"#]);
        assert_eq!(ok[0]["ok"], true);
        assert_eq!(ok[1]["result"], "pong");
        assert_eq!(ok[1]["id"], 2);
    }
}
