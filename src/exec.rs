//! Game and mod execution: the three logging modes, exit-code propagation,
//! crash renaming, background mods, signal handling, and cleanup.

use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::Instant;

use crate::config::{self, App};
use crate::logging::{self, append_line, log_separator, Compressor};

/// Outcome of attempting to run the launch command.
enum RunOutcome {
    /// The command ran; carries its shell-style exit code.
    Ran(i32),
    /// The wrapper itself could not start the command (a `game` project error).
    SpawnFailed(String),
}

/// Convert an exit status to a shell-style code (128 + signal when killed).
fn status_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        code
    } else if let Some(sig) = status.signal() {
        128 + sig
    } else {
        1
    }
}

/// Reset SIGINT/SIGTERM to default in the child so it still responds to Ctrl-C
/// even though the parent ignores those signals.
fn set_child_signals(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            Ok(())
        });
    }
}

/// Ignore SIGINT/SIGTERM in the parent so it survives to run cleanup/sync-back,
/// mirroring how bash keeps running while waiting on a foreground child.
fn ignore_signals_in_parent() {
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }
}

/// Spawn a command with stdout and stderr merged into one readable pipe.
fn spawn_merged(cmd: &[String]) -> std::io::Result<(os_pipe::PipeReader, Child)> {
    let (reader, writer) = os_pipe::pipe()?;
    let writer_clone = writer.try_clone()?;
    let mut command = Command::new(&cmd[0]);
    command.args(&cmd[1..]);
    command.stdout(writer);
    command.stderr(writer_clone);
    set_child_signals(&mut command);
    let child = command.spawn()?;
    Ok((reader, child))
}

/// Read a reader line by line (binary-safe via lossy UTF-8), invoking `f` per
/// line with the trailing newline removed.
fn for_each_line<R: Read>(reader: R, mut f: impl FnMut(&str)) {
    let mut buf_reader = BufReader::new(reader);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        match buf_reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                while matches!(buf.last(), Some(b'\n') | Some(b'\r')) {
                    buf.pop();
                }
                let line = String::from_utf8_lossy(&buf);
                f(&line);
            }
            Err(_) => break,
        }
    }
}

/// Strip a trailing `.log` from a log path, returning the stem as a string.
fn log_stem(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.strip_suffix(".log").unwrap_or(&s).to_string()
}

/// Run the game, returning its exit code. Honors the three logging levels.
pub fn run_game(app: &mut App) -> i32 {
    let log = app.log_file.clone();

    if app.logging_level >= 0 {
        logging::log_game(app);
        if let Some(log) = &log {
            log_separator(log, '=', "\u{1f680} GAME START \u{1f680}");
        }
    }

    ignore_signals_in_parent();

    let outcome = match app.logging_level {
        -1 => run_direct(app),
        1 => run_verbose(app),
        _ => run_normal(app),
    };

    let exit_status = match &outcome {
        RunOutcome::Ran(code) => *code,
        RunOutcome::SpawnFailed(_) => 127,
    };

    if app.logging_level >= 0 {
        if let Some(log) = &log {
            if let RunOutcome::SpawnFailed(err) = &outcome {
                append_line(log, &format!("Launch error: {err}"));
            }
            log_separator(log, '=', "\u{1f6d1} GAME EXITED \u{1f6d1}");
            append_line(log, &format!("Exit Status: {exit_status}"));
        }
        if exit_status != 0 {
            if let Some(old) = app.log_file.clone() {
                let new_name =
                    PathBuf::from(format!("{} (CRASHED: {}).log", log_stem(&old), exit_status));
                if std::fs::rename(&old, &new_name).is_ok() {
                    app.log_file = Some(new_name);
                }
            }
        }
    }

    notify_on_failure(app, &outcome);

    exit_status
}

/// Notify the user when the game crashed, distinguishing a game crash from a
/// `game` (wrapper/project) launch failure.
fn notify_on_failure(app: &App, outcome: &RunOutcome) {
    let name = if app.game_name.is_empty() {
        logging::derive_game_name(app)
    } else {
        app.game_name.clone()
    };
    let name = if name.is_empty() {
        "the game".to_string()
    } else {
        name
    };

    match outcome {
        RunOutcome::SpawnFailed(err) => {
            config::notify("game: launch failed", &format!("Could not start {name}: {err}"));
        }
        RunOutcome::Ran(code) if *code != 0 => {
            config::notify("Game crashed", &format!("{name} exited with code {code}"));
        }
        RunOutcome::Ran(_) => {}
    }
}

/// Level -1: run with inherited stdio, no capture.
fn run_direct(app: &App) -> RunOutcome {
    let mut command = Command::new(&app.cmd[0]);
    command.args(&app.cmd[1..]);
    command.stdin(Stdio::inherit());
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());
    set_child_signals(&mut command);
    match command.status() {
        Ok(status) => RunOutcome::Ran(status_code(status)),
        Err(e) => RunOutcome::SpawnFailed(e.to_string()),
    }
}

/// Level 0: capture, de-duplicate, append to log.
fn run_normal(app: &App) -> RunOutcome {
    let Some(log) = app.log_file.clone() else {
        return run_direct(app);
    };
    let (reader, mut child) = match spawn_merged(&app.cmd) {
        Ok(pair) => pair,
        Err(e) => return RunOutcome::SpawnFailed(e.to_string()),
    };

    let mut logf = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .ok();
    let mut comp = Compressor::new(0);
    for_each_line(reader, |line| {
        for out in comp.push(line) {
            if let Some(f) = logf.as_mut() {
                let _ = writeln!(f, "{out}");
            }
        }
    });
    for out in comp.finish() {
        if let Some(f) = logf.as_mut() {
            let _ = writeln!(f, "{out}");
        }
    }

    match child.wait() {
        Ok(status) => RunOutcome::Ran(status_code(status)),
        Err(_) => RunOutcome::Ran(1),
    }
}

/// Level 1: capture, prepend elapsed timestamp, tee to terminal, de-duplicate
/// by message, append to log.
fn run_verbose(app: &App) -> RunOutcome {
    let Some(log) = app.log_file.clone() else {
        return run_direct(app);
    };
    let (reader, mut child) = match spawn_merged(&app.cmd) {
        Ok(pair) => pair,
        Err(e) => return RunOutcome::SpawnFailed(e.to_string()),
    };

    let mut tty: Box<dyn Write> = if std::io::stdout().is_terminal() {
        std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/tty")
            .map(|f| Box::new(f) as Box<dyn Write>)
            .unwrap_or_else(|_| Box::new(std::io::stdout()))
    } else {
        Box::new(std::io::stdout())
    };

    let mut logf = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .ok();
    let mut comp = Compressor::new(1);
    let start = Instant::now();

    for_each_line(reader, |line| {
        let elapsed = start.elapsed().as_secs_f64();
        let stamped = format!("[{elapsed:07.1}] {line}");
        let _ = writeln!(tty, "{stamped}");
        let _ = tty.flush();
        for out in comp.push(&stamped) {
            if let Some(f) = logf.as_mut() {
                let _ = writeln!(f, "{out}");
            }
        }
    });
    for out in comp.finish() {
        if let Some(f) = logf.as_mut() {
            let _ = writeln!(f, "{out}");
        }
    }

    match child.wait() {
        Ok(status) => RunOutcome::Ran(status_code(status)),
        Err(_) => RunOutcome::Ran(1),
    }
}

/// Launch each requested mod in the background, logging per mod when enabled.
pub fn run_mods(app: &mut App) {
    if app.mods_to_launch.is_empty() {
        return;
    }

    let log = app.log_file.clone();
    if app.logging_level >= 0 {
        if let Some(log) = &log {
            let date = Command::new("date")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim_end().to_string())
                .unwrap_or_default();
            log_separator(
                log,
                '=',
                &format!("\u{1f680} LAUNCHING UNIFIED MODS AT {date} \u{1f680}"),
            );
        }
    }

    let mods = app.mods_to_launch.clone();
    let level = app.logging_level;
    for (i, mod_cmd) in mods.iter().enumerate() {
        let counter = i + 1;
        if level >= 0 {
            let Some(log) = &log else { continue };
            let mod_log = PathBuf::from(format!("{} (MOD: {counter}).log", log_stem(log)));
            let _ = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&mod_log);

            log_separator(
                &mod_log,
                '=',
                &format!("\u{1f527} LAUNCHING MOD (MOD/{counter}) \u{1f527}"),
            );
            append_line(&mod_log, mod_cmd);
            log_separator(&mod_log, '=', "\u{1f4c4} LOG FILE INFO \u{1f4c4}");
            append_line(&mod_log, &format!("Log File: {}", mod_log.to_string_lossy()));

            if let Ok((reader, child)) = spawn_mod(mod_cmd) {
                let pid = child.id();
                app.mod_pids.push(pid);
                spawn_mod_logger(reader, mod_log.clone(), level);
                log_separator(
                    &mod_log,
                    '=',
                    &format!("\u{1f527} MOD/{counter} RUNNING \u{1f527}"),
                );
                append_line(&mod_log, &format!("MOD/{counter} PID: {pid}"));
                drop(child);
            }
        } else {
            let child = Command::new("bash")
                .arg("-c")
                .arg(mod_cmd)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if let Ok(c) = child {
                app.mod_pids.push(c.id());
            }
        }
    }
}

/// Spawn `bash -c <mod>` with merged output for logging.
fn spawn_mod(mod_cmd: &str) -> std::io::Result<(os_pipe::PipeReader, Child)> {
    let (reader, writer) = os_pipe::pipe()?;
    let writer_clone = writer.try_clone()?;
    let child = Command::new("bash")
        .arg("-c")
        .arg(mod_cmd)
        .stdin(Stdio::null())
        .stdout(writer)
        .stderr(writer_clone)
        .spawn()?;
    Ok((reader, child))
}

/// Detached thread that compresses a mod's output into its log file.
fn spawn_mod_logger(reader: os_pipe::PipeReader, mod_log: PathBuf, level: i32) {
    thread::spawn(move || {
        let mut logf = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&mod_log)
            .ok();
        let mut comp = Compressor::new(level);
        for_each_line(reader, |line| {
            for out in comp.push(line) {
                if let Some(f) = logf.as_mut() {
                    let _ = writeln!(f, "{out}");
                }
            }
        });
        for out in comp.finish() {
            if let Some(f) = logf.as_mut() {
                let _ = writeln!(f, "{out}");
            }
        }
    });
}

/// Kill tracked mod processes when `-e` (cleanup on exit) is set.
pub fn cleanup(app: &App) {
    if !app.cleanup_mods_on_exit {
        return;
    }
    if app.logging_level >= 0 {
        app.log("Cleaning up mod processes...");
    }
    for &pid in &app.mod_pids {
        unsafe {
            if libc::kill(pid as i32, 0) == 0 {
                libc::kill(pid as i32, libc::SIGTERM);
                if app.logging_level >= 0 {
                    app.log(&format!("Killed PID {pid}"));
                }
            }
        }
    }
}
