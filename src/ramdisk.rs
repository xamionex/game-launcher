//! Optional RAM-disk staging of the game directory (the `-R` flag).
//!
//! Faithful port of the shell functions; all privileged steps shell out to
//! `sudo` exactly as before and only run when `-R` is set.

use std::path::PathBuf;
use std::process::Command;

use crate::config::{App, RAM_MOUNT};
use crate::logging::append_line;

/// Resolve the log target for ramdisk messages: the active log, else
/// `$HOME/logs/ramdisk.log` (matching `${LOG_FILE:-$HOME/logs/ramdisk.log}`).
fn ram_log_path(app: &App) -> PathBuf {
    if let Some(log) = &app.log_file {
        return log.clone();
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("logs").join("ramdisk.log")
}

fn ram_log(app: &App, msg: &str) {
    append_line(&ram_log_path(app), msg);
}

/// Convert a size string like `32G` / `512M` / `1024K` to bytes.
fn parse_size(s: &str) -> u64 {
    if let Some(num) = s.strip_suffix('G') {
        num.parse::<u64>().unwrap_or(0) * 1024 * 1024 * 1024
    } else if let Some(num) = s.strip_suffix('M') {
        num.parse::<u64>().unwrap_or(0) * 1024 * 1024
    } else {
        s.strip_suffix('K').unwrap_or(s).parse::<u64>().unwrap_or(0)
    }
}

/// Verify the game fits in the RAM disk (with a 10% buffer).
fn check_ramdisk_size(app: &App, game_dir: &str) -> bool {
    let ramdisk_size = if app.ramdisk_size.is_empty() {
        "16G".to_string()
    } else {
        app.ramdisk_size.clone()
    };
    let ram_bytes = parse_size(&ramdisk_size);

    let game_size = Command::new("du")
        .arg("-sb")
        .arg(game_dir)
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
        });

    let Some(game_size) = game_size else {
        ram_log(app, "[RAMDISK] WARNING: Could not determine game size");
        return false;
    };

    let required = game_size * 110 / 100;
    if required > ram_bytes {
        ram_log(app, "[RAMDISK] ERROR: Game too large for RAM disk");
        ram_log(
            app,
            &format!(
                "[RAMDISK] Game size: {}MB, RAM disk: {}MB",
                game_size / 1024 / 1024,
                ram_bytes / 1024 / 1024
            ),
        );
        return false;
    }

    ram_log(
        app,
        &format!(
            "[RAMDISK] Size check passed: Game {}MB, RAM {}MB",
            game_size / 1024 / 1024,
            ram_bytes / 1024 / 1024
        ),
    );
    true
}

fn is_mountpoint(path: &str) -> bool {
    Command::new("mountpoint")
        .arg("-q")
        .arg(path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Stage the current game directory into a tmpfs RAM disk and bind-mount it.
pub fn create_ramdisk(app: &mut App) {
    let ramdisk = RAM_MOUNT;
    let original_pwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    ram_log(app, "[RAMDISK] Starting setup...");
    ram_log(app, &format!("[RAMDISK] Current path: {original_pwd}"));

    if !original_pwd.contains("/common/") {
        ram_log(
            app,
            &format!("[RAMDISK] ERROR: Could not find 'common' in path ({original_pwd})"),
        );
        return;
    }

    let (prefix, rest) = original_pwd.split_once("/common/").unwrap_or(("", ""));
    let game_name = rest.split('/').next().unwrap_or("");
    let game_dir = format!("{prefix}/common/{game_name}");

    ram_log(app, &format!("[RAMDISK] Detected game: {game_name}"));
    ram_log(app, &format!("[RAMDISK] Full path: {game_dir}"));

    if !app.use_ramdisk {
        ram_log(app, "[RAMDISK] Disabled by flag, skipping setup");
        return;
    }

    if !check_ramdisk_size(app, &game_dir) {
        ram_log(app, "[RAMDISK] Game too large for RAM disk, disabling");
        return;
    }

    if !is_mountpoint(ramdisk) {
        ram_log(app, &format!("[RAMDISK] Mounting tmpfs at {ramdisk}..."));
        let _ = Command::new("sudo").args(["mkdir", "-p", ramdisk]).status();
        let opts = format!(
            "size={},uid={},gid={}",
            app.ramdisk_size,
            unsafe { libc::getuid() },
            unsafe { libc::getgid() }
        );
        let ok = Command::new("sudo")
            .args(["mount", "-t", "tmpfs", "-o", &opts, "tmpfs", ramdisk])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            ram_log(app, "[RAMDISK] ERROR: Failed to mount tmpfs");
            return;
        }
    } else {
        ram_log(app, "[RAMDISK] RAM disk already mounted");
    }

    let ram_game_dir = format!("{ramdisk}/{game_name}");
    ram_log(app, &format!("[RAMDISK] Copying game to RAM: {ram_game_dir}"));
    let _ = std::fs::create_dir_all(&ram_game_dir);

    let _ = Command::new("sudo")
        .args([
            "rsync",
            "-aAXH",
            "--info=progress2",
            &format!("{game_dir}/"),
            &format!("{ram_game_dir}/"),
        ])
        .status();

    ram_log(app, "[RAMDISK] Creating bind mount...");
    let _ = Command::new("sudo")
        .args(["mount", "--bind", &ram_game_dir, &game_dir])
        .status();

    std::env::set_var("GAME_DIR_ORIG", &game_dir);
    std::env::set_var("GAME_DIR_RAM", &ram_game_dir);
    std::env::set_var("ORIGINAL_PWD", &original_pwd);
    app.game_dir_orig = Some(game_dir);
    app.game_dir_ram = Some(ram_game_dir);

    ram_log(app, &format!("[RAMDISK] Setup complete: {game_name} -> RAM"));
}

/// Sync the RAM disk back and unmount it (best effort, with lazy fallback).
pub fn sync_back_from_ramdisk(app: &App) {
    let ramdisk = RAM_MOUNT;
    ram_log(app, "[RAMDISK] Starting cleanup...");

    if !is_mountpoint(ramdisk) {
        ram_log(
            app,
            &format!("[RAMDISK] Nothing to unmount ({ramdisk} not mounted)"),
        );
        return;
    }

    ram_log(app, &format!("[RAMDISK] Syncing data and unmounting {ramdisk}..."));
    let _ = Command::new("sync").status();

    unmount(app, ramdisk);
    if let Some(orig) = &app.game_dir_orig {
        unmount(app, orig);
    }

    ram_log(app, "[RAMDISK] Cleanup completed");
}

/// Unmount `target`, falling back to a lazy unmount on failure.
fn unmount(app: &App, target: &str) {
    let ok = Command::new("sudo")
        .args(["umount", target])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        ram_log(app, &format!("[RAMDISK] Successfully unmounted {target}"));
        return;
    }
    ram_log(app, "[RAMDISK] Normal unmount failed, trying lazy unmount...");
    let lazy = Command::new("sudo")
        .args(["umount", "-l", target])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if lazy {
        ram_log(app, "[RAMDISK] Lazy unmount succeeded");
    } else {
        ram_log(app, &format!("[RAMDISK] Failed to unmount {target}"));
    }
}
