//! RFC-0018 rule 2.7: help, versions, usage errors with exit 2 on stderr, and nothing written
//! before the arguments are checked.

use std::ffi::OsString;

use crate::{argv, invoke, project, workdir};

const SUBCOMMANDS: [&str; 10] = ["info", "commands", "run", "batch", "bake", "export", "render", "new", "mcp", "serve"];

/// The shortest valid command line of each subcommand.
fn minimal(sub: &str) -> Vec<&str> {
    match sub {
        "info" => vec!["info", "builtin:cube"],
        "run" => vec!["run", "builtin:cube", "layer.add.fill"],
        "batch" => vec!["batch", "s.json"],
        "bake" => vec!["bake", "builtin:cube", "-o", "o.tcraft"],
        "export" => vec!["export", "builtin:cube", "dir"],
        "render" => vec!["render", "builtin:cube", "o.png"],
        "new" => vec!["new", "builtin:cube", "o.tcraft"],
        other => vec![other],
    }
}

fn usage_error(args: &[&str]) -> String {
    let got = invoke(args.iter().copied());
    assert_eq!(got.code, 2, "{args:?}: {}", got.err);
    assert_eq!(got.out, "", "{args:?} wrote to stdout");
    assert!(got.err.starts_with("error: "), "{args:?}: {}", got.err);
    got.err
}

#[test]
fn help_and_the_version_go_to_stdout_with_exit_0() {
    for args in [&["--help"][..], &["-h"], &["help"], &["--version"], &["-V"]] {
        let got = invoke(args.iter().copied());
        assert_eq!((got.code, got.err.as_str()), (0, ""), "{args:?}");
        assert!(got.out.contains("texturecraft-cli"), "{args:?}: {}", got.out);
    }
    let help = invoke(["--help"]).out;
    assert!(help.contains("Exit status:") && help.contains("128 + the signal number"), "{help}");
    for sub in SUBCOMMANDS {
        for flag in ["--help", "-h"] {
            let got = invoke([sub, flag]);
            assert_eq!((got.code, got.err.as_str()), (0, ""), "{sub} {flag}");
            assert!(got.out.contains(&format!("Usage: texturecraft-cli {sub}")), "{sub}: {}", got.out);
        }
        assert_eq!(invoke(["help", sub]).out, invoke([sub, "--help"]).out, "{sub}");
    }
}

#[test]
fn every_subcommand_takes_the_version_and_there_is_no_version_subcommand() {
    let top = invoke(["--version"]).out;
    let want = top.strip_prefix("texturecraft-cli ").unwrap().trim_end().to_string();
    assert!(want.starts_with(texturecraft_engine::VERSION), "{want}");
    for sub in SUBCOMMANDS {
        for flag in ["--version", "-V"] {
            let got = invoke([sub, flag]);
            assert_eq!((got.code, got.out.as_str(), got.err.as_str()), (0, format!("texturecraft-cli-{sub} {want}\n").as_str(), ""), "{sub} {flag}");
        }
    }
    usage_error(&["version"]);
}

#[test]
fn no_arguments_is_a_usage_error_that_prints_the_help_on_stderr() {
    let got = invoke(Vec::<OsString>::new());
    assert_eq!((got.code, got.out.as_str()), (2, ""));
    assert!(got.err.contains("Usage: texturecraft-cli <COMMAND>"), "{}", got.err);
}

#[test]
fn an_unknown_flag_or_subcommand_is_refused_by_every_subcommand() {
    for sub in SUBCOMMANDS {
        let mut args = minimal(sub);
        args.push("--fromat");
        let message = usage_error(&args);
        assert!(message.contains("--fromat"), "{sub}: {message}");
    }
    usage_error(&["rendre"]);
    usage_error(&["--fromat"]);
}

#[test]
fn a_flag_given_twice_is_refused() {
    usage_error(&["new", "builtin:cube", "o.tcraft", "--resolution", "8", "--resolution", "9"]);
    usage_error(&["bake", "builtin:cube", "-o", "a.tcraft", "-o", "b.tcraft"]);
    usage_error(&["render", "builtin:cube", "o.png", "--samples", "2", "--cpu", "--cpu"]);
    usage_error(&["mcp", "--read-root", "a", "--automation-read-root", "b"]);
}

#[test]
fn bad_values_conflicts_and_missing_arguments_are_refused_before_any_file_is_written() {
    let dir = workdir("refused");
    let out = dir.join("never");
    let o = out.to_str().unwrap();
    let project_file = dir.join("p.tcraft");
    let p = project_file.to_str().unwrap();
    for (args, expect) in [
        (vec!["new", "builtin:cube", o, "--resolution", "0"], "--resolution"),
        (vec!["new", "builtin:cube", o, "--resolution", "big"], "--resolution"),
        (vec!["bake", "builtin:cube", "-o", o, "--resolution", "-1"], "-1"),
        (vec!["bake", "builtin:cube"], "give -o"),
        (vec!["bake", p], "--in-place"),
        (vec!["bake", "builtin:cube", "--in-place"], "-o"),
        (vec!["export", "builtin:cube", o, "--size", "0"], "--size"),
        (vec!["export", "builtin:cube"], "required"),
        (vec!["run", "builtin:cube", "layer.add.fill", "{not json"], "not JSON"),
        (vec!["run", "builtin:cube", "layer.add.fill", "{}", "--in-place"], "--output"),
        (vec!["render", "builtin:cube", o, "--size", "big"], "<W>x<H>"),
        (vec!["render", "builtin:cube", o, "--size", "0x5"], "<W>x<H>"),
        (vec!["render", "builtin:cube", o, "--view", "4d"], "--view"),
        (vec!["render", "builtin:cube", o, "--yaw", "NaN"], "--yaw"),
        (vec!["render", "builtin:cube", o, "--samples", "0"], "--samples"),
        (vec!["render", "builtin:cube", o, "--cpu"], "--samples"),
        (vec!["render", "builtin:cube", o, "--bounces", "3"], "--samples"),
        (vec!["mcp", "--token", "abcdefghijklmnop"], "--bridge"),
        (vec!["mcp", "--bridge", "9878", "--read-root", o], "headless server"),
        (vec!["mcp", "--bridge", "99999"], "--bridge"),
        (vec!["mcp", "--bridge", "0"], "--bridge"),
        (vec!["serve", "--port", "http"], "--port"),
    ] {
        let message = usage_error(&args);
        assert!(message.contains(expect), "{args:?}: {message}");
    }
    assert!(!out.exists());
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn loopback_ports_take_the_host_prefix_and_a_dashed_name_follows_a_double_dash() {
    // `--bridge 127.0.0.1:PORT` and `localhost:PORT` are still ports: the failure is the missing token.
    for bridge in ["127.0.0.1:9", "localhost:9"] {
        let got = invoke(["mcp", "--bridge", bridge, "--token", "short"]);
        assert_eq!(got.code, 1, "{bridge}: {}", got.err);
    }
    let got = invoke(["info", "--", "-odd.tcraft"]);
    assert_eq!(got.code, 1);
    assert!(got.err.contains("-odd.tcraft"), "{}", got.err);
    usage_error(&["render", "builtin:cube", "o.png", "--camera", "--cpu"]);
}

#[test]
fn info_and_commands_print_what_a_program_reads() {
    let got = invoke(["info", "builtin:cube"]);
    assert_eq!((got.code, got.err.as_str()), (0, ""));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&got.out).unwrap()["kind"], "mesh");
    let table = invoke(["commands", "--filter", "layer.add"]);
    assert_eq!((table.code, table.err.as_str()), (0, ""));
    assert!(table.out.lines().all(|l| l.starts_with("layer.add") || l.trim().is_empty()) && !table.out.is_empty(), "{}", table.out);
    let json = invoke(["commands", "--filter", "layer.add", "--json"]);
    assert_eq!(json.code, 0);
    let list: serde_json::Value = serde_json::from_str(&json.out).unwrap();
    assert!(list.as_array().is_some_and(|l| !l.is_empty()), "{}", json.out);
    let got = invoke(["info", "builtin:bogus"]);
    assert_eq!((got.code, got.out.as_str()), (1, ""));
    assert!(got.err.starts_with("error: unknown built-in mesh `bogus`"), "{}", got.err);
}

#[test]
fn new_run_bake_and_export_through_files() {
    let dir = workdir("files");
    let a = project(&dir, "a.tcraft");
    let b = dir.join("b.tcraft");
    let got = invoke(argv(&[&"run", &a, &"layer.add.fill", &r#"{"color":[1,0,0]}"#, &"-o", &b]));
    assert_eq!(got.code, 0, "{}", got.err);
    assert!(got.err.starts_with("saved "), "{}", got.err);
    assert!(b.is_file());
    let got = invoke(argv(&[&"bake", &b, &"--resolution", &"128", &"--maps", &"ambientOcclusion", &"--in-place"]));
    assert_eq!(got.code, 0, "{}", got.err);
    let textures = dir.join("tex");
    let got = invoke(argv(&[&"export", &b, &textures, &"--preset", &"glTF PBR Metal Roughness", &"--size", &"128"]));
    assert_eq!(got.code, 0, "{}", got.err);
    assert!(got.out.lines().count() >= 3, "{}", got.out);
    let got = invoke(argv(&[&"new", &"builtin:cube", &a]));
    assert_eq!(got.code, 0, "{}", got.err);
    let got = invoke(argv(&[&"new", &a, &dir.join("again.tcraft")]));
    assert_eq!((got.code, got.out.as_str()), (1, ""));
    assert!(got.err.contains("is already a project"), "{}", got.err);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_output_that_would_replace_an_input_is_refused_without_in_place() {
    let dir = workdir("replace");
    let source = project(&dir, "a.tcraft");
    let a = source.to_str().unwrap();
    let before = std::fs::read(&source).unwrap();
    let again = dir.join(".").join("a.tcraft");
    let again = again.to_str().unwrap();
    for args in [
        vec!["run", a, "layer.add.fill", "{}", "-o", a],
        vec!["run", a, "layer.add.fill", "{}", "-o", again],
        vec!["bake", a, "-o", a],
        vec!["bake", a],
        vec!["render", a, again],
        vec!["new", a, a],
    ] {
        let message = usage_error(&args);
        assert!(message.contains("--in-place") || message.contains("would replace"), "{args:?}: {message}");
    }
    assert_eq!(std::fs::read(&source).unwrap(), before, "a refused command changes nothing");
    // `--in-place` lets only a project be replaced, and only by itself.
    let got = invoke(["run", a, "layer.add.fill", "{}", "-o", a, "--in-place"]);
    assert_eq!(got.code, 0, "{}", got.err);
    let got = invoke(["bake", a, "--in-place", "--resolution", "128", "--maps", "ambientOcclusion"]);
    assert_eq!(got.code, 0, "{}", got.err);
    let elsewhere = dir.join("b.tcraft");
    assert!(usage_error(&["run", a, "layer.add.fill", "{}", "-o", elsewhere.to_str().unwrap(), "--in-place"]).contains("--in-place"));
    assert!(usage_error(&["bake", a, "-o", elsewhere.to_str().unwrap(), "--in-place"]).contains("--in-place"));
    assert!(!elsewhere.exists());
    // Headless::save appends .tcraft, so `-o scene` would replace scene.tcraft.
    let unsuffixed = dir.join("a");
    let unsuffixed = unsuffixed.to_str().unwrap();
    let after_in_place = std::fs::read(&source).unwrap();
    let message = usage_error(&["run", a, "layer.add.fill", "{}", "-o", unsuffixed]);
    assert!(message.contains("would replace") || message.contains("--in-place"), "{message}");
    assert_eq!(std::fs::read(&source).unwrap(), after_in_place, "an unsuffixed -o changes nothing");
    let got = invoke(["run", a, "layer.add.fill", "{}", "-o", unsuffixed, "--in-place"]);
    assert_eq!(got.code, 0, "{}", got.err);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn batch_runs_a_script_and_a_failing_step_fails_the_command() {
    let dir = workdir("batch");
    let good = dir.join("good.json");
    std::fs::write(&good, r#"[{"id": "layer.add.paint", "params": {"name": "Scripted"}}]"#).unwrap();
    let got = invoke(argv(&[&"batch", &good]));
    assert_eq!((got.code, got.err.as_str()), (0, ""), "{}", got.out);
    let bad = dir.join("bad.json");
    std::fs::write(&bad, r#"[{"id": "layer.add.paint"}, {"id": "no.such.command"}]"#).unwrap();
    let got = invoke(argv(&[&"batch", &bad]));
    assert_eq!(got.code, 1);
    assert!(got.err.starts_with("error: step 1 (`no.such.command`) failed: "), "{}", got.err);
    for (body, expect) in [("[", "is not JSON"), ("{}", "list of steps"), (r#"{"steps": [], "open": false}"#, "`open` field must be a string")] {
        let script = dir.join("odd.json");
        std::fs::write(&script, body).unwrap();
        let got = invoke(argv(&[&"batch", &script]));
        assert_eq!(got.code, 1, "{body}");
        assert!(got.err.contains(expect), "{body}: {}", got.err);
    }
    let got = invoke(argv(&[&"batch", &dir.join("missing.json")]));
    assert_eq!(got.code, 1);
    assert!(got.err.contains("missing.json"), "{}", got.err);
    // A script that saves over the project it opened needs --in-place, and must not have run.
    let opened = project(&dir, "opened.tcraft");
    let before = std::fs::read(&opened).unwrap();
    let overwrite = dir.join("overwrite.json");
    std::fs::write(
        &overwrite,
        serde_json::json!({
            "open": opened,
            "save": opened,
            "steps": [{"id": "layer.add.paint", "params": {"name": "Batch"}}]
        })
        .to_string(),
    )
    .unwrap();
    let message = usage_error(&["batch", overwrite.to_str().unwrap()]);
    assert!(message.contains("--in-place") || message.contains("would replace"), "{message}");
    assert_eq!(std::fs::read(&opened).unwrap(), before);
    let got = invoke(argv(&[&"batch", &overwrite, &"--in-place"]));
    assert_eq!(got.code, 0, "{}", got.err);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_failed_command_exits_1_with_the_message_on_stderr() {
    let got = invoke(["run", "builtin:cube", "no.such.command"]);
    assert_eq!((got.code, got.out.as_str()), (1, ""));
    assert!(got.err.starts_with("error: "), "{}", got.err);
    let got = invoke(["info", "missing.tcraft"]);
    assert_eq!(got.code, 1);
    assert!(got.err.starts_with("error: ") && got.err.contains("missing.tcraft"), "{}", got.err);
}

#[test]
fn the_desktop_launch_options_are_refused_before_any_subcommand() {
    for args in
        [vec!["--launch-profile", "nightly-v1", "--version"], vec!["--profile-root=/tmp/p", "--version"], vec!["info", "builtin:cube", "--launch-profile=x"]]
    {
        let message = usage_error(&args);
        assert!(message.contains("desktop launch option"), "{args:?}: {message}");
    }
}

#[cfg(unix)]
#[test]
fn a_path_that_is_not_valid_unicode_is_named_in_the_error() {
    use std::os::unix::ffi::OsStringExt;
    let dir = workdir("non-unicode");
    // The engine's commands take their paths as JSON text: the command says so and names the file.
    let input = dir.join(OsString::from_vec(b"in-\xff.tcraft".to_vec()));
    let got = invoke(argv(&[&"info", &input]));
    assert_eq!(got.code, 1);
    assert!(got.err.contains("the path is not valid Unicode") && got.err.contains("in-"), "{}", got.err);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_closed_stdout_is_not_an_error() {
    struct Closed;
    impl std::io::Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut err = Vec::new();
    assert_eq!(texturecraft_cli::run(["commands"], &mut Closed, &mut err), 0);
    assert_eq!(texturecraft_cli::run(["--help"], &mut Closed, &mut err), 0);
    assert!(err.is_empty());
}

#[test]
fn headless_mcp_requires_both_roots() {
    for args in [
        vec!["mcp"],
        vec!["mcp", "--read-root", "."],
        vec!["mcp", "--write-root", "."],
        vec!["mcp", "--automation-read-root", "."],
        vec!["mcp", "--automation-write-root", "."],
    ] {
        let message = usage_error(&args);
        assert!(message.contains("--read-root") && message.contains("--write-root"), "{args:?}: {message}");
    }
}
