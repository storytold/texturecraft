//! Integration tests, built as one test binary: each file is a module. They call `run` in-process,
//! except where a test needs the real binary (a protocol on stdio, the environment).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod cli;
mod man_and_completions;
mod protocol;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// What `run` returned and wrote.
pub struct Outcome {
    pub code: i32,
    pub out: String,
    pub err: String,
}

pub fn invoke<I, T>(args: I) -> Outcome
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = texturecraft_cli::run(args, &mut out, &mut err);
    Outcome { code, out: String::from_utf8_lossy(&out).into_owned(), err: String::from_utf8_lossy(&err).into_owned() }
}

/// A new, empty folder for one test.
pub fn workdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("texturecraft-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A small project made by `new`.
pub fn project(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let made = invoke(argv(&[&"new", &"builtin:cube", &path, &"--resolution", &"128"]));
    assert_eq!(made.code, 0, "{}", made.err);
    path
}

/// An argument vector from parts of different types (strings, paths).
pub fn argv(parts: &[&dyn AsRef<OsStr>]) -> Vec<OsString> {
    parts.iter().map(|p| p.as_ref().to_os_string()).collect()
}
