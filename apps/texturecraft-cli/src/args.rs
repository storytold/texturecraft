//! The command line as clap derive types. Values arrive typed, so the commands never validate a
//! string; what clap can't say alone is a [`UsageError`], printed and exited like clap's own.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use clap::error::ErrorKind;
use clap::{Args, FromArgMatches, Parser, Subcommand};
use serde_json::Value;

/// The end of `--help` of every command.
const EXIT_STATUS: &str = "Exit status:\n  0  success\n  1  a command failed\n  2  usage error\n\nA process ended by a signal exits 128 + the signal number (130 for Ctrl-C), as in the shell.";

const INPUTS: &str = "An <INPUT> is a .tcraft project, a mesh file (.glb .gltf .fbx .dae .usd .usda .usdc .usdz .abc .obj .ply .stl) or builtin:<id> (material_ball, sphere, cube, rounded_cube, cylinder, torus, plane, crate, udim_plane). See docs/control-protocol.md.";

/// Headless TextureCraft
#[derive(Parser, Debug)]
#[command(name = "texturecraft-cli", propagate_version = true, arg_required_else_help = true, after_help = EXIT_STATUS)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand, Debug)]
pub(crate) enum Command {
    /// Texture sets, layers, triangles, materials and the UV report, as JSON
    #[command(after_help = INPUTS)]
    Info(InfoArgs),
    /// List the engine commands (id, label, params)
    Commands(CommandsArgs),
    /// Run one command and print its JSON result; -o saves the project
    #[command(after_help = INPUTS)]
    Run(RunArgs),
    /// Run a script of commands
    ///
    /// The script is JSON: [{"id", "params"}, ...] or {"open": <input>, "steps": [...], "save": <out.tcraft>}.
    Batch(BatchArgs),
    /// Bake mesh maps for every texture set
    ///
    /// A project is saved in place without -o, which needs --in-place; a mesh needs -o.
    #[command(after_help = INPUTS)]
    Bake(BakeArgs),
    /// Export textures (default preset: Document channels + Normal + AO (No Alpha))
    #[command(after_help = INPUTS)]
    Export(ExportArgs),
    /// Render the viewport to PNG (needs a GPU or software adapter)
    ///
    /// --camera looks through a camera saved in the project (camera.add); --samples path traces it
    /// instead (Rendering mode), on the GPU when it has compute shaders, else (or with --cpu) on
    /// the CPU.
    #[command(after_help = INPUTS)]
    Render(RenderArgs),
    /// A new project from a mesh or a built-in mesh
    ///
    /// --sample bakes and applies a smart material to every texture set. The sample's mesh maps are
    /// baked at the resolution up to 1024 px (most of the file's size), so a 2048 sample project is
    /// as large as a 1024 one.
    #[command(after_help = INPUTS)]
    New(NewArgs),
    /// MCP server on stdio: headless, or bridged to `texturecraft --control <port>`
    ///
    /// Headless, file access is only below the roots.
    Mcp(McpArgs),
    /// JSON lines ({"id","method","params"}) on stdio, or on 127.0.0.1:<port> with a token
    Serve(ServeArgs),
}

#[derive(Args, Debug)]
pub(crate) struct InfoArgs {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
}

#[derive(Args, Debug)]
pub(crate) struct CommandsArgs {
    /// Only the commands whose id or label contains this text
    #[arg(long, value_name = "TEXT")]
    pub(crate) filter: Option<String>,
    /// Print the list as JSON instead of a table
    #[arg(long)]
    pub(crate) json: bool,
}

#[derive(Args, Debug)]
pub(crate) struct RunArgs {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
    #[arg(value_name = "COMMAND-ID")]
    pub(crate) id: String,
    /// The command's parameters, as JSON
    #[arg(value_name = "PARAMS-JSON", value_parser = json_value)]
    pub(crate) params: Option<Value>,
    /// Save the project here
    #[arg(short = 'o', long = "output", value_name = "OUT.TCRAFT")]
    pub(crate) output: Option<PathBuf>,
    /// Allow -o to replace the project it opened
    #[arg(long, requires = "output")]
    pub(crate) in_place: bool,
}

#[derive(Debug, PartialEq, thiserror::Error)]
#[error("params are not JSON: {0}")]
pub(crate) struct NotJson(String);

fn json_value(s: &str) -> Result<Value, NotJson> {
    serde_json::from_str(s).map_err(|e| NotJson(e.to_string()))
}

#[derive(Args, Debug)]
pub(crate) struct BatchArgs {
    #[arg(value_name = "SCRIPT.JSON")]
    pub(crate) script: PathBuf,
    /// Allow `save` to replace the project `open` named
    #[arg(long)]
    pub(crate) in_place: bool,
}

#[derive(Args, Debug)]
pub(crate) struct BakeArgs {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
    /// Save the baked project here
    #[arg(short = 'o', long = "output", value_name = "OUT.TCRAFT")]
    pub(crate) output: Option<PathBuf>,
    /// Bake at this size
    #[arg(long, value_name = "PX", value_parser = positive)]
    pub(crate) resolution: Option<u32>,
    /// Only these maps (ids, comma separated)
    #[arg(long, value_name = "ID,ID,...", value_delimiter = ',')]
    pub(crate) maps: Vec<String>,
    /// Save over the project it opened (without -o), or allow -o to replace it
    #[arg(long)]
    pub(crate) in_place: bool,
}

#[derive(Args, Debug)]
pub(crate) struct ExportArgs {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
    #[arg(value_name = "DIR")]
    pub(crate) dir: PathBuf,
    /// An export preset by name
    #[arg(long, value_name = "NAME")]
    pub(crate) preset: Option<String>,
    /// File format (an extension)
    #[arg(long, value_name = "EXT")]
    pub(crate) format: Option<String>,
    /// Padding mode
    #[arg(long, value_name = "MODE")]
    pub(crate) padding: Option<String>,
    /// Texture size
    #[arg(long, value_name = "PX", value_parser = positive)]
    pub(crate) size: Option<u32>,
}

#[derive(Args, Debug)]
pub(crate) struct RenderArgs {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
    #[arg(value_name = "OUT.PNG")]
    pub(crate) output: PathBuf,
    /// Image size
    #[arg(long, value_name = "WxH")]
    pub(crate) size: Option<Dimensions>,
    /// The viewport: 3D, or the UV view of the active texture set
    #[arg(long, value_name = "VIEW", value_parser = ["3d", "2d", "uv"])]
    pub(crate) view: Option<String>,
    /// Look through a camera saved in the project
    #[arg(long, value_name = "NAME")]
    pub(crate) camera: Option<String>,
    /// Camera yaw in degrees
    #[arg(long, value_name = "DEG", allow_negative_numbers = true, value_parser = finite)]
    pub(crate) yaw: Option<f32>,
    /// Camera pitch in degrees
    #[arg(long, value_name = "DEG", allow_negative_numbers = true, value_parser = finite)]
    pub(crate) pitch: Option<f32>,
    /// Path trace with this many samples per pixel
    #[arg(long, value_name = "N", value_parser = positive)]
    pub(crate) samples: Option<u32>,
    /// Light bounces of the path tracer
    #[arg(long, value_name = "N", requires = "samples")]
    pub(crate) bounces: Option<u32>,
    /// Path trace on the CPU
    #[arg(long, requires = "samples")]
    pub(crate) cpu: bool,
}

#[derive(Args, Debug)]
pub(crate) struct NewArgs {
    #[arg(value_name = "INPUT")]
    pub(crate) input: PathBuf,
    #[arg(value_name = "OUT.TCRAFT")]
    pub(crate) output: PathBuf,
    /// Texture size
    #[arg(long, value_name = "PX", value_parser = positive)]
    pub(crate) resolution: Option<u32>,
    /// Bake and apply a smart material to every texture set
    #[arg(long)]
    pub(crate) sample: bool,
}

/// The folders a server may read from and write to; with neither, it has no file access.
#[derive(Args, Debug, Default)]
pub(crate) struct RootArgs {
    /// Serve file access below this folder (alias --automation-read-root)
    #[arg(long, alias = "automation-read-root", value_name = "DIR")]
    pub(crate) read_root: Option<PathBuf>,
    /// Allow writes below this folder (alias --automation-write-root)
    #[arg(long, alias = "automation-write-root", value_name = "DIR")]
    pub(crate) write_root: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub(crate) struct McpArgs {
    /// Bridge to a running app's control port instead of a headless session
    #[arg(long, value_name = "PORT", value_parser = port)]
    pub(crate) bridge: Option<u16>,
    /// The app's control token (TEXTURECRAFT_CONTROL_TOKEN; else the file the app wrote)
    #[arg(long, value_name = "TOKEN", env = "TEXTURECRAFT_CONTROL_TOKEN", hide_env_values = true)]
    pub(crate) token: Option<String>,
    #[command(flatten)]
    pub(crate) roots: RootArgs,
}

#[derive(Args, Debug)]
pub(crate) struct ServeArgs {
    /// Listen on 127.0.0.1:<PORT> with a token, instead of stdio
    #[arg(long, value_name = "PORT", value_parser = port)]
    pub(crate) port: Option<u16>,
    #[command(flatten)]
    pub(crate) roots: RootArgs,
}

#[derive(Debug, PartialEq, thiserror::Error)]
#[error("`{0}` is not a number of pixels above zero")]
pub(crate) struct NotPositive(String);

fn positive(s: &str) -> Result<u32, NotPositive> {
    s.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(|| NotPositive(s.to_string()))
}

#[derive(Debug, PartialEq, thiserror::Error)]
#[error("`{0}` is not a finite number")]
pub(crate) struct NotFinite(String);

fn finite(s: &str) -> Result<f32, NotFinite> {
    s.parse::<f32>().ok().filter(|n| n.is_finite()).ok_or_else(|| NotFinite(s.to_string()))
}

#[derive(Debug, PartialEq, thiserror::Error)]
#[error("`{0}` is not a loopback port (1-65535)")]
pub(crate) struct BadPort(String);

/// A port, as `7878`, `127.0.0.1:7878` or `localhost:7878`.
fn port(s: &str) -> Result<u16, BadPort> {
    let digits = s.strip_prefix("127.0.0.1:").or_else(|| s.strip_prefix("localhost:")).unwrap_or(s);
    digits.parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(|| BadPort(s.to_string()))
}

/// `--size` of `render`: WIDTHxHEIGHT.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Dimensions {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, PartialEq, thiserror::Error)]
#[error("`{0}` is not <W>x<H>, e.g. 1280x800")]
pub(crate) struct BadDimensions(String);

impl FromStr for Dimensions {
    type Err = BadDimensions;

    fn from_str(s: &str) -> Result<Self, BadDimensions> {
        let (width, height) = s.split_once(['x', 'X']).ok_or_else(|| BadDimensions(s.to_string()))?;
        match (positive(width), positive(height)) {
            (Ok(width), Ok(height)) => Ok(Dimensions { width, height }),
            _ => Err(BadDimensions(s.to_string())),
        }
    }
}

/// A mistake in the command line that clap can't see by itself.
#[derive(Debug, PartialEq, thiserror::Error)]
pub(crate) enum UsageError {
    #[error("{output} {} is the {input}, which it would replace: choose another file{}", path.display(), if *in_place_helps { ", or pass --in-place" } else { "" })]
    WouldReplace { output: &'static str, input: &'static str, path: PathBuf, in_place_helps: bool },
    #[error("--in-place is for an output that is the INPUT, and this is another file")]
    InPlaceNotUsed,
    #[error("a mesh is not saved in place: give -o <out.tcraft> to save the baked project")]
    BakeNeedsOutput,
    #[error("saving over {} needs --in-place (or give -o <out.tcraft>)", path.display())]
    BakeInPlace { path: PathBuf },
    #[error("{flag} is for a headless server, and --bridge connects to an app")]
    RootsWithBridge { flag: &'static str },
    #[error("--token is for --bridge, and this is a headless server")]
    TokenNeedsBridge,
}

impl UsageError {
    /// The clap error, printed with the usage and exited with 2 like clap's own.
    pub(crate) fn into_clap(self, subcommand: &str) -> clap::Error {
        let mut command = crate::command();
        // Built, so a subcommand's usage names the program: `texturecraft-cli run`.
        command.build();
        let mut command = command.find_subcommand_mut(subcommand).map_or_else(crate::command, std::mem::take);
        command.error(ErrorKind::ArgumentConflict, self.to_string())
    }
}

/// Whether two paths name the same existing file, through links and `.` and `..`.
fn same_file(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

/// Whether `path` is a .tcraft project.
pub(crate) fn is_project(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(texturecraft_engine::core::PROJECT_EXT))
}

/// The path `Headless::save` writes: it appends `.tcraft` when the extension is missing.
fn project_save_path(path: &Path) -> PathBuf {
    if is_project(path) {
        path.to_owned()
    } else {
        let mut raw = path.as_os_str().to_os_string();
        raw.push(".");
        raw.push(texturecraft_engine::core::PROJECT_EXT);
        PathBuf::from(raw)
    }
}

fn would_replace(output: &'static str, input: &'static str, to: &Path, from: &Path, in_place_helps: bool) -> Result<(), UsageError> {
    if same_file(to, from) { Err(UsageError::WouldReplace { output, input, path: to.to_owned(), in_place_helps }) } else { Ok(()) }
}

fn would_replace_project(output: &'static str, input: &'static str, to: &Path, from: &Path, in_place_helps: bool) -> Result<(), UsageError> {
    would_replace(output, input, &project_save_path(to), from, in_place_helps)
}

fn same_project(output: &Path, input: &Path) -> bool {
    same_file(&project_save_path(output), input)
}

/// If a batch script's `save` would replace `open`, it needs `--in-place`. An unreadable or
/// malformed script is left to the command, which reports that as a failure (exit 1).
fn batch_would_replace(a: &BatchArgs) -> Result<(), UsageError> {
    let Ok(text) = std::fs::read_to_string(&a.script) else {
        return Ok(());
    };
    let Ok(script) = serde_json::from_str::<Value>(&text) else {
        return Ok(());
    };
    let field = |name: &str| script.get(name).and_then(Value::as_str);
    let Some(output) = field("save") else {
        return Ok(());
    };
    let input = field("open").unwrap_or("builtin:material_ball");
    if input.starts_with("builtin:") {
        return if a.in_place { Err(UsageError::InPlaceNotUsed) } else { Ok(()) };
    }
    let input = Path::new(input);
    let output = Path::new(output);
    match (same_project(output, input), a.in_place) {
        (true, false) => Err(UsageError::WouldReplace { output: "save", input: "open", path: project_save_path(output), in_place_helps: true }),
        (false, true) => Err(UsageError::InPlaceNotUsed),
        _ => Ok(()),
    }
}

impl Command {
    /// What clap can't check: the files a command would replace, and flags for another mode.
    fn check(&self) -> Result<(), (&'static str, UsageError)> {
        match self {
            Command::Run(a) => match (&a.output, a.in_place) {
                (Some(out), false) => would_replace_project("-o", "INPUT", out, &a.input, true).map_err(|e| ("run", e)),
                (Some(out), true) if !same_project(out, &a.input) => Err(("run", UsageError::InPlaceNotUsed)),
                _ => Ok(()),
            },
            Command::Bake(a) => match (&a.output, a.in_place) {
                (None, false) if is_project(&a.input) => Err(("bake", UsageError::BakeInPlace { path: a.input.clone() })),
                (None, _) if !is_project(&a.input) => Err(("bake", UsageError::BakeNeedsOutput)),
                (Some(out), false) => would_replace_project("-o", "INPUT", out, &a.input, true).map_err(|e| ("bake", e)),
                (Some(out), true) if !same_project(out, &a.input) => Err(("bake", UsageError::InPlaceNotUsed)),
                _ => Ok(()),
            },
            Command::Render(a) => would_replace("OUT.PNG", "INPUT", &a.output, &a.input, false).map_err(|e| ("render", e)),
            Command::New(a) => would_replace_project("OUT.TCRAFT", "INPUT", &a.output, &a.input, false).map_err(|e| ("new", e)),
            Command::Batch(a) => batch_would_replace(a).map_err(|e| ("batch", e)),
            Command::Mcp(a) if a.bridge.is_some() => match (&a.roots.read_root, &a.roots.write_root) {
                (Some(_), _) => Err(("mcp", UsageError::RootsWithBridge { flag: "--read-root" })),
                (_, Some(_)) => Err(("mcp", UsageError::RootsWithBridge { flag: "--write-root" })),
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }
}

/// Parses the arguments (without the program name). Help and the version come back as errors too,
/// which print to stdout and exit 0.
pub(crate) fn parse(args: &[OsString]) -> Result<Cli, clap::Error> {
    let matches = crate::command().try_get_matches_from(std::iter::once(OsString::from("texturecraft-cli")).chain(args.iter().cloned()))?;
    let cli = Cli::from_arg_matches(&matches)?;
    // An inherited TEXTURECRAFT_CONTROL_TOKEN must not force --bridge: the variable is how a
    // client authenticates when --bridge is already set. Only an explicit --token flag requires it.
    let token_flag_without_bridge = matches
        .subcommand_matches("mcp")
        .is_some_and(|mcp| mcp.value_source("token") == Some(clap::parser::ValueSource::CommandLine) && mcp.get_one::<u16>("bridge").is_none());
    if token_flag_without_bridge {
        return Err(UsageError::TokenNeedsBridge.into_clap("mcp"));
    }
    cli.command.check().map_err(|(sub, e)| e.into_clap(sub))?;
    Ok(cli)
}
