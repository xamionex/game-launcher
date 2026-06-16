//! Command construction: Proton detection, SteamLinuxRuntime stripping
//! (the spaces-in-path fix), and custom vkd3d-proton selection.

use std::path::Path;

use crate::config::App;

/// Scan the first 12 original-command tokens for Proton/`.exe` indicators and
/// capture the Proton path and version when present.
pub fn determine_proton(app: &mut App) {
    app.log("Grabbing proton version");

    for arg in app.original_cmd.iter().take(12) {
        let arg_lc = arg.to_lowercase();
        if arg_lc.contains("proton") || arg_lc.contains(".exe") {
            app.isproton = true;

            if arg_lc.contains("proton") {
                app.proton_path = arg.clone();
                let proton_dir = Path::new(arg).parent();
                if let Some(dir) = proton_dir {
                    if app.proton_ver.is_empty() {
                        if let Some(name) = dir.file_name() {
                            app.proton_ver = name.to_string_lossy().into_owned();
                        }
                    }
                    let version_file = dir.join("version");
                    if version_file.is_file() {
                        if let Ok(contents) = std::fs::read_to_string(&version_file) {
                            app.proton_ver = contents.trim_end().to_string();
                        }
                    }
                }
            }
            break;
        }
    }
}

/// True when `token` is a single SteamLinuxRuntime entry-point path, matching
/// the original sed pattern `/[^ ]*SteamLinuxRuntime[^ ]*`.
fn is_runtime_path(token: &str) -> bool {
    token.starts_with('/') && !token.contains(' ') && token.contains("SteamLinuxRuntime")
}

/// Remove SteamLinuxRuntime segments from a command vector by filtering
/// elements directly.
///
/// This replaces the original string round-trip
/// (`cmd_str="${CMD[*]}"; eval "CMD=($cmd_str)"`) that re-split on whitespace
/// and destroyed any argument containing spaces. Operating on the vector keeps
/// every other argument byte-for-byte intact.
///
/// For each ` -- <runtime-path>` pair, the `--` and the runtime path are
/// dropped; a directly following `--verb=waitforexitandrun` token is dropped
/// too (matching the two sed substitutions).
fn strip_steam_linux_runtime(cmd: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(cmd.len());
    let mut i = 0;
    while i < cmd.len() {
        if cmd[i] == "--" && i + 1 < cmd.len() && is_runtime_path(&cmd[i + 1]) {
            // Skip "--" and the runtime path token.
            i += 2;
            // Drop a trailing verb token if present.
            if i < cmd.len() && cmd[i] == "--verb=waitforexitandrun" {
                i += 1;
            }
            continue;
        }
        out.push(cmd[i].clone());
        i += 1;
    }
    out
}

/// Build the final command vector from the original command, applying the
/// optional executable replacement and pressure-vessel stripping.
pub fn build_command(app: &mut App) -> Result<(), String> {
    let mut cmd = app.original_cmd.clone();

    if !app.replacement_exe.is_empty() {
        // Find the last "waitforexitandrun" marker.
        let last_wait = cmd.iter().rposition(|a| a == "waitforexitandrun");
        if let Some(idx) = last_wait {
            cmd.truncate(idx + 1);
            // Split the replacement on whitespace to allow trailing arguments
            // (parity with the original `IFS=' ' read -ra`).
            cmd.extend(app.replacement_exe.split_whitespace().map(|s| s.to_string()));
        } else if let Some(last) = cmd.last_mut() {
            *last = app.replacement_exe.clone();
        }
    }

    if app.pressure_vessel && !app.isproton {
        cmd = strip_steam_linux_runtime(&cmd);
    }

    if cmd.is_empty() {
        return Err("No command specified.".to_string());
    }

    app.cmd = cmd;
    Ok(())
}

/// Detect a binary's architecture from ELF or PE headers.
///
/// Returns `Some(32)` or `Some(64)`, or `None` if undetermined.
pub fn get_binary_arch(path: &Path) -> Option<u32> {
    let data = std::fs::read(path).ok()?;

    // ELF: 0x7F 'E' 'L' 'F', EI_CLASS at offset 4 (1 = 32-bit, 2 = 64-bit).
    if data.len() >= 5 && &data[0..4] == b"\x7fELF" {
        return match data[4] {
            1 => Some(32),
            2 => Some(64),
            _ => None,
        };
    }

    // PE: 'M' 'Z' at start; e_lfanew (u32 LE) at 0x3C points to the PE header,
    // whose Machine field (u16 LE) sits 4 bytes in.
    if data.len() >= 0x40 && &data[0..2] == b"MZ" {
        let pe_offset = u32::from_le_bytes([data[0x3C], data[0x3D], data[0x3E], data[0x3F]]) as usize;
        if pe_offset > 0 && pe_offset < 1_048_576 && pe_offset + 6 <= data.len() {
            let machine = u16::from_le_bytes([data[pe_offset + 4], data[pe_offset + 5]]);
            return match machine {
                0x014c => Some(32),
                0x8664 => Some(64),
                _ => None,
            };
        }
    }

    None
}

/// Load a custom vkd3d-proton build when `-V` is set and the build exists,
/// selecting x64/x86 DLLs based on the detected game architecture.
pub fn setup_custom_vkd3d(app: &App) {
    if !app.enable_custom_vkd3d {
        return;
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let vkd3d_base = Path::new(&home)
        .join("Projects")
        .join("vkd3d-proton")
        .join("build")
        .join("vkd3d-proton-master");
    if !vkd3d_base.is_dir() {
        return;
    }

    // First argument that looks like a Windows executable.
    let game_exe = app.original_cmd.iter().find(|a| a.ends_with(".exe"));

    let arch = match game_exe {
        Some(exe) if Path::new(exe).is_file() => {
            let a = get_binary_arch(Path::new(exe));
            if a.is_none() {
                app.log(&format!(
                    "Warning: Could not detect architecture for {exe}, falling back to x64"
                ));
            }
            a
        }
        _ => {
            app.log("No game EXE found, defaulting to x64");
            None
        }
    };

    let dll_dir = match arch {
        Some(64) => vkd3d_base.join("x64"),
        Some(32) => vkd3d_base.join("x86"),
        _ => vkd3d_base.join("x64"),
    };

    if dll_dir.is_dir() {
        let prev = std::env::var("PROTON_DLL_PATH").unwrap_or_default();
        let new_path = if prev.is_empty() {
            dll_dir.to_string_lossy().into_owned()
        } else {
            format!("{}:{prev}", dll_dir.to_string_lossy())
        };
        std::env::set_var("PROTON_DLL_PATH", new_path);

        let prev_overrides = std::env::var("WINEDLLOVERRIDES").unwrap_or_default();
        let overrides = if prev_overrides.is_empty() {
            "d3d12=n,b;d3d12core=n,b".to_string()
        } else {
            format!("d3d12=n,b;d3d12core=n,b;{prev_overrides}")
        };
        std::env::set_var("WINEDLLOVERRIDES", overrides);

        app.log(&format!("Set VKD3D path to: {}", dll_dir.to_string_lossy()));
    }
}

#[cfg(test)]
mod tests {
    // Tests build an App by mutating fields after Default for readability.
    #![allow(clippy::field_reassign_with_default)]
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn runtime_path_detection() {
        assert!(is_runtime_path(
            "/home/u/.steam/SteamLinuxRuntime_sniper/_v2-entry-point"
        ));
        // A path containing a space is not a single runtime token.
        assert!(!is_runtime_path("/home/u/SteamLinuxRuntime dir/run"));
        assert!(!is_runtime_path("relative/SteamLinuxRuntime/run"));
        assert!(!is_runtime_path("/home/u/Orebits Demo/game"));
    }

    #[test]
    fn strip_keeps_space_paths_intact() {
        // A full native invocation that uses the Steam runtime, with a final
        // executable path containing spaces.
        let cmd = v(&[
            "/s/steam-launch-wrapper",
            "--",
            "/s/reaper",
            "SteamLaunch",
            "AppId=42",
            "--",
            "/s/SteamLinuxRuntime_sniper/_v2-entry-point",
            "--verb=waitforexitandrun",
            "--",
            "/games/Orebits Demo/Orebits DemoV1.0",
        ]);
        let out = strip_steam_linux_runtime(&cmd);
        // The runtime token, its `--`, and the verb are removed.
        assert!(!out.iter().any(|a| a.contains("SteamLinuxRuntime")));
        assert!(!out.iter().any(|a| a == "--verb=waitforexitandrun"));
        // The space path survives as a single element.
        assert_eq!(out.last().unwrap(), "/games/Orebits Demo/Orebits DemoV1.0");
    }

    #[test]
    fn build_command_preserves_space_path_without_runtime() {
        // Reproduces the reported crash input: native game, no runtime token,
        // path with spaces. The original eval round-trip split this apart.
        let mut app = App::default();
        app.original_cmd = v(&[
            "/s/steam-launch-wrapper",
            "--",
            "/s/reaper",
            "SteamLaunch",
            "AppId=4521640",
            "--",
            "/home/p/.local/share/Steam/steamapps/common/Orebits Demo/Orebits DemoV1.0",
        ]);
        app.pressure_vessel = true;
        app.isproton = false;

        build_command(&mut app).unwrap();

        assert_eq!(app.cmd.len(), app.original_cmd.len());
        assert_eq!(
            app.cmd.last().unwrap(),
            "/home/p/.local/share/Steam/steamapps/common/Orebits Demo/Orebits DemoV1.0"
        );
    }

    #[test]
    fn replacement_exe_after_last_waitforexitandrun() {
        let mut app = App::default();
        app.original_cmd = v(&[
            "proton",
            "waitforexitandrun",
            "/games/old.exe",
            "--old-arg",
        ]);
        app.isproton = true; // skip runtime stripping
        app.replacement_exe = "/games/new.exe --new".to_string();

        build_command(&mut app).unwrap();

        assert_eq!(
            app.cmd,
            v(&["proton", "waitforexitandrun", "/games/new.exe", "--new"])
        );
    }

    #[test]
    fn empty_command_is_an_error() {
        let mut app = App::default();
        assert!(build_command(&mut app).is_err());
    }

    #[test]
    fn proton_detection_from_path() {
        let mut app = App::default();
        app.original_cmd = v(&[
            "/home/u/.steam/compatibilitytools.d/GE-Proton9-20/proton",
            "waitforexitandrun",
            "/games/game.exe",
        ]);
        determine_proton(&mut app);
        assert!(app.isproton);
        assert_eq!(app.proton_ver, "GE-Proton9-20");
    }

    #[test]
    fn arch_detection_elf_and_pe() {
        let dir = std::env::temp_dir();

        let elf = dir.join(format!("game_test_elf_{}", std::process::id()));
        std::fs::write(&elf, [0x7f, b'E', b'L', b'F', 2, 0, 0, 0]).unwrap();
        assert_eq!(get_binary_arch(&elf), Some(64));
        std::fs::remove_file(&elf).ok();

        let mut pe = vec![0u8; 0x50];
        pe[0] = b'M';
        pe[1] = b'Z';
        pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        pe[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
        let pe_path = dir.join(format!("game_test_pe_{}", std::process::id()));
        std::fs::write(&pe_path, &pe).unwrap();
        assert_eq!(get_binary_arch(&pe_path), Some(64));
        std::fs::remove_file(&pe_path).ok();
    }
}
