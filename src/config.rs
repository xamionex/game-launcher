//! Configuration defaults and shared runtime state.
//!
//! Mirrors the variable block at the top of the original `game.sh`.

use std::path::PathBuf;

/// Number of plain `.log` files kept per game folder; older ones are archived
/// to `.tar.gz` on the next launch.
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
/// `value` holds the literal `(empty)` marker when the user requested an unset
/// (e.g. `LD_PRELOAD=`), matching the original script semantics.
#[derive(Debug, Clone)]
pub struct CustomExport {
    pub name: String,
    pub before: String,
    pub value: String,
}

/// Sentinel stored in [`CustomExport::value`] meaning "unset this variable".
pub const EMPTY_MARKER: &str = "(empty)";

/// Parsed flags plus runtime state, threaded through the launch pipeline.
#[derive(Debug)]
pub struct App {
    // === Toggles enabled by default (can be disabled) ===
    pub gamemode: bool,
    pub mangohud: bool,
    pub protonhax: bool,
    pub pressure_vessel: bool,
    pub speedhack: bool,
    /// Set `STEAM_COMPAT_RUNTIME_SDL3=0` when true (the `-L` flag).
    pub disable_sdl3: bool,
    pub wayland_force_enable: bool,
    pub wayland_force_disable: bool,
    pub enable_custom_vkd3d: bool,

    // === Toggles disabled by default (can be enabled) ===
    pub gamescope: bool,
    pub gamescope_wayland: bool,
    pub wezterm: bool,
    pub onlinefix: bool,
    pub cleanup_mods_on_exit: bool,
    pub lsfg: bool,
    pub modding_support: bool,

    // === Valued flags ===
    pub logging_level: i32,
    pub instances: u32,
    pub replacement_exe: String,
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
            protonhax: true,
            pressure_vessel: true,
            speedhack: true,
            disable_sdl3: false,
            wayland_force_enable: false,
            wayland_force_disable: false,
            enable_custom_vkd3d: false,

            gamescope: false,
            gamescope_wayland: false,
            wezterm: false,
            onlinefix: false,
            cleanup_mods_on_exit: false,
            lsfg: false,
            modding_support: false,

            logging_level: 0,
            instances: 1,
            replacement_exe: String::new(),
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
    /// Append a line to the active log file, if logging is enabled.
    ///
    /// Mirrors `echo ... >> "$LOG_FILE"`; a no-op when no log file is set
    /// (e.g. silent mode), and errors are ignored just like the shell.
    pub fn log(&self, msg: &str) {
        if let Some(path) = &self.log_file {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{msg}");
            }
        }
    }
}

/// Base directory for logs: `$HOME/logs/game`.
pub fn log_base() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("logs").join("game")
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
