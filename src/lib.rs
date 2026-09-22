//! Rust Steam launch wrapper port from a bash script.

pub mod args;
pub mod command;
pub mod config;
pub mod config_file;
pub mod eos;
pub mod exec;
pub mod logging;
pub mod payload;
pub mod ramdisk;
pub mod tui;
pub mod wrappers;

use config::App;

/// Parse arguments, build the launch command, and run the game.
///
/// Returns the process exit code.
/// Unlike the shell version (whose exit code was that of the trailing `sync_back_from_ramdisk`, effectively always 0), this propagates the game's own exit code so Steam observes crashes.
pub fn run() -> i32 {
    let argv: Vec<String> = std::env::args().collect();
    let prog = argv.first().cloned().unwrap_or_else(|| "game".to_string());
    let rest: &[String] = if argv.len() > 1 { &argv[1..] } else { &[] };

    // The config file seeds the defaults; the command line overrides it.
    let mut app = App::default();
    let config_notes = config_file::load(&mut app);

    if rest.is_empty() {
        // Bare invocation: show the usage. Nothing is wrong yet, so this one
        // does not notify.
        args::print_help(&prog);
        return 1;
    }

    match args::parse_flags(&mut app, rest) {
        Ok(()) => {}
        Err(args::ParseError::Usage) => {
            args::notify_invalid_usage();
            args::print_help(&prog);
            return 1;
        }
        Err(args::ParseError::Invalid(msg)) => {
            args::notify_invalid_usage();
            eprintln!("{msg}");
            return 1;
        }
    }

    // `-C` is an action: edit the config, then exit. Any command is ignored.
    if app.config_tui {
        return match tui::run() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("Config editor: {e}");
                1
            }
        };
    }

    if app.logging_level >= 0 {
        logging::setup_logging(&mut app);
    }
    for note in &config_notes {
        app.log(note);
    }

    command::determine_proton(&mut app);
    wrappers::determine_wayland_by_gpu(&mut app);

    if let Err(msg) = command::build_command(&mut app) {
        args::notify_invalid_usage();
        eprintln!("Error: {msg}");
        args::print_help(&prog);
        return 1;
    }

    eos::apply_eos_proxy(&app);
    wrappers::apply_wrappers(&mut app);
    command::setup_custom_vkd3d(&app);
    // RAM disk staging is disabled for now; the module is kept for later.
    // ramdisk::create_ramdisk(&mut app);
    exec::run_mods(&mut app);
    wrappers::apply_environment_modifications(&app);
    let code = exec::run_game(&mut app);
    // ramdisk::sync_back_from_ramdisk(&app);
    exec::cleanup(&app);
    code
}
