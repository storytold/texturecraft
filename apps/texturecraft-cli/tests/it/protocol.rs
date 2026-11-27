//! The subcommands that talk over a stream (`mcp` and `serve` on stdio, through the real binary,
//! which owns stdin and stdout) and the environment variable of the control token.

use std::io::Write;
use std::net::TcpListener;
use std::process::{Command, Output, Stdio};

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_texturecraft-cli"));
    command.env("TEXTURECRAFT_NO_PREFS", "1").env_remove("TEXTURECRAFT_CONTROL_TOKEN");
    command
}

/// A port nothing listens on, with no token file.
fn free_port() -> String {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().to_string()
}

#[test]
fn serve_writes_nothing_but_json_lines_to_stdout() {
    let mut child = binary().arg("serve").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"id":1,"method":"ping","params":{{}}}}"#).unwrap();
    writeln!(stdin, "not json").unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stderr.is_empty(), "{}", String::from_utf8_lossy(&output.stderr));
    let lines: Vec<_> = String::from_utf8(output.stdout).unwrap().lines().map(str::to_string).collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    for line in &lines {
        assert!(serde_json::from_str::<serde_json::Value>(line).unwrap().get("ok").is_some(), "{line}");
    }
}

#[test]
fn serve_ends_quietly_and_mcp_reports_it_when_stdin_is_closed() {
    let output = binary().arg("serve").stdin(Stdio::null()).output().unwrap();
    assert_eq!((output.status.code(), output.stdout.is_empty(), output.stderr.is_empty()), (Some(0), true, true));
    // An MCP client must open with `initialize`; end of input before it is a failed start.
    let output = binary().arg("mcp").stdin(Stdio::null()).output().unwrap();
    assert_eq!((output.status.code(), output.stdout.is_empty()), (Some(1), true));
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("error: MCP start: "), "{}", String::from_utf8_lossy(&output.stderr));
}

fn bridge(token_flag: Option<&str>, env: Option<&str>) -> Output {
    let port = free_port();
    let mut command = binary();
    command.args(["mcp", "--bridge", &port]);
    if let Some(token) = token_flag {
        command.args(["--token", token]);
    }
    if let Some(token) = env {
        command.env("TEXTURECRAFT_CONTROL_TOKEN", token);
    }
    command.stdin(Stdio::null()).output().unwrap()
}

#[test]
fn the_control_token_variable_is_a_fallback_the_flag_wins_and_an_empty_one_is_unset() {
    let secret = "env-token-0123456789-secret";
    // Empty or blank counts as unset: the error says to set one, as when there is none.
    for blank in ["", "   "] {
        let output = bridge(None, Some(blank));
        let err = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{err}");
        assert!(err.contains("no control token: set TEXTURECRAFT_CONTROL_TOKEN"), "{err}");
    }
    // The variable is used when there is no flag: a good token gets as far as connecting.
    let err = String::from_utf8_lossy(&bridge(None, Some(secret)).stderr).into_owned();
    assert!(!err.contains("16 to 1024") && !err.contains(secret), "{err}");
    // The flag wins over a good variable: its bad value is what is reported, without echoing either.
    let err = String::from_utf8_lossy(&bridge(Some("short"), Some(secret)).stderr).into_owned();
    assert!(err.contains("control token must be 16 to 1024"), "{err}");
    assert!(!err.contains(secret) && !err.contains("short\n"), "{err}");
    // A bad value in the variable is refused too, and its value stays out of the message.
    let err = String::from_utf8_lossy(&bridge(None, Some("bad token")).stderr).into_owned();
    assert!(err.contains("control token must be 16 to 1024") && !err.contains("bad token"), "{err}");
}

#[test]
fn help_lists_the_variable_but_never_its_value() {
    let output = binary().args(["mcp", "--help"]).env("TEXTURECRAFT_CONTROL_TOKEN", "do-not-print-this-0123456789").output().unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("TEXTURECRAFT_CONTROL_TOKEN") && !help.contains("do-not-print-this"), "{help}");
}

#[test]
fn a_variable_alone_grants_no_file_access() {
    // The token variable is for authentication only: a headless server with no roots still refuses files.
    let mut child =
        binary().arg("serve").env("TEXTURECRAFT_CONTROL_TOKEN", "env-token-0123456789-secret").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"id":1,"method":"project.save","params":{{"path":"never.tcraft"}}}}"#).unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    let reply: serde_json::Value = serde_json::from_str(String::from_utf8(output.stdout).unwrap().lines().next().unwrap()).unwrap();
    assert_eq!(reply["ok"], false, "{reply}");
}

#[test]
fn a_token_in_the_environment_does_not_require_bridge() {
    // Headless MCP must parse even when TEXTURECRAFT_CONTROL_TOKEN is set; only --token on the
    // command line requires --bridge.
    let output = binary().args(["mcp", "--fromat"]).env("TEXTURECRAFT_CONTROL_TOKEN", "env-token-0123456789-secret").output().unwrap();
    let err = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{err}");
    assert!(err.contains("--fromat"), "{err}");
    assert!(!err.contains("--bridge") && !err.contains("--token is for"), "{err}");

    let output = binary().args(["mcp", "--token", "flag-token-0123456789"]).output().unwrap();
    let err = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{err}");
    assert!(err.contains("--token") && err.contains("--bridge"), "{err}");
}
