//! Log file setup, banner separators, the structured launch dump, and the
//! consecutive-line de-duplication compressor.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;

use crate::config::{self, App};

/// Run `date` with the given format args and return the trimmed output.
fn run_date(args: &[&str]) -> String {
    Command::new("date")
        .args(args)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim_end().to_string())
        .unwrap_or_default()
}

/// `${VAR:-default}` semantics: use `default` when unset or empty.
fn env_or(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => v,
        _ => default.to_string(),
    }
}

/// Locate an executable on `PATH`, like `command -v`.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Recursively search `.` for `filename`, returning the first match.
fn find_first(filename: &str) -> Option<PathBuf> {
    let mut stack = vec![PathBuf::from(".")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().map(|n| n == filename).unwrap_or(false) {
                return Some(path);
            }
        }
    }
    None
}

/// Resolve the Steam App ID from the command line or a `steam_appid.txt` file.
fn get_appid(app: &App) -> Option<String> {
    let re = Regex::new(r"AppId=([0-9]+)").ok()?;
    for arg in &app.original_cmd {
        if let Some(caps) = re.captures(arg) {
            return Some(caps[1].to_string());
        }
    }
    if let Some(file) = find_first("steam_appid.txt") {
        if let Ok(contents) = std::fs::read_to_string(&file) {
            let num_re = Regex::new(r"[0-9]+").ok()?;
            if let Some(m) = num_re.find(&contents) {
                return Some(m.as_str().to_string());
            }
        }
    }
    None
}

/// Derive a human-friendly game name from the current directory, falling back
/// to the trailing command's basename (extension stripped).
pub fn derive_game_name(app: &App) -> String {
    match std::env::current_dir() {
        Ok(dir) if !dir.as_os_str().is_empty() => dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        _ => app
            .original_cmd
            .last()
            .map(|last| {
                let trimmed = last.strip_prefix("./").unwrap_or(last);
                let base = Path::new(trimmed)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| trimmed.to_string());
                match base.rsplit_once('.') {
                    Some((stem, _)) => stem.to_string(),
                    None => base,
                }
            })
            .unwrap_or_default(),
    }
}

/// Resolve the Steam App ID from env vars, then the command line / appid file.
pub fn resolve_appid(app: &App) -> String {
    for key in ["SteamAppId", "STEAM_APPID", "APPID"] {
        let v = env_or(key, "");
        if !v.is_empty() {
            return v;
        }
    }
    get_appid(app).unwrap_or_default()
}

/// Create a per-game log folder and a fresh timestamped log file inside it,
/// then archive older logs.
///
/// Layout: `$HOME/logs/game/<appid name>/<timestamp>.log`.
/// At most [`config::MAX_LOGS`] plain `.log` files are kept per folder; older
/// ones are compressed to `<name>.tar.gz` and removed.
pub fn setup_logging(app: &mut App) {
    let base = config::log_base();
    let _ = std::fs::create_dir_all(&base);

    let game_name = derive_game_name(app);
    app.appid = resolve_appid(app);
    app.game_name = game_name.clone();

    let base_filename = if !app.appid.is_empty() {
        format!("{} {}", app.appid, game_name)
    } else {
        game_name.clone()
    };

    // One folder per appid (or per process name when there is no appid).
    let folder = base.join(sanitize_component(&base_filename));
    let _ = std::fs::create_dir_all(&folder);

    let date = run_date(&["+%Y%m%d_%H%M%S"]);
    let log_file = folder.join(format!("{date}.log"));
    let _ = OpenOptions::new().create(true).append(true).open(&log_file);

    rotate_and_archive(&folder);

    app.log_file = Some(log_file);
}

/// Replace path separators so a name can be safely used as a folder component.
fn sanitize_component(name: &str) -> String {
    name.replace('/', "_")
}

/// Keep the newest [`config::MAX_LOGS`] `.log` files in `folder`; archive each
/// older one to `<name>.tar.gz` and remove the original.
fn rotate_and_archive(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().map(|x| x == "log").unwrap_or(false) {
                let mtime = e
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                Some((mtime, path))
            } else {
                None
            }
        })
        .collect();

    // Newest first; archive everything past MAX_LOGS.
    logs.sort_by_key(|m| std::cmp::Reverse(m.0));
    for (_, path) in logs.into_iter().skip(config::MAX_LOGS) {
        archive_log(folder, &path);
    }
}

/// Compress a single log into `<name>.tar.gz` next to it, then delete the log.
///
/// On any failure the original log is left untouched (no data loss).
fn archive_log(folder: &Path, log: &Path) {
    let Some(name) = log.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let archive = folder.join(format!("{name}.tar.gz"));

    let ok = Command::new("tar")
        .arg("-czf")
        .arg(&archive)
        .arg("-C")
        .arg(folder)
        .arg(name)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if ok {
        let _ = std::fs::remove_file(log);
    } else {
        let _ = std::fs::remove_file(&archive);
    }
}

const LINE_WIDTH: usize = 100;

/// Build a centered banner line using the first character of `symbol`.
pub fn separator_line(symbol: char, text: &str) -> String {
    if text.is_empty() {
        return symbol.to_string().repeat(LINE_WIDTH);
    }

    // Strip ANSI color codes for width calculation.
    let ansi = Regex::new("\u{1b}\\[[0-9;]*m").unwrap();
    let clean = ansi.replace_all(text, "");
    let text_len = clean.chars().count();
    let total = text_len + 2;

    if total > LINE_WIDTH {
        return text.to_string();
    }

    let space = LINE_WIDTH - total;
    let left = space.div_ceil(2);
    let right = space - left;
    format!(
        "{} {} {}",
        symbol.to_string().repeat(left),
        text,
        symbol.to_string().repeat(right)
    )
}

/// Append a line to a log file, ignoring errors (matching `echo >> file`).
pub fn append_line(path: &Path, line: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// Write a banner separator to a log file.
pub fn log_separator(path: &Path, symbol: char, text: &str) {
    append_line(path, &separator_line(symbol, text));
}

/// Filtered environment dump prefixes (mirrors the original grep alternation).
fn env_filter_match(key: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "STEAM", "PROTON", "WINE", "DXVK", "VK_", "LD_", "MANGOHUD", "GAMEMODE", "GAME",
    ];
    PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Write the full structured launch dump to the active log file.
pub fn log_game(app: &App) {
    let Some(log) = app.log_file.clone() else {
        return;
    };
    let log = log.as_path();

    log_separator(log, '=', &format!("LAUNCHING GAME: {}", run_date(&[])));

    log_separator(log, '=', "ORIGINAL COMMAND");
    append_line(log, &app.original_cmd.join(" "));

    log_separator(log, '=', "MODIFIED COMMAND");
    append_line(log, &app.cmd.join(" "));

    log_separator(log, '=', "PRESSURE VESSEL STATUS");
    if app.pressure_vessel && !app.isproton {
        append_line(log, "Pressure Vessel: STRIPPED");
    } else {
        append_line(log, "Pressure Vessel: DEFAULT");
    }

    if app.isproton {
        log_separator(log, '=', "PROTON / WINE INFO");
        let proton_path = if app.proton_path.is_empty() {
            "'(unknown)'".to_string()
        } else {
            app.proton_path.clone()
        };
        let proton_ver = if app.proton_ver.is_empty() {
            "'(unknown)'".to_string()
        } else {
            app.proton_ver.clone()
        };
        append_line(log, &format!("Proton Path      : {proton_path}"));
        append_line(log, &format!("Proton Version   : {proton_ver}"));
        append_line(
            log,
            &format!("WINEPREFIX       : {}", env_or("WINEPREFIX", "'(unset)'")),
        );
        append_line(
            log,
            &format!(
                "WINEDLLOVERRIDES : {}",
                env_or("WINEDLLOVERRIDES", "'(unset)'")
            ),
        );
        append_line(
            log,
            &format!("WINEESYNC        : {}", env_or("WINEESYNC", "'(unset)'")),
        );
        append_line(
            log,
            &format!("WINEFSYNC        : {}", env_or("WINEFSYNC", "'(unset)'")),
        );
        append_line(
            log,
            &format!("WINE_NTSYNC      : {}", env_or("WINE_NTSYNC", "'(unset)'")),
        );
        append_line(
            log,
            &format!("WINEDEBUG        : {}", env_or("WINEDEBUG", "'(unset)'")),
        );
    }

    if !app.custom_exports.is_empty() {
        log_separator(log, '=', "MODIFICATIONS TO ENVIRONMENT");
        for export in &app.custom_exports {
            let current = env_or(&export.name, "'(empty)'");
            append_line(log, &format!("{}:", export.name));
            append_line(log, &format!("  BEFORE: {}", export.before));
            append_line(log, &format!("  AFTER : {current}"));
        }
    }

    log_separator(log, '=', "RUNTIME ENVIRONMENT");
    append_line(
        log,
        &format!("LD_PRELOAD        : {}", env_or("LD_PRELOAD", "'(unset)'")),
    );
    append_line(
        log,
        &format!(
            "LD_LIBRARY_PATH   : {}",
            env_or("LD_LIBRARY_PATH", "'(unset)'")
        ),
    );
    append_line(
        log,
        &format!("DXVK_HUD          : {}", env_or("DXVK_HUD", "'(unset)'")),
    );
    append_line(
        log,
        &format!(
            "VK_INSTANCE_LAYERS: {}",
            env_or("VK_INSTANCE_LAYERS", "'(unset)'")
        ),
    );
    append_line(
        log,
        &format!(
            "DXVK_LOG_LEVEL    : {}",
            env_or("DXVK_LOG_LEVEL", "'(unset)'")
        ),
    );
    append_line(log, &format!("MangoHud Enabled  : {}", app.mangohud));
    append_line(log, &format!("GameMode Enabled  : {}", app.gamemode));
    append_line(
        log,
        &format!("vkBasalt Enabled  : {}", env_or("ENABLE_VKBASALT", "false")),
    );

    log_separator(log, '=', "STEAM / COMPATIBILITY");
    let steam_exe = which("steam")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "not found".to_string());
    append_line(
        log,
        &format!(
            "Steam App ID           : {}",
            if app.appid.is_empty() {
                "'(unset)'".to_string()
            } else {
                app.appid.clone()
            }
        ),
    );
    append_line(log, &format!("Steam Executable       : {steam_exe}"));
    append_line(
        log,
        &format!(
            "Steam Compat Tool Path : {}",
            env_or("STEAM_COMPAT_CLIENT_INSTALL_PATH", "'(unset)'")
        ),
    );
    append_line(
        log,
        &format!(
            "Steam Compat Data Path : {}",
            env_or("STEAM_COMPAT_DATA_PATH", "'(unset)'")
        ),
    );
    append_line(
        log,
        &format!(
            "Steam Runtime          : {}",
            env_or("STEAM_RUNTIME", "'(unset)'")
        ),
    );

    log_separator(log, '=', "SYSTEM INFO");
    let kernel = Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim_end().to_string())
        .unwrap_or_default();
    append_line(log, &format!("Kernel             : {kernel}"));
    append_line(
        log,
        &format!("Locale             : {}", env_or("LANG", "'(unset)'")),
    );
    append_line(
        log,
        &format!(
            "Desktop Environment: {}",
            env_or("XDG_CURRENT_DESKTOP", "'(unset)'")
        ),
    );
    append_line(
        log,
        &format!(
            "Session Type       : {}",
            env_or("XDG_SESSION_TYPE", "'(unset)'")
        ),
    );
    let wayland_display = env_or("WAYLAND_DISPLAY", "");
    let display = env_or("DISPLAY", "'(none)'");
    append_line(
        log,
        &format!("Display Server     : {wayland_display}{display}"),
    );

    log_separator(log, '=', "GPU / DRIVER INFO");
    append_line(log, &format!("GPU(s)   : {}", gpu_list()));
    append_line(log, &format!("Driver   : {}", driver_info()));

    log_separator(log, '=', "FILTERED ENVIRONMENT VARIABLES");
    for (key, value) in std::env::vars() {
        if env_filter_match(&key) {
            append_line(log, &format!("{key}={value}"));
        }
    }

    log_separator(log, '=', "LOG FILE INFO");
    append_line(log, &format!("Log File: {}", log.to_string_lossy()));

    log_separator(log, '=', "PWD");
    let pwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    append_line(log, &format!("Current Directory: {pwd}"));
    if let Ok(prefix) = std::env::var("WINEPREFIX") {
        if !prefix.is_empty() {
            append_line(
                log,
                &format!("Expected Proton log (if enabled): {prefix}/steam-*.log"),
            );
        }
    }
}

/// `lspci | grep -E 'VGA|3D'` (case sensitive), or `not detected`.
fn gpu_list() -> String {
    let output = Command::new("lspci").output();
    let Ok(output) = output else {
        return "not detected".to_string();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("VGA") || l.contains("3D"))
        .collect();
    if lines.is_empty() {
        "not detected".to_string()
    } else {
        lines.join("\n")
    }
}

/// `glxinfo | grep 'OpenGL version'`, or `glxinfo not available`.
fn driver_info() -> String {
    let output = Command::new("glxinfo").output();
    let Ok(output) = output else {
        return "glxinfo not available".to_string();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("OpenGL version"))
        .collect();
    if lines.is_empty() {
        "glxinfo not available".to_string()
    } else {
        lines.join("\n")
    }
}

/// Streaming compressor that collapses consecutive duplicate lines.
///
/// Level 0 compares whole lines; level 1 ignores the leading `[timestamp]`
/// prefix when comparing but preserves the first occurrence's timestamp.
pub struct Compressor {
    level: i32,
    has_prev: bool,
    count: u64,
    // level 0
    last: String,
    // level 1
    last_ts: String,
    last_msg: String,
}

impl Compressor {
    pub fn new(level: i32) -> Self {
        Compressor {
            level,
            has_prev: false,
            count: 0,
            last: String::new(),
            last_ts: String::new(),
            last_msg: String::new(),
        }
    }

    fn split_ts(line: &str) -> (String, String) {
        match line.find(']') {
            Some(idx) => (line[..=idx].to_string(), line[idx + 1..].to_string()),
            None => (String::new(), line.to_string()),
        }
    }

    fn flush0(&self) -> String {
        if self.count == 1 {
            self.last.clone()
        } else {
            format!("[x{}] {}", self.count, self.last)
        }
    }

    fn flush1(&self) -> String {
        if self.count == 1 {
            format!("{}{}", self.last_ts, self.last_msg)
        } else {
            format!("{} [x{}] {}", self.last_ts, self.count, self.last_msg)
        }
    }

    /// Feed one line; returns any completed (flushed) output lines.
    pub fn push(&mut self, line: &str) -> Vec<String> {
        if self.level == 0 {
            if self.has_prev && line == self.last {
                self.count += 1;
                return Vec::new();
            }
            let mut out = Vec::new();
            if self.has_prev {
                out.push(self.flush0());
            }
            self.last = line.to_string();
            self.count = 1;
            self.has_prev = true;
            out
        } else if self.level == 1 {
            let (ts, msg) = Self::split_ts(line);
            if self.has_prev && msg == self.last_msg {
                self.count += 1;
                return Vec::new();
            }
            let mut out = Vec::new();
            if self.has_prev {
                out.push(self.flush1());
            }
            self.last_ts = ts;
            self.last_msg = msg;
            self.count = 1;
            self.has_prev = true;
            out
        } else {
            vec![line.to_string()]
        }
    }

    /// Emit the final buffered group.
    pub fn finish(&mut self) -> Vec<String> {
        if !self.has_prev {
            return Vec::new();
        }
        let out = if self.level == 1 {
            self.flush1()
        } else {
            self.flush0()
        };
        self.has_prev = false;
        vec![out]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a compressor over a slice of lines and collect all output.
    fn run(level: i32, lines: &[&str]) -> Vec<String> {
        let mut comp = Compressor::new(level);
        let mut out = Vec::new();
        for line in lines {
            out.extend(comp.push(line));
        }
        out.extend(comp.finish());
        out
    }

    #[test]
    fn separator_is_full_width_and_centered() {
        let line = separator_line('=', "");
        assert_eq!(line.chars().count(), LINE_WIDTH);

        let titled = separator_line('=', "TITLE");
        // " TITLE " plus padding fills the whole width.
        assert_eq!(titled.chars().count(), LINE_WIDTH);
        assert!(titled.contains(" TITLE "));
    }

    #[test]
    fn compress_level0_collapses_duplicates() {
        let out = run(0, &["a", "a", "a", "b", "c", "c"]);
        assert_eq!(out, vec!["[x3] a", "b", "[x2] c"]);
    }

    #[test]
    fn compress_level0_single_lines_unchanged() {
        let out = run(0, &["x", "y", "z"]);
        assert_eq!(out, vec!["x", "y", "z"]);
    }

    #[test]
    fn compress_level1_dedups_by_message_keeping_first_timestamp() {
        let out = run(
            1,
            &["[0000001.0] boot", "[0000002.0] boot", "[0000003.0] done"],
        );
        assert_eq!(out, vec!["[0000001.0] [x2]  boot", "[0000003.0] done"]);
    }

    #[test]
    fn rotation_keeps_three_logs_and_archives_older() {
        use std::time::{Duration, UNIX_EPOCH};

        let dir = std::env::temp_dir().join(format!("game_rot_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Five logs with strictly increasing modification times.
        let base = UNIX_EPOCH + Duration::from_secs(1_000_000);
        for i in 0..5u64 {
            let path = dir.join(format!("log{i}.log"));
            std::fs::write(&path, format!("content {i}")).unwrap();
            let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            f.set_modified(base + Duration::from_secs(i)).unwrap();
        }

        rotate_and_archive(&dir);

        let count_logs = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().map(|x| x == "log").unwrap_or(false))
            .count();
        assert_eq!(count_logs, config::MAX_LOGS); // 3 newest kept

        // Oldest two are gone and archived.
        assert!(!dir.join("log0.log").exists());
        assert!(!dir.join("log1.log").exists());
        assert!(dir.join("log2.log").exists());
        assert!(dir.join("log0.log.tar.gz").exists());
        assert!(dir.join("log1.log.tar.gz").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
