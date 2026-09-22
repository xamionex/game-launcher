//! Environment setup and command wrapping (gamemode, mangohud, protonhax, gamescope, wezterm, linuwux) plus Wayland/GPU detection.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;

use crate::config::{App, EMPTY_MARKER};
use crate::payload;

/// Upstream release asset for the netsock patch (`fix.so`); fetched into the payload cache on first use and falling back to the embedded copy.
const NETSNOCK_URL: &str =
    "https://github.com/yesyes0649/steamnetsock-patch/releases/latest/download/fix.so";

/// Return the `lspci -vnn` lines describing display adapters, or an empty string if `lspci` is unavailable.
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

/// Decide whether to enable Wayland, honoring `-W`/`-X` overrides and otherwise enabling it for NVIDIA GPUs.
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

/// True when running inside a Steam Deck style gaming-mode session (gamescope with the Steam session type).
fn is_gaming_mode() -> bool {
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let steam_game = std::env::var("SteamGameId").is_ok();
    session.eq_ignore_ascii_case("gamescope")
        || desktop.eq_ignore_ascii_case("gamescope")
        || (steam_game && desktop.eq_ignore_ascii_case("steam"))
}

/// The running kernel's release string, e.g. `6.12.7-bore`.
fn kernel_release() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// True when the running kernel has the BORE scheduler active.
///
/// BORE kernels expose `kernel.sched_bore` (1 = enabled, the default); older patch sets are recognised by the release string instead.
fn bore_scheduler_active() -> bool {
    if let Ok(value) = std::fs::read_to_string("/proc/sys/kernel/sched_bore") {
        return value.trim() == "1";
    }
    kernel_release().to_lowercase().contains("bore")
}

/// True when any running process has one of these `comm` names.
fn process_running(names: &[&str]) -> bool {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .filter(|n| n.chars().all(|c| c.is_ascii_digit()))
        else {
            continue;
        };
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        if names.contains(&comm.trim()) {
            return true;
        }
    }
    false
}

/// Why `gamemoderun` should be skipped on this system, if anything.
///
/// GameMode and ananicy-cpp both renice processes, which fights and produces the stutter they are both meant to remove; BORE kernels already provide the responsiveness GameMode aims for.
fn gamemode_conflict() -> Option<&'static str> {
    if bore_scheduler_active() {
        return Some("BORE scheduler active");
    }
    if process_running(&["ananicy-cpp", "ananicy"]) {
        return Some("ananicy-cpp running");
    }
    None
}

/// Run `program` with `args`, returning its stdout when it succeeds.
fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A Wayland output, with a short description when the source provides one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    /// Resolution and position, e.g. `1920x1080 at 1920,0`; empty when unknown.
    pub detail: String,
}

impl Monitor {
    fn new(name: &str) -> Monitor {
        Monitor {
            name: name.to_string(),
            detail: String::new(),
        }
    }
}

/// Compose a detail string from optional mode and position parts.
fn monitor_detail(mode: &str, position: &str) -> String {
    match (mode.is_empty(), position.is_empty()) {
        (false, false) => format!("{mode} at {position}"),
        (false, true) => mode.to_string(),
        (true, false) => format!("at {position}"),
        (true, true) => String::new(),
    }
}

/// Parse one `wl_output` block of `wayland-info` output.
fn monitor_from_wayland_info_block(lines: &[&str]) -> Monitor {
    let mut monitor = Monitor::new("");
    let mut position = String::new();

    for (index, line) in lines.iter().enumerate() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("name:") {
            monitor.name = value.trim().trim_matches('\'').trim().to_string();
        } else if let Some(rest) = line.strip_prefix("x:") {
            // "1920, y: 0, scale: 1,"
            let parts: Vec<&str> = rest.split(',').collect();
            let x = parts.first().map(|p| p.trim()).unwrap_or("");
            let y = parts
                .get(1)
                .and_then(|p| p.trim().strip_prefix("y:"))
                .map(str::trim)
                .unwrap_or("");
            if !x.is_empty() && !y.is_empty() {
                position = format!("{x},{y}");
            }
        } else if let Some(rest) = line.strip_prefix("width:") {
            // The current mode is the one whose flags line says so.
            let current = lines
                .get(index + 1)
                .map(|flags| flags.contains("flags:") && flags.contains("current"))
                .unwrap_or(false);
            if current {
                let width = rest.split_whitespace().next().unwrap_or("");
                let height = line
                    .split("height:")
                    .nth(1)
                    .and_then(|h| h.split_whitespace().next())
                    .unwrap_or("");
                if !width.is_empty() && !height.is_empty() {
                    monitor.detail = monitor_detail(&format!("{width}x{height}"), &position);
                }
            }
        }
    }

    if !position.is_empty() && monitor.detail.is_empty() {
        monitor.detail = monitor_detail("", &position);
    }
    monitor
}

/// wl_output entries from `wayland-info`, one block per monitor:
///
/// ```text
/// interface: 'wl_output', version: 4, name: 65
///     name: DP-1
///     x: 1920, y: 0, scale: 1,
///     mode:
///         width: 1920 px, height: 1080 px, refresh: 165.001 Hz,
///         flags: current
/// ```
fn monitors_from_wayland_info(text: &str) -> Vec<Monitor> {
    let mut out: Vec<Monitor> = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    let mut in_output = false;

    for line in text.lines() {
        if line.starts_with("interface:") {
            if in_output {
                push_wayland_info_block(&block, &mut out);
            }
            block.clear();
            in_output = line.contains("'wl_output'");
            continue;
        }
        if in_output {
            block.push(line);
        }
    }
    if in_output {
        push_wayland_info_block(&block, &mut out);
    }
    out
}

/// Parse a block and keep it when it names an output that is not already listed.
fn push_wayland_info_block(block: &[&str], out: &mut Vec<Monitor>) {
    if block.is_empty() {
        return;
    }
    let monitor = monitor_from_wayland_info_block(block);
    if !monitor.name.is_empty() && !out.contains(&monitor) {
        out.push(monitor);
    }
}

/// Monitor names from the JSON listings of `wlr-randr --json`, `hyprctl monitors -j` and `swaymsg -t get_outputs`, which are all arrays of objects carrying a `name`.
fn monitors_from_json(text: &str) -> Vec<Monitor> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name").and_then(|n| n.as_str()))
                .map(Monitor::new)
                .collect()
        })
        .unwrap_or_default()
}

/// Connected outputs from `kscreen-doctor -o`.
///
/// Each block starts with `Output: <id> <name> <uuid>`, reports `connected` or `disconnected` on a following line and carries `Geometry: x,y WxH`. The output is colored with ANSI escapes, which are stripped before parsing.
fn monitors_from_kscreen(text: &str) -> Vec<Monitor> {
    let ansi = Regex::new("\u{1b}\\[[0-9;]*m").unwrap();
    let clean = ansi.replace_all(text, "");
    let lines: Vec<&str> = clean.lines().collect();

    let mut out: Vec<Monitor> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(rest) = line.trim().strip_prefix("Output:") else {
            continue;
        };
        let Some(name) = rest.split_whitespace().nth(1) else {
            continue;
        };
        let connected = lines[index + 1..]
            .iter()
            .take(3)
            .any(|l| l.trim() == "connected");
        if !connected || out.iter().any(|m| m.name == name) {
            continue;
        }

        let mut monitor = Monitor::new(name);
        if let Some(geometry) = lines[index + 1..]
            .iter()
            .take(12)
            .find(|l| l.trim_start().starts_with("Geometry:"))
        {
            // "Geometry: 1920,0 1920x1080"
            let parts: Vec<&str> = geometry
                .trim_start()
                .trim_start_matches("Geometry:")
                .split_whitespace()
                .collect();
            if parts.len() >= 2 {
                monitor.detail = monitor_detail(parts[1], parts[0]);
            }
        }
        out.push(monitor);
    }
    out
}

/// Connected connector names from `/sys/class/drm`, e.g. `card1-DP-1` -> `DP-1`.
fn monitors_from_drm() -> Vec<Monitor> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut out: Vec<Monitor> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let status = std::fs::read_to_string(entry.path().join("status")).unwrap_or_default();
        if status.trim() != "connected" {
            continue;
        }
        let Some((_, connector)) = name.split_once('-') else {
            continue;
        };
        if connector.is_empty() || out.iter().any(|m| m.name == connector) {
            continue;
        }
        out.push(Monitor::new(connector));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Wayland outputs known to the session.
///
/// Compositor tools are tried in order and the DRM connectors are used as a last resort, so there is normally something to pick from.
pub fn detect_monitors() -> Vec<Monitor> {
    /// A detection source: program, arguments, and stdout parser.
    type Source = (
        &'static str,
        &'static [&'static str],
        fn(&str) -> Vec<Monitor>,
    );

    let attempts: &[Source] = &[
        ("wayland-info", &[], monitors_from_wayland_info),
        ("wlr-randr", &["--json"], monitors_from_json),
        ("hyprctl", &["monitors", "-j"], monitors_from_json),
        ("swaymsg", &["-t", "get_outputs"], monitors_from_json),
        ("kscreen-doctor", &["-o"], monitors_from_kscreen),
    ];
    for (program, args, parse) in attempts {
        if let Some(stdout) = command_stdout(program, args) {
            let monitors = parse(&stdout);
            if !monitors.is_empty() {
                return monitors;
            }
        }
    }
    monitors_from_drm()
}

/// The output the focused window is on, when the compositor can tell us.
///
/// This is how the config editor learns which monitor its own terminal is on, so the names in the picker can be told apart.
pub fn active_monitor() -> Option<String> {
    // KWin (Plasma): the output of the active window.
    for program in ["qdbus6", "qdbus"] {
        if let Some(out) = command_stdout(
            program,
            &["org.kde.KWin", "/KWin", "org.kde.KWin.activeOutputName"],
        ) {
            let name = out.trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }

    // Hyprland: the monitor of the active workspace.
    if let Some(out) = command_stdout("hyprctl", &["activeworkspace", "-j"]) {
        if let Some(name) = json_string_field(&out, "monitor") {
            return Some(name);
        }
    }

    // Sway: the output of the focused workspace.
    if let Some(out) = command_stdout("swaymsg", &["-t", "get_workspaces"]) {
        if let Some(name) = focused_workspace_output(&out) {
            return Some(name);
        }
    }

    None
}

/// Read a top-level string field from a JSON object.
fn json_string_field(text: &str, field: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let found = value.get(field)?.as_str()?.trim();
    if found.is_empty() {
        None
    } else {
        Some(found.to_string())
    }
}

/// Output of the focused workspace in `swaymsg -t get_workspaces` output.
fn focused_workspace_output(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let workspaces = value.as_array()?;
    workspaces
        .iter()
        .find(|workspace| workspace.get("focused").and_then(|f| f.as_bool()) == Some(true))
        .and_then(|workspace| workspace.get("output"))
        .and_then(|output| output.as_str())
        .map(str::trim)
        .filter(|output| !output.is_empty())
        .map(str::to_string)
}

/// The Wayland monitor to export, if Wayland is on and one is configured.
fn wayland_monitor_env(app: &App) -> Option<&str> {
    if app.wayland_enabled && !app.wayland_monitor.is_empty() {
        Some(&app.wayland_monitor)
    } else {
        None
    }
}

/// Prepend `prefix` tokens to `cmd`, returning a new vector.
fn prepend(prefix: &[&str], cmd: &[String]) -> Vec<String> {
    let mut out: Vec<String> = prefix.iter().map(|s| s.to_string()).collect();
    out.extend_from_slice(cmd);
    out
}

/// True if `path` is a regular file with an executable bit set.
fn is_executable(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

/// True if `program` resolves to an executable, either directly (when it contains a `/`) or via a `PATH` lookup, like `command -v`.
fn command_exists(program: &str) -> bool {
    if program.contains('/') {
        return is_executable(Path::new(program));
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| is_executable(&dir.join(program)))
}

/// Prepend the LinuwUx hypervisor loader as the outermost wrapper.
///
/// LinuwUx must run before MangoHud/Gamescope/etc. — when it ends up inside another wrapper (e.g. `mangohud linuwux ...`) the game frequently fails to start.
/// Wrapping the whole chain in `env LD_PRELOAD=...` keeps it at the front while still handing the loader to the game and its helper processes.
fn wrap_linuwux(app: &App, cmd: Vec<String>) -> Vec<String> {
    if !app.hypervisor {
        return cmd;
    }
    let Some(path) = hypervisor_path() else {
        app.log("LinuwUx: HOME unset, skipping loader");
        return cmd;
    };
    if !path.is_file() {
        app.log(&format!(
            "Extracting hypervisor loader to {}",
            path.display()
        ));
        extract_so(app, include_bytes!("../liblinuwux.so"), &path);
    }
    if !path.is_file() {
        return cmd;
    }
    let mut out: Vec<String> = vec![
        "env".to_string(),
        format!("LD_PRELOAD={}", path.to_string_lossy()),
        "PROTON_DISABLE_LSTEAMCLIENT=0".to_string(),
    ];
    out.extend(cmd);
    out
}

/// Prepend a wrapper only if it is enabled and its binary exists.
///
/// When enabled but missing, the wrapper is skipped and the omission is logged.
fn maybe_wrap(
    app: &App,
    cmd: Vec<String>,
    enabled: bool,
    program: &str,
    prefix: &[&str],
) -> Vec<String> {
    if !enabled {
        return cmd;
    }
    if command_exists(program) {
        prepend(prefix, &cmd)
    } else {
        app.log(&format!("Wrapper not found, skipping: {program}"));
        cmd
    }
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

        if let Some(monitor) = wayland_monitor_env(app) {
            std::env::set_var("WAYLANDDRV_PRIMARY_MONITOR", monitor);
            app.log(&format!("Wayland primary monitor: {monitor}"));
        }
    } else if !app.wayland_monitor.is_empty() {
        app.log(&format!(
            "Wayland primary monitor {} ignored: Wayland is disabled, force it with -W",
            app.wayland_monitor
        ));
    }

    let info = gpu_info();

    std::env::set_var("PROTON_ENABLE_HDR", "1");
    std::env::set_var("ENABLE_HDR_WSI", "1");
    std::env::set_var("PROTON_USE_EAC_LINUX", "1");
    std::env::set_var("PROTON_USE_NTSYNC", "1");

    // DXVK
    use std::fs;

    // Prefer a per-game config (<exe>.conf) if one exists, otherwise fall back
    // to the global dxvk.conf.
    //
    // The game's own executable is the right key, not `app.cmd.first()`: by the time this runs, `build_command` has already produced a cmd whose first token is the Proton launcher (e.g. `.../GE-Proton9-20/proton`) for Wine games, or the native binary otherwise.
    // Using that would look up `proton.conf` instead of `<gamename>.conf`.
    // The original bash script used `$EXE_NAME`, derived from the game executable, so we mirror that by scanning `original_cmd` for a `.exe` and falling back to the last token.
    let exe_name = app
        .original_cmd
        .iter()
        .find(|a| a.to_lowercase().ends_with(".exe"))
        .or_else(|| app.original_cmd.last())
        .and_then(|p| std::path::Path::new(p).file_stem())
        .and_then(|s| s.to_str());

    let dxvk_dir = dirs::home_dir().unwrap().join(".config/dxvk");

    let dxvk_config_file = exe_name
        .map(|name| dxvk_dir.join(format!("{name}.conf")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| dxvk_dir.join("dxvk.conf"));

    app.log(&format!(
        "Using DXVK_CONFIG generated from {}",
        dxvk_config_file.display()
    ));

    match fs::read_to_string(&dxvk_config_file) {
        Ok(contents) => {
            // Match the original bash pipeline:
            //   rg --pcre2 "^[^#].+(?=$)" | sed "s/$/;/g" | xargs echo
            // i.e. keep non-empty, non-comment lines, append `;`, and join with spaces into a single string.
            let dxvk_config = contents
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|line| format!("{line};"))
                .collect::<Vec<_>>()
                .join(" ");

            app.log(&format!("DXVK_CONFIG={dxvk_config}"));
            std::env::set_var("DXVK_CONFIG", &dxvk_config);
        }
        Err(e) => {
            app.log(&format!(
                "Failed to read DXVK config {}: {e}",
                dxvk_config_file.display()
            ));
        }
    }

    std::env::set_var(
        "VKD3D_CONFIG",
        "dxr12,dxr,descriptor_heap,enable_experimental_features",
    );
    std::env::set_var("PROTON_VKD3D_HEAP", "1");
    std::env::set_var("PROTON_DXVK_LOWLATENCY", "1");
    std::env::set_var("LOW_LATENCY_LAYER", "1");
    std::env::set_var("PROTON_FSR4_UPGRADE", "1");
    std::env::set_var("FSR4_UPGRADE", "1");

    if contains_ci(&info, "NVIDIA") {
        //std::env::set_var("PROTON_ENABLE_NVAPI", "1");
        //std::env::set_var("DXVK_ENABLE_NVAPI", "1");
        std::env::set_var("PROTON_DLSS_UPGRADE", "1");
        std::env::set_var("__GL_THREADED_OPTIMIZATIONS", "1");
        std::env::set_var("PROTON_NVIDIA_LIBS", "1");
        std::env::set_var("PROTON_NVIDIA_LIBS_NO_32BIT", "1");
        std::env::set_var("PROTON_NVIDIA_NVOPTIX", "1");
        std::env::set_var("PROTON_ENABLE_NGX_UPDATER", "1");
        std::env::set_var("LOW_LATENCY_LAYER_REFLEX", "1");
    } else if contains_ci(&info, "AMD")
        || contains_ci(&info, "Advanced Micro Devices")
        || contains_ci(&info, "Radeon")
    {
        //std::env::set_var("ENABLE_LAYER_MESA_ANTI_LAG", "1");
    }

    // Native games skip protonhax; Proton games never had it on anyway.
    if !app.isproton {
        app.protonhax = false;
    }

    // Gaming mode (Steam Deck / handheld) already shows a HUD via the Steam overlay, so MangoHud is redundant there. `-H` forces it back on.
    if is_gaming_mode() && !app.mangohud_force {
        app.log("Gaming mode detected, disabling MangoHud");
        app.mangohud = false;
    }

    let mut cmd = std::mem::take(&mut app.cmd);

    cmd = maybe_wrap(app, cmd, app.protonhax, "protonhax", &["protonhax", "init"]);

    // Gamescope has two variants and sets Wayland env vars, so it is handled outside maybe_wrap but still checks for the binary.
    if app.gamescope_wayland {
        if command_exists("gamescope") {
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
        } else {
            app.log("Wrapper not found, skipping: gamescope");
        }
    } else if app.gamescope {
        if command_exists("gamescope") {
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
        } else {
            app.log("Wrapper not found, skipping: gamescope");
        }
    }

    cmd = maybe_wrap(app, cmd, app.mangohud, "mangohud", &["mangohud"]);
    cmd = maybe_wrap(
        app,
        cmd,
        app.wezterm,
        "wezterm",
        &["wezterm", "start", "--cwd", ".", "--"],
    );

    // GameMode is skipped when it would fight the system's own scheduler setup.
    if app.gamemode {
        if let Some(reason) = gamemode_conflict() {
            app.log(&format!("GameMode: {reason}, skipping gamemoderun"));
            app.gamemode = false;
        }
    }
    cmd = maybe_wrap(app, cmd, app.gamemode, "gamemoderun", &["gamemoderun"]);

    // LinuwUx must sit at the very front of the chain: `linuwux mangohud ...`
    // works, but `mangohud linuwux ...` often fails.
    cmd = wrap_linuwux(app, cmd);

    app.cmd = cmd;
}

/// Merge `netsock.so` into an existing `LD_AUDIT` value (colon-separated).
///
/// Returns the merged value. If `netsock_path` is empty, returns `current` unchanged.
/// If `current` is empty, returns just `netsock_path`.
fn merge_ld_audit(current: &str, netsock_path: &str) -> String {
    if netsock_path.is_empty() {
        current.to_string()
    } else if current.is_empty() {
        netsock_path.to_string()
    } else {
        format!("{netsock_path}:{current}")
    }
}

/// Write an embedded `.so` payload to `path`, creating parent directories.
///
/// Logs a warning instead of failing when the write does not succeed; the launch is not blocked for this.
fn extract_so(app: &App, bytes: &[u8], path: &Path) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            app.log(&format!("Failed to create {}: {e}", parent.display()));
            return;
        }
    }
    if let Err(e) = std::fs::write(path, bytes) {
        app.log(&format!("Failed to extract {}: {e}", path.display()));
    }
}

/// Target path for the netsock loader: `$HOME/.config/SLSsteam/tools/netsock/netsock.so`.
fn netsock_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .map(|h| Path::new(&h).join(".config/SLSsteam/tools/netsock/netsock.so"))
        .ok()
}

/// Target path for the hypervisor loader: `$HOME/.local/lib/liblinuwux.so`.
fn hypervisor_path() -> Option<PathBuf> {
    std::env::var("HOME")
        .map(|h| Path::new(&h).join(".local/lib/liblinuwux.so"))
        .ok()
}

/// Apply the captured custom exports, unsetting variables marked `(empty)`.
///
/// After applying exports, `-F` installs the netsock loader to `$HOME/.config/SLSsteam/tools/netsock/netsock.so` when it is missing and merges it into `LD_AUDIT` (colon-separated), preserving any value the user set via `KEY=VALUE` or inherited from the environment.
/// The loader is taken from the payload cache when available (downloaded once, see [`crate::payload`]) and from the copy embedded in the binary otherwise.
///
/// The `-v` hypervisor loader is applied earlier, in [`apply_wrappers`], so that it lands at the front of the command chain instead of leaking into every wrapper process via a global `LD_PRELOAD`.
pub fn apply_environment_modifications(app: &App) {
    for export in &app.custom_exports {
        if export.value == EMPTY_MARKER {
            std::env::remove_var(&export.name);
        } else {
            std::env::set_var(&export.name, &export.value);
        }
    }

    if app.fix_audit {
        if let Some(path) = netsock_path() {
            if !path.is_file() {
                app.log(&format!("Extracting netsock loader to {}", path.display()));
                if let Some(bytes) = payload::load(
                    app,
                    "netsock.so",
                    NETSNOCK_URL,
                    include_bytes!("../netsock.so"),
                ) {
                    extract_so(app, &bytes, &path);
                }
            }
            let current = std::env::var("LD_AUDIT").unwrap_or_default();
            let merged = merge_ld_audit(&current, &path.to_string_lossy());
            std::env::set_var("LD_AUDIT", merged);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shorthand for the monitor assertions.
    fn monitor(name: &str, detail: &str) -> Monitor {
        Monitor {
            name: name.to_string(),
            detail: detail.to_string(),
        }
    }

    #[test]
    fn merge_ld_audit_prepends_netsock_so() {
        // Empty current -> just netsock.so
        let result = merge_ld_audit("", "/home/user/.config/SLSsteam/tools/netsock/netsock.so");
        assert_eq!(
            result,
            "/home/user/.config/SLSsteam/tools/netsock/netsock.so"
        );

        // Existing value -> prepended with colon
        let result = merge_ld_audit(
            "/other/lib.so",
            "/home/user/.config/SLSsteam/tools/netsock/netsock.so",
        );
        assert_eq!(
            result,
            "/home/user/.config/SLSsteam/tools/netsock/netsock.so:/other/lib.so"
        );

        // Empty netsock path -> no change
        let result = merge_ld_audit("/existing.so", "");
        assert_eq!(result, "/existing.so");

        // Both empty -> empty
        let result = merge_ld_audit("", "");
        assert_eq!(result, "");
    }

    #[test]
    fn command_exists_detects_executables() {
        // This test binary is a real executable, referenced by absolute path.
        let me = std::env::current_exe().unwrap();
        assert!(command_exists(me.to_str().unwrap()));

        // Missing absolute path and missing bare name are both false.
        assert!(!command_exists("/nonexistent/definitely/not/here_zzz"));
        assert!(!command_exists("game_wrapper_missing_binary_zzz123"));
    }

    #[test]
    fn process_running_finds_the_current_process() {
        let me = std::fs::read_to_string("/proc/self/comm").unwrap();
        assert!(process_running(&[me.trim()]));
        assert!(!process_running(&["game_missing_process_zzz"]));
    }

    #[test]
    fn kernel_release_is_readable() {
        // Guards the file used for the BORE fallback check.
        assert!(!kernel_release().is_empty());
    }

    #[test]
    fn wayland_info_monitors_are_parsed() {
        let text = "\
interface: 'wl_drm',                              version:  2, name: 12
interface: 'wl_output',                           version:  4, name: 65
\tname: DP-1
\tdescription: Promotion and Display Technology Ltd. 27GM620BF DP-1
\tx: 1920, y: 0, scale: 1,
\tmode:
\t\twidth: 1024 px, height: 768 px, refresh: 60.000 Hz,
\t\tflags:
\t\twidth: 1920 px, height: 1080 px, refresh: 165.001 Hz,
\t\tflags: current
interface: 'kde_output_order_v1',                 version:  1, name: 68
interface: 'wl_output',                           version:  4, name: 66
\tname: DP-2
\tx: 0, y: 0, scale: 1,
\tmode:
\t\twidth: 1920 px, height: 1080 px, refresh: 60.000 Hz,
\t\tflags: current
";
        assert_eq!(
            monitors_from_wayland_info(text),
            vec![
                monitor("DP-1", "1920x1080 at 1920,0"),
                monitor("DP-2", "1920x1080 at 0,0"),
            ]
        );
    }

    #[test]
    fn json_monitors_are_parsed() {
        let hyprctl = r#"[{"id":0,"name":"DP-1","description":"Dell","monitor":"DP-1"}]"#;
        assert_eq!(monitors_from_json(hyprctl), vec![monitor("DP-1", "")]);

        let wlr_randr = r#"[{"name":"eDP-1","enabled":true},{"name":"HDMI-A-1","enabled":false}]"#;
        assert_eq!(
            monitors_from_json(wlr_randr),
            vec![monitor("eDP-1", ""), monitor("HDMI-A-1", "")]
        );

        assert!(monitors_from_json("not json").is_empty());
    }

    #[test]
    fn kscreen_monitors_are_parsed_and_colors_stripped() {
        let text = "\
\u{1b}[01;32mOutput: \u{1b}[0;0m1 DP-2 20ea350b-uuid
\t\u{1b}[01;32menabled\u{1b}[0;0m
\t\u{1b}[01;32mconnected\u{1b}[0;0m
\t\u{1b}[01;33mGeometry: \u{1b}[0;0m0,0 1920x1080
\u{1b}[01;32mOutput: \u{1b}[0;0m2 DP-1 4f508cbc-uuid
\t\u{1b}[01;32menabled\u{1b}[0;0m
\t\u{1b}[01;33mdisconnected\u{1b}[0;0m
\t\u{1b}[01;33mGeometry: \u{1b}[0;0m1920,0 1920x1080
";
        assert_eq!(
            monitors_from_kscreen(text),
            vec![monitor("DP-2", "1920x1080 at 0,0")]
        );
    }

    #[test]
    fn monitor_detail_is_composed_from_what_is_known() {
        assert_eq!(monitor_detail("1920x1080", "0,0"), "1920x1080 at 0,0");
        assert_eq!(monitor_detail("1920x1080", ""), "1920x1080");
        assert_eq!(monitor_detail("", "0,0"), "at 0,0");
        assert_eq!(monitor_detail("", ""), "");
    }

    #[test]
    fn hyprland_active_monitor_is_parsed() {
        let text = r#"{"id":1,"name":"1","monitor":"DP-2","windows":2}"#;
        assert_eq!(json_string_field(text, "monitor"), Some("DP-2".to_string()));
        assert_eq!(json_string_field("{}", "monitor"), None);
        assert_eq!(json_string_field(r#"{"monitor":""}"#, "monitor"), None);
        assert_eq!(json_string_field("not json", "monitor"), None);
    }

    #[test]
    fn sway_focused_output_is_parsed() {
        let text = r#"[{"name":"1","focused":false,"output":"DP-2"},{"name":"2","focused":true,"output":"DP-1"}]"#;
        assert_eq!(focused_workspace_output(text), Some("DP-1".to_string()));

        let none_focused = r#"[{"name":"1","focused":false,"output":"DP-2"}]"#;
        assert_eq!(focused_workspace_output(none_focused), None);
        assert_eq!(focused_workspace_output("not json"), None);
    }

    #[test]
    fn wayland_monitor_is_only_exported_with_wayland() {
        let mut app = App::default();
        assert_eq!(wayland_monitor_env(&app), None);

        app.wayland_monitor = "DP-1".to_string();
        assert_eq!(wayland_monitor_env(&app), None, "Wayland is off");

        app.wayland_enabled = true;
        assert_eq!(wayland_monitor_env(&app), Some("DP-1"));
    }

    #[test]
    fn gaming_mode_detection() {
        // Gamescope session type.
        std::env::set_var("XDG_SESSION_TYPE", "gamescope");
        std::env::remove_var("XDG_CURRENT_DESKTOP");
        std::env::remove_var("SteamGameId");
        assert!(is_gaming_mode());

        // Steam session type with a game running.
        std::env::set_var("XDG_SESSION_TYPE", "steam");
        std::env::set_var("XDG_CURRENT_DESKTOP", "steam");
        std::env::set_var("SteamGameId", "12345");
        assert!(is_gaming_mode());

        // Desktop session is not gaming mode.
        std::env::set_var("XDG_SESSION_TYPE", "wayland");
        std::env::set_var("XDG_CURRENT_DESKTOP", "KDE");
        std::env::remove_var("SteamGameId");
        assert!(!is_gaming_mode());
    }
}
