//! Environment setup and command wrapping (gamemode, mangohud, speedhack,
//! protonhax, gamescope, wezterm) plus Wayland/GPU detection.

use std::process::Command;

use crate::config::{App, EMPTY_MARKER};

/// Return the `lspci -vnn` lines describing display adapters, or an empty
/// string if `lspci` is unavailable.
fn gpu_info() -> String {
    let output = Command::new("lspci").arg("-vnn").output();
    let Ok(output) = output else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .filter(|line| {
            let l = line.to_lowercase();
            l.contains("vga") || l.contains("3d") || l.contains("display")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Decide whether to enable Wayland, honoring `-W`/`-X` overrides and otherwise
/// enabling it for NVIDIA GPUs.
pub fn determine_wayland_by_gpu(app: &mut App) {
    let info = gpu_info();
    app.log("Determining wayland");

    if app.wayland_force_enable {
        app.log("Wayland was force enabled");
        app.wayland_enabled = true;
    } else if app.wayland_force_disable {
        app.log("Wayland was force disabled");
        app.wayland_enabled = false;
    } else if contains_ci(&info, "NVIDIA") {
        app.log("Detected NVIDIA: wayland enabled");
        app.wayland_enabled = true;
    } else {
        app.log("Detected NON-NVIDIA: wayland disabled");
        app.wayland_enabled = false;
    }
}

/// Prepend `prefix` tokens to `cmd`, returning a new vector.
fn prepend(prefix: &[&str], cmd: &[String]) -> Vec<String> {
    let mut out: Vec<String> = prefix.iter().map(|s| s.to_string()).collect();
    out.extend_from_slice(cmd);
    out
}

/// Export the global environment and wrap the command with the enabled tools.
pub fn apply_wrappers(app: &mut App) {
    if app.disable_sdl3 {
        std::env::set_var("STEAM_COMPAT_RUNTIME_SDL3", "0");
    }

    std::env::set_var("ENABLE_LSFG", "0");

    if !app.winedlloverrides_list.is_empty() {
        std::env::set_var("WINEDLLOVERRIDES", app.winedlloverrides_list.join(";"));
    }

    if app.lsfg {
        std::env::set_var("ENABLE_LSFG", "1");
        std::env::set_var("LSFG_PROCESS", "mangohud");
    }

    if app.wayland_enabled {
        std::env::set_var("PROTON_ENABLE_WAYLAND", "1");
        std::env::set_var("PROTON_USE_WAYLAND", "1");
        std::env::set_var("QT_QPA_PLATFORM", "wayland");
        std::env::set_var("SDL_VIDEODRIVER", "wayland");
    }

    let info = gpu_info();

    std::env::set_var("PROTON_ENABLE_HDR", "1");
    std::env::set_var("ENABLE_HDR_WSI", "1");
    std::env::set_var("PROTON_USE_EAC_LINUX", "1");
    std::env::set_var("PROTON_USE_NTSYNC", "1");
    std::env::set_var("DXVK_CONFIG", "dxgi.syncInterval=0");
    std::env::set_var(
        "VKD3D_CONFIG",
        "dxr12,dxr,descriptor_heap,enable_experimental_features",
    );
    std::env::set_var("PROTON_VKD3D_HEAP", "1");
    std::env::set_var("PROTON_DXVK_LOWLATENCY", "1");
    std::env::set_var("PROTON_FSR4_UPGRADE", "1");

    if contains_ci(&info, "NVIDIA") {
        std::env::set_var("PROTON_ENABLE_NVAPI", "1");
        std::env::set_var("DXVK_ENABLE_NVAPI", "1");
        std::env::set_var("PROTON_DLSS_UPGRADE", "1");
        std::env::set_var("__GL_THREADED_OPTIMIZATIONS", "1");
        std::env::set_var("PROTON_NVIDIA_LIBS", "1");
        std::env::set_var("PROTON_NVIDIA_LIBS_NO_32BIT", "1");
        std::env::set_var("PROTON_NVIDIA_NVOPTIX", "1");
        std::env::set_var("PROTON_ENABLE_NGX_UPDATER", "1");
    } else if contains_ci(&info, "AMD")
        || contains_ci(&info, "Advanced Micro Devices")
        || contains_ci(&info, "Radeon")
    {
        std::env::set_var("ENABLE_LAYER_MESA_ANTI_LAG", "1");
    }

    // Proton games skip the speedhack layer; native games skip protonhax.
    if app.isproton {
        app.speedhack = false;
    } else {
        app.protonhax = false;
    }

    let mut cmd = std::mem::take(&mut app.cmd);

    if app.speedhack {
        cmd = prepend(&["speedhack"], &cmd);
    }
    if app.protonhax {
        cmd = prepend(&["protonhax", "init"], &cmd);
    }
    if app.gamescope_wayland {
        std::env::set_var("PROTON_ENABLE_WAYLAND", "1");
        std::env::set_var("PROTON_USE_WAYLAND", "1");
        cmd = prepend(
            &[
                "gamescope",
                "-r",
                "165",
                "--force-grab-cursor",
                "-w",
                "1920",
                "-h",
                "1080",
                "-f",
                "--rt",
                "--hdr-enabled",
                "--hdr-itm-enabled",
                "-S",
                "stretch",
                "--backend",
                "wayland",
                "--expose-wayland",
                "--",
            ],
            &cmd,
        );
    } else if app.gamescope {
        cmd = prepend(
            &[
                "gamescope",
                "-r",
                "165",
                "--force-grab-cursor",
                "-w",
                "1920",
                "-h",
                "1080",
                "-f",
                "--rt",
                "--hdr-enabled",
                "--hdr-itm-enabled",
                "-S",
                "stretch",
                "--",
            ],
            &cmd,
        );
    }
    if app.mangohud {
        cmd = prepend(&["mangohud"], &cmd);
    }
    if app.wezterm {
        cmd = prepend(&["wezterm", "start", "--cwd", ".", "--"], &cmd);
    }
    if app.gamemode {
        cmd = prepend(&["gamemoderun"], &cmd);
    }

    app.cmd = cmd;
}

/// Merge `fix.so` into an existing `LD_AUDIT` value (colon-separated).
///
/// Returns the merged value. If `fix_path` is empty, returns `current` unchanged.
/// If `current` is empty, returns just `fix_path`.
fn merge_ld_audit(current: &str, fix_path: &str) -> String {
    if fix_path.is_empty() {
        current.to_string()
    } else if current.is_empty() {
        fix_path.to_string()
    } else {
        format!("{fix_path}:{current}")
    }
}

/// Apply the captured custom exports, unsetting variables marked `(empty)`.
///
/// After applying exports, if `-F` was set, merge `$HOME/scripts/fix.so` into
/// `LD_AUDIT` (colon-separated), preserving any value the user set via
/// `KEY=VALUE` or inherited from the environment.
pub fn apply_environment_modifications(app: &App) {
    for export in &app.custom_exports {
        if export.value == EMPTY_MARKER {
            std::env::remove_var(&export.name);
        } else {
            std::env::set_var(&export.name, &export.value);
        }
    }

    if app.fix_audit {
        let fix_path = std::env::var("HOME")
            .map(|h| format!("{h}/scripts/fix.so"))
            .unwrap_or_else(|_| String::new());

        if !fix_path.is_empty() {
            let current = std::env::var("LD_AUDIT").unwrap_or_default();
            let merged = merge_ld_audit(&current, &fix_path);
            std::env::set_var("LD_AUDIT", merged);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_ld_audit_prepends_fix_so() {
        // Empty current -> just fix.so
        let result = merge_ld_audit("", "/home/user/scripts/fix.so");
        assert_eq!(result, "/home/user/scripts/fix.so");

        // Existing value -> prepended with colon
        let result = merge_ld_audit("/other/lib.so", "/home/user/scripts/fix.so");
        assert_eq!(result, "/home/user/scripts/fix.so:/other/lib.so");

        // Empty fix path -> no change
        let result = merge_ld_audit("/existing.so", "");
        assert_eq!(result, "/existing.so");

        // Both empty -> empty
        let result = merge_ld_audit("", "");
        assert_eq!(result, "");
    }
}
