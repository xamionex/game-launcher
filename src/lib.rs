//! Rust port of `game.sh`: a Steam launch wrapper.
//!
//! Orchestration mirrors the original `main` ordering. Two intentional changes
//! relative to the shell version:
//!
//! 1. SteamLinuxRuntime stripping operates on the argument vector instead of a
//!    flattened, re-split string, so executables/paths containing spaces are
//!    preserved (the original bug that prevented such games from launching).
//! 2. `-v VAR=VALUE` is replaced by positional `KEY=VALUE` tokens
//!    (e.g. `game FLAG=1 -- %command%`), feeding the same export/logging path.

pub mod args;
pub mod command;
pub mod config;
pub mod exec;
pub mod logging;
pub mod ramdisk;
pub mod wrappers;

use config::App;

/// Parse arguments, build the launch command, and run the game.
///
/// Returns the process exit code. Unlike the shell version (whose exit code was
/// that of the trailing `sync_back_from_ramdisk`, effectively always 0), this
/// propagates the game's own exit code so Steam observes crashes.
pub fn run() -> i32 {
    let argv: Vec<String> = std::env::args().collect();
    let prog = argv.first().cloned().unwrap_or_else(|| "game".to_string());
    let rest: &[String] = if argv.len() > 1 { &argv[1..] } else { &[] };

    let mut app = App::default();
    match args::parse_flags(&mut app, rest) {
        Ok(()) => {}
        Err(args::ParseError::Usage) => {
            args::print_help(&prog);
            return 1;
        }
        Err(args::ParseError::Invalid(msg)) => {
            eprintln!("{msg}");
            return 1;
        }
    }

    if app.logging_level >= 0 {
        logging::setup_logging(&mut app);
    }

    command::determine_proton(&mut app);
    wrappers::determine_wayland_by_gpu(&mut app);

    if let Err(msg) = command::build_command(&mut app) {
        eprintln!("Error: {msg}");
        args::print_help(&prog);
        return 1;
    }

    wrappers::apply_wrappers(&mut app);
    command::setup_custom_vkd3d(&app);
    ramdisk::create_ramdisk(&mut app);
    exec::run_mods(&mut app);
    wrappers::apply_environment_modifications(&app);
    let code = exec::run_game(&mut app);
    ramdisk::sync_back_from_ramdisk(&app);
    exec::cleanup(&app);
    code
}
