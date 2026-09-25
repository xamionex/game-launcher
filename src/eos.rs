//! EOS-Proxy automation (the `-E` flag).
//!
//! The proxy replaces the game's `EOSSDK-Win64-Shipping.dll`:
//! the original is renamed to `EOSSDK-Win64-Shipping.yes` (the proxy loads it back at runtime) and the proxy is written in its place.
//! A game that already has the `.yes` backup is left alone, and so is a game without the original dll, because the proxy cannot work without it.

use std::path::{Path, PathBuf};

use crate::config::App;
use crate::payload;

/// Upstream release asset; fetched into the payload cache on first use.
pub(crate) const DLL_URL: &str =
    "https://github.com/yesyes0649/eos-proxy/releases/latest/download/EOSSDK-Win64-Shipping.dll";
/// The dll shipped with games that use EOS, and the name the proxy must have.
pub(crate) const DLL_NAME: &str = "EOSSDK-Win64-Shipping.dll";
/// Backup name the original dll is renamed to; its presence means already applied.
const BACKUP_NAME: &str = "EOSSDK-Win64-Shipping.yes";
/// Copy embedded in the binary, used when the download fails.
const BUNDLED_DLL: &[u8] = include_bytes!("../EOSSDK-Win64-Shipping.dll");

/// What to do for one game folder.
#[derive(Debug, PartialEq)]
enum Plan {
    Skip(String),
    Apply { target: PathBuf, backup: PathBuf },
}

/// Decide whether the proxy still needs to be installed in `game_dir`.
fn plan(game_dir: &Path) -> Plan {
    let target = game_dir.join(DLL_NAME);
    let backup = game_dir.join(BACKUP_NAME);

    if backup.is_file() {
        return Plan::Skip(format!(
            "EOS: {BACKUP_NAME} already present in {}, skipping",
            game_dir.display()
        ));
    }
    if !target.is_file() {
        return Plan::Skip(format!(
            "EOS: no {DLL_NAME} in {}, skipping",
            game_dir.display()
        ));
    }
    Plan::Apply { target, backup }
}

/// Folder holding the game's dll: the exe's folder when it exists, otherwise the current directory.
/// The first candidate that actually contains the dll wins, so a game whose exe sits in a subfolder still resolves.
fn game_dir(app: &App) -> Option<PathBuf> {
    let exe_dir = app
        .original_cmd
        .iter()
        .find(|a| a.to_lowercase().ends_with(".exe"))
        .and_then(|exe| Path::new(exe).parent())
        .filter(|p| p.is_dir())
        .map(Path::to_path_buf);
    let cwd = std::env::current_dir().ok();

    for dir in [exe_dir.as_deref(), cwd.as_deref()].into_iter().flatten() {
        if dir.join(DLL_NAME).is_file() {
            return Some(dir.to_path_buf());
        }
    }
    exe_dir.or(cwd)
}

/// Install the EOS proxy for the current game when `-E` is set.
pub fn apply_eos_proxy(app: &App) {
    if !app.eos_proxy {
        return;
    }
    let Some(dir) = game_dir(app) else {
        app.log("EOS: could not determine the game folder, skipping");
        return;
    };

    match plan(&dir) {
        Plan::Skip(msg) => app.log(&msg),
        Plan::Apply { target, backup } => {
            let Some(bytes) = payload::load(app, DLL_NAME, DLL_URL, BUNDLED_DLL) else {
                app.log("EOS: no proxy dll available, skipping");
                return;
            };
            install(app, &target, &backup, &bytes);
        }
    }
}

/// Rename the original dll to `backup` and write the proxy to `target`.
///
/// The rename is undone when the proxy cannot be written, so a failure never leaves the game without its dll.
fn install(app: &App, target: &Path, backup: &Path, bytes: &[u8]) -> bool {
    if let Err(e) = std::fs::rename(target, backup) {
        app.log(&format!("EOS: failed to rename {}: {e}", target.display()));
        return false;
    }
    app.log(&format!(
        "EOS: renamed {} to {}",
        target.display(),
        backup.display()
    ));

    match std::fs::write(target, bytes) {
        Ok(()) => {
            app.log(&format!("EOS: installed proxy dll at {}", target.display()));
            true
        }
        Err(e) => {
            app.log(&format!("EOS: failed to write proxy dll: {e}"));
            if let Err(e) = std::fs::rename(backup, target) {
                app.log(&format!("EOS: failed to restore the original dll: {e}"));
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("game_eos_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn plan_skips_when_backup_exists() {
        let dir = scratch("backup");
        std::fs::write(dir.join(DLL_NAME), b"original").unwrap();
        std::fs::write(dir.join(BACKUP_NAME), b"proxy").unwrap();

        match plan(&dir) {
            Plan::Skip(msg) => assert!(msg.contains("already present")),
            other => panic!("expected Skip, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_skips_when_original_dll_is_missing() {
        let dir = scratch("missing");
        match plan(&dir) {
            Plan::Skip(msg) => assert!(msg.contains(DLL_NAME)),
            other => panic!("expected Skip, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_applies_when_only_the_original_dll_exists() {
        let dir = scratch("apply");
        std::fs::write(dir.join(DLL_NAME), b"original").unwrap();

        match plan(&dir) {
            Plan::Apply { target, backup } => {
                assert_eq!(target, dir.join(DLL_NAME));
                assert_eq!(backup, dir.join(BACKUP_NAME));
            }
            other => panic!("expected Apply, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_moves_the_original_aside_and_writes_the_proxy() {
        let dir = scratch("install");
        let target = dir.join(DLL_NAME);
        let backup = dir.join(BACKUP_NAME);
        std::fs::write(&target, b"original").unwrap();

        let app = App::default();
        assert!(install(&app, &target, &backup, b"proxy"));

        assert_eq!(std::fs::read(&target).unwrap(), b"proxy");
        assert_eq!(std::fs::read(&backup).unwrap(), b"original");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_restores_the_original_when_the_write_fails() {
        let dir = scratch("restore");
        let target = dir.join(DLL_NAME);
        let backup = dir.join("missing").join(BACKUP_NAME);
        std::fs::write(&target, b"original").unwrap();

        let app = App::default();
        // The backup directory does not exist, so the rename fails before
        // anything is touched.
        assert!(!install(&app, &target, &backup, b"proxy"));
        assert_eq!(std::fs::read(&target).unwrap(), b"original");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
