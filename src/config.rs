//! Configuration defaults and shared runtime state.

use std::path::PathBuf;

/// Number of plain `.log` files kept per game folder; older ones are archived to `.tar.gz` on the next launch.
pub const MAX_LOGS: usize = 3;
/// tmpfs mount point used for RAM-disk loading.
pub const RAM_MOUNT: &str = "/mnt/gameram";

/// DLL overrides applied when `-o` (OnlineFix) is enabled.
pub const ONLINEFIX_DLLS: &[&str] = &[
    "OnlineFix64=n,b",
    "SteamOverlay64=n,b",
    "winmm=n,b",
    "dnet=n,b",
    "steam_api64=n,b",
];

/// DLL overrides applied when `-m` (modding support) is enabled.
pub const MODDING_DLLS: &[&str] = &["dwmapi=n,b", "winhttp=n,b", "winmm=n,b", "version=n,b"];

/// A custom environment export captured from a positional `KEY=VALUE` token.
///
/// `value` holds the literal `(empty)` marker when the user requested an unset (e.g. `LD_PRELOAD=`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomExport {
    pub name: String,
    pub before: String,
    pub value: String,
}

/// Sentinel stored in [`CustomExport::value`] meaning "unset this variable".
pub const EMPTY_MARKER: &str = "(empty)";

/// Build a [`CustomExport`] from a `NAME=VALUE` string, capturing the current value of the variable for before/after logging.
pub fn make_export(assignment: &str) -> CustomExport {
    let (name, value) = match assignment.split_once('=') {
        Some((n, v)) => (n.to_string(), v.to_string()),
        None => (assignment.to_string(), String::new()),
    };
    let value = if value.is_empty() {
        EMPTY_MARKER.to_string()
    } else {
        value
    };
    let before = std::env::var(&name).unwrap_or_else(|_| "(unset)".to_string());
    CustomExport {
        name,
        before,
        value,
    }
}

/// Parsed flags plus runtime state, threaded through the launch pipeline.
///
/// `PartialEq` is derived so tests can assert that the config file defaults
/// match [`App::default`].
#[derive(Debug, PartialEq)]
pub struct App {
    // === Toggles enabled by default (can be disabled) ===
    pub gamemode: bool,
    pub mangohud: bool,
    /// Force MangoHud on even in gaming mode (`-H`).
    pub mangohud_force: bool,
    pub protonhax: bool,
    pub wayland_force_enable: bool,
    pub wayland_force_disable: bool,
    pub enable_custom_vkd3d: bool,

    // === Toggles disabled by default (can be enabled) ===
    pub pressure_vessel: bool,
    /// Set `STEAM_COMPAT_RUNTIME_SDL3=0` when true (the `-L` flag).
    pub disable_sdl3: bool,
    pub gamescope: bool,
    pub gamescope_wayland: bool,
    pub wezterm: bool,
    pub onlinefix: bool,
    pub cleanup_mods_on_exit: bool,
    pub lsfg: bool,
    pub modding_support: bool,
    pub fix_audit: bool,
    /// Enable LinuwUx hypervisor loader via `LD_PRELOAD` (the `-v` flag).
    pub hypervisor: bool,
    /// Swap the game's `EOSSDK-Win64-Shipping.dll` for the EOS proxy (the `-E` flag).
    pub eos_proxy: bool,

    /// Open the interactive config editor and exit (the `-C` flag).
    /// Not a config file setting; it is a command-line action.
    pub config_tui: bool,

    // === Valued flags ===
    pub logging_level: i32,
    pub instances: u32,
    pub replacement_exe: String,
    /// Wayland output used as the primary monitor by the Wine Wayland driver (`WAYLANDDRV_PRIMARY_MONITOR`), for example `DP-1`.
    /// Only applied when Wayland is enabled.
    pub wayland_monitor: String,
    pub winedlloverrides_list: Vec<String>,
    pub mods_to_launch: Vec<String>,
    pub custom_exports: Vec<CustomExport>,

    // === RAM-disk options ===
    pub use_ramdisk: bool,
    pub ramdisk_size: String,
    pub sync_back_on_exit: bool,

    // === Runtime state ===
    pub original_cmd: Vec<String>,
    pub cmd: Vec<String>,
    pub isproton: bool,
    pub proton_path: String,
    pub proton_ver: String,
    pub wayland_enabled: bool,
    pub log_file: Option<PathBuf>,
    pub appid: String,
    pub game_name: String,
    pub mod_pids: Vec<u32>,
    pub game_dir_orig: Option<String>,
    pub game_dir_ram: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        App {
            gamemode: true,
            mangohud: true,
            mangohud_force: false,
            protonhax: true,
            wayland_force_enable: false,
            wayland_force_disable: false,
            enable_custom_vkd3d: false,

            pressure_vessel: false,
            disable_sdl3: false,
            gamescope: false,
            gamescope_wayland: false,
            wezterm: false,
            onlinefix: false,
            cleanup_mods_on_exit: false,
            lsfg: false,
            modding_support: false,
            fix_audit: false,
            hypervisor: false,
            eos_proxy: false,

            config_tui: false,

            logging_level: 0,
            instances: 1,
            replacement_exe: String::new(),
            wayland_monitor: String::new(),
            winedlloverrides_list: Vec::new(),
            mods_to_launch: Vec::new(),
            custom_exports: Vec::new(),

            use_ramdisk: false,
            ramdisk_size: "32G".to_string(),
            sync_back_on_exit: true,

            original_cmd: Vec::new(),
            cmd: Vec::new(),
            isproton: false,
            proton_path: String::new(),
            proton_ver: String::new(),
            wayland_enabled: false,
            log_file: None,
            appid: String::new(),
            game_name: String::new(),
            mod_pids: Vec::new(),
            game_dir_orig: None,
            game_dir_ram: None,
        }
    }
}

impl App {
    /// Append the OnlineFix DLL overrides, shared by the `-o` flag and the `onlinefix` config setting.
    pub fn add_onlinefix_dlls(&mut self) {
        self.winedlloverrides_list
            .extend(ONLINEFIX_DLLS.iter().map(|s| s.to_string()));
    }

    /// Append the modding-support DLL overrides, shared by the `-m` flag and the `modding_support` config setting.
    pub fn add_modding_dlls(&mut self) {
        self.winedlloverrides_list
            .extend(MODDING_DLLS.iter().map(|s| s.to_string()));
    }

    /// Append a line to the active log file, if logging is enabled.
    pub fn log(&self, msg: &str) {
        if let Some(path) = &self.log_file {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(f, "{msg}");
            }
        }
    }
}

/// Ensure `$HOME/logs` exists, creating it if missing, and return it.
pub fn ensure_logs_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let logs = PathBuf::from(home).join("logs");
    let _ = std::fs::create_dir_all(&logs);
    logs
}

/// Base directory for logs: `$HOME/logs/game`.
pub fn log_base() -> PathBuf {
    ensure_logs_dir().join("game")
}

/// Send a best-effort desktop notification via `notify-send`.
///
/// Silently does nothing if `notify-send` is unavailable.
pub fn notify(summary: &str, body: &str) {
    let mut cmd = std::process::Command::new("notify-send");
    cmd.arg(summary);
    if !body.is_empty() {
        cmd.arg(body);
    }
    let _ = cmd.status();
}
