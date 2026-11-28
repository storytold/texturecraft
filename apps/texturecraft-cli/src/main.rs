//! Headless TextureCraft command line. Never opens a window. See the library crate for the commands.
#![forbid(unsafe_code)]

fn main() {
    // `args_os`, not `args`: a non-Unicode argument must reach clap, not panic before anything is printed
    let code = texturecraft_cli::run(std::env::args_os().skip(1), &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
