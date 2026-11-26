//! The user config file: `~/.config/game-launcher/config.toml`.
//!
//! The file is written with [`DEFAULT_CONFIG`] the first time the launcher runs and is never overwritten afterwards, so edits survive.
//! Its values seed [`App`] before the command line is parsed, which means flags always win.
//! Lists (`dll_overrides`, `mods`, `exports`) are appended to by their flags.
//!
//! A file that cannot be parsed is ignored as a whole, keeping the built-in defaults; the reason is printed to stderr and written to the launch log.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value};

use crate::config::{
    make_export, App, GameProfile, LEGION_MEMORY_DEFAULT_MIB, LEGION_MEMORY_MAX_MIB,
    LEGION_MEMORY_MIN_MIB, LEGION_TIMEOUT_DEFAULT_SECONDS, LEGION_TIMEOUT_MAX_SECONDS,
    LEGION_TIMEOUT_MIN_SECONDS,
};

/// Directory holding the launcher's own files: `~/.config/game-launcher`.
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("game-launcher"))
}

/// Path of the user config file: `~/.config/game-launcher/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("config.toml"))
}

/// Directory caching payloads fetched from upstream releases:
/// `~/.config/game-launcher/payloads`.
pub fn payloads_dir() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("payloads"))
}

/// Default config file contents, written on first run.
///
/// The keys here, [`FileConfig`] and the `-C` editor's field table are kept in sync by tests:
/// `default_config_matches_builtin_defaults` and `field_keys_match_default_config`.
pub const DEFAULT_CONFIG: &str = r#"# game-launcher configuration.
#
# Created with defaults on first run; edit and save, values are read on every launch.
# Command-line flags override whatever is set here, and the list values below are appended to by their flags.

# --- Tools enabled by default (disable with the flag) ---

# GameMode. -g disables it. Skipped automatically on BORE kernels and when ananicy-cpp is running, since both conflict with GameMode's renicing.
gamemode = true

# MangoHud overlay. -H disables it.
mangohud = true

# Force MangoHud on in gaming mode (handhelds). -Y.
mangohud_force = false

# ProtonHax launch hooks. -p disables them.
protonhax = true

# Force Wayland on regardless of GPU vendor. -W.
wayland_force_enable = false

# Force Wayland off. -X.
wayland_force_disable = false

# --- Tools disabled by default (enable with the flag) ---

# Pressure Vessel elimination. -P.
pressure_vessel = false

# SDL3 elimination in the Steam runtime. -L.
disable_sdl3 = false

# Gamescope, X11 backend. -s.
gamescope = false

# Gamescope, Wayland backend. -S.
gamescope_wayland = false

# Run the game in wezterm. -w.
wezterm = false

# OnlineFix DLL overrides. -o.
onlinefix = false

# Kill mod processes when the launcher exits. -e.
cleanup_mods_on_exit = false

# LSFG-VK frame generation. -f.
lsfg = false

# Modding support DLL overrides. -m.
modding_support = false

# LD_AUDIT netsock loader, cached and installed on demand. -F.
fix_audit = false

# Hypervisor kernel modules: unloads KVM (killing running virtual machines), loads cpuid_fault_emulation before the game and restores the previous module state when it exits, via modprobe as root. -h.
hypervisor_modules = false

# LinuwUx hypervisor loader, self-extracted. -v.
hypervisor = false

# Custom vkd3d-proton from ~/Projects/vkd3d-proton. -V.
enable_custom_vkd3d = false

# EOS-Proxy: replaces the game's EOSSDK-Win64-Shipping.dll. -E.
eos_proxy = false

# --- Values ---

# -1 silent, 0 normal, 1 verbose. -l.
logging_level = 0

# Number of instances (accepted, currently inert). -i.
instances = 1

# Replace the launched executable. -r.
replacement_exe = ""

# Wayland/X11 default when neither force flag is given.
# "auto" lets the GPU decide (NVIDIA: Wayland, else X11).
# "wayland" or "x11" change that choice so that -W and -X override it.
wayland_default = "auto"

# Primary monitor for the Wine Wayland driver, e.g. "DP-1".
# This sets WAYLANDDRV_PRIMARY_MONITOR and is only applied when Wayland is enabled (see wayland_force_enable above). -M sets it, or pick one from the detected monitors with -C.
wayland_monitor = ""

# --- Legion KVM (experimental) ---

# Run Proton games inside Legion's KVM virtual machine. -K enables it for one launch.
# Requires /dev/kvm, a Wayland host session, a Vulkan driver and the bundled legion binary.
legion = false

# Legion executable. Empty means next to the game binary, then PATH, ~/.local/bin/legion, /usr/local/bin/legion.
legion_path = ""

# Per-game Legion profiles. -C opens the list; a profile is also created automatically on the first Legion launch of a game.
# Profiles are matched by Steam App ID first, then by executable file name, so the same game works from Steam and from a direct .exe launch.
# Example:
# [[legion_games]]
# name = "My game"
# app_id = "123456"
# exe = "game.exe"
# enabled = true
# memory_mib = 12288
# render_node = ""
# fullscreen = false
# startup_ui = "auto"
# startup_timeout_seconds = 180
legion_games = []

# --- Lists (command-line flags append to these) ---

# Extra DLL overrides, one entry per override, e.g. ["dinput8=n,b", "dxgi=n,b"]. -d.
dll_overrides = []

# Background mod commands, one entry per command, e.g. ["./mod-loader.sh"]. -u.
mods = []

# Environment exports as "NAME=VALUE", e.g. ["PROTON_NO_ESYNC=1"]; "NAME=" unsets.
# Positional KEY=VALUE tokens on the command line are added to these.
exports = []
"#;

/// A parsed config file.
///
/// Every field is optional: `None` means the line was absent and the built-in default is kept, so users can delete lines they do not care about.
/// Keys the launcher does not know are ignored rather than rejected, because a config written by a newer version (or one with a typo) must never take the whole file down.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct FileConfig {
    pub mangohud: Option<bool>,
    pub mangohud_force: Option<bool>,
    pub protonhax: Option<bool>,
    pub wayland_force_enable: Option<bool>,
    pub wayland_force_disable: Option<bool>,

    pub gamemode: Option<bool>,
    pub pressure_vessel: Option<bool>,
    pub disable_sdl3: Option<bool>,
    pub gamescope: Option<bool>,
    pub gamescope_wayland: Option<bool>,
    pub wezterm: Option<bool>,
    pub onlinefix: Option<bool>,
    pub cleanup_mods_on_exit: Option<bool>,
    pub lsfg: Option<bool>,
    pub modding_support: Option<bool>,
    pub fix_audit: Option<bool>,
    pub hypervisor_modules: Option<bool>,
    pub hypervisor: Option<bool>,
    pub enable_custom_vkd3d: Option<bool>,
    pub eos_proxy: Option<bool>,

    pub logging_level: Option<i32>,
    pub instances: Option<u32>,
    pub replacement_exe: Option<String>,
    pub wayland_default: Option<String>,
    pub wayland_monitor: Option<String>,

    pub legion: Option<bool>,
    pub legion_path: Option<String>,
    pub legion_games: Option<Vec<GameProfile>>,

    pub dll_overrides: Option<Vec<String>>,
    pub mods: Option<Vec<String>>,
    pub exports: Option<Vec<String>>,
}

/// Sanitize a profile from the config file.
///
/// Out of range Legion values would make the launch fail later, so they are replaced with defaults and reported here instead.
fn validate_profile(profile: &GameProfile, notes: &mut Vec<String>) -> GameProfile {
    let mut profile = profile.clone();
    let label = profile.display_name();
    profile.name = profile.name.trim().to_string();
    profile.app_id = profile.app_id.trim().to_string();
    profile.exe = profile.exe.trim().to_string();
    profile.render_node = profile.render_node.trim().to_string();

    if !profile.app_id.is_empty() && !profile.app_id.chars().all(|c| c.is_ascii_digit()) {
        notes.push(format!(
            "Config: Legion profile {label:?} has a non-numeric app_id; ignoring the app_id matcher"
        ));
        profile.app_id.clear();
    }
    if !(LEGION_MEMORY_MIN_MIB..=LEGION_MEMORY_MAX_MIB).contains(&profile.memory_mib) {
        notes.push(format!(
            "Config: Legion profile {label:?} has memory_mib {} outside {LEGION_MEMORY_MIN_MIB}..{LEGION_MEMORY_MAX_MIB}; using {LEGION_MEMORY_DEFAULT_MIB}",
            profile.memory_mib
        ));
        profile.memory_mib = LEGION_MEMORY_DEFAULT_MIB;
    }
    if !(LEGION_TIMEOUT_MIN_SECONDS..=LEGION_TIMEOUT_MAX_SECONDS)
        .contains(&profile.startup_timeout_seconds)
    {
        notes.push(format!(
            "Config: Legion profile {label:?} has startup_timeout_seconds {} outside {LEGION_TIMEOUT_MIN_SECONDS}..{LEGION_TIMEOUT_MAX_SECONDS}; using {LEGION_TIMEOUT_DEFAULT_SECONDS}",
            profile.startup_timeout_seconds
        ));
        profile.startup_timeout_seconds = LEGION_TIMEOUT_DEFAULT_SECONDS;
    }
    if !matches!(
        profile.startup_ui.as_str(),
        "auto" | "window" | "log" | "off"
    ) {
        notes.push(format!(
            "Config: Legion profile {label:?} has startup_ui {:?} (expected auto, window, log or off); using auto",
            profile.startup_ui
        ));
        profile.startup_ui = "auto".to_string();
    }
    profile
}

/// Copy the set values of `cfg` onto `app`, returning notes for the log.
fn apply(cfg: &FileConfig, app: &mut App) -> Vec<String> {
    let mut notes = Vec::new();

    macro_rules! set {
        ($($field:ident),+ $(,)?) => {
            $( if let Some(v) = cfg.$field { app.$field = v; } )+
        };
    }
    set!(
        mangohud,
        mangohud_force,
        protonhax,
        wayland_force_enable,
        wayland_force_disable,
        gamemode,
        pressure_vessel,
        disable_sdl3,
        gamescope,
        gamescope_wayland,
        wezterm,
        onlinefix,
        cleanup_mods_on_exit,
        lsfg,
        modding_support,
        fix_audit,
        hypervisor_modules,
        hypervisor,
        enable_custom_vkd3d,
        eos_proxy,
        instances,
    );

    if let Some(level) = cfg.logging_level {
        if (-1..=1).contains(&level) {
            app.logging_level = level;
        } else {
            notes.push(format!(
                "Config: logging_level {level} is invalid (-1, 0 or 1), keeping {}",
                app.logging_level
            ));
        }
    }
    if let Some(v) = &cfg.replacement_exe {
        app.replacement_exe = v.clone();
    }
    if let Some(v) = &cfg.wayland_default {
        match v.as_str() {
            "auto" | "x11" | "wayland" => app.wayland_default = v.clone(),
            _ => notes.push(format!(
                "Config: wayland_default {v:?} is invalid (auto, x11 or wayland), keeping {}",
                app.wayland_default
            )),
        }
    }
    if let Some(v) = &cfg.wayland_monitor {
        app.wayland_monitor = v.clone();
    }
    if let Some(v) = cfg.legion {
        app.legion = v;
    }
    if let Some(v) = &cfg.legion_path {
        app.legion_path = v.trim().to_string();
    }
    if let Some(games) = &cfg.legion_games {
        app.legion_games = games
            .iter()
            .map(|game| validate_profile(game, &mut notes))
            .collect();
    }
    if let Some(list) = &cfg.dll_overrides {
        app.winedlloverrides_list = list.clone();
    }
    if let Some(list) = &cfg.mods {
        app.mods_to_launch = list.clone();
    }
    if let Some(list) = &cfg.exports {
        app.custom_exports = list.iter().map(|e| make_export(e)).collect();
    }

    // The `-o` and `-m` flags also pull in their DLL override sets, so the settings must do the same.
    if app.onlinefix {
        app.add_onlinefix_dlls();
    }
    if app.modding_support {
        app.add_modding_dlls();
    }

    notes
}

/// Load the config file into `app`, creating it with defaults when missing.
///
/// Returns human readable notes for the launch log.
/// Never fails: a bad file is reported and skipped so a game can still launch.
pub fn load(app: &mut App) -> Vec<String> {
    match config_path() {
        Some(path) => load_from(app, &path),
        None => {
            let note = "Config: could not resolve a config directory, using built-in defaults";
            eprintln!("game: {note}");
            vec![note.to_string()]
        }
    }
}

/// Top-level keys that the launcher does not know about.
///
/// Compared against [`DEFAULT_CONFIG`], which is kept in sync with the parser, so a typo (or a key from a newer version) can be reported without failing the launch.
fn unknown_keys(doc: &DocumentMut) -> Vec<String> {
    let defaults = default_document();
    let known: Vec<&str> = defaults.iter().map(|(key, _)| key).collect();
    doc.iter()
        .filter(|(key, _)| !known.contains(key))
        .map(|(key, _)| key.to_string())
        .collect()
}

/// A hint appended to permission errors.
///
/// A config directory created by a root install (or an earlier `sudo` run) belongs to root, and the launcher runs as the user, so writing there fails with `Permission denied (os error 13)`.
pub fn permission_hint(error: &std::io::Error, path: &Path) -> String {
    if error.kind() != std::io::ErrorKind::PermissionDenied {
        return String::new();
    }
    let dir = path.parent().unwrap_or(path);
    let user = std::env::var("USER").unwrap_or_else(|_| "$USER".to_string());
    format!(" (check ownership: sudo chown -R {user} {})", dir.display())
}

/// [`load`] for an explicit path, used by tests.
fn load_from(app: &mut App, path: &Path) -> Vec<String> {
    if !path.is_file() {
        return create_default(path);
    }

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            let note = format!(
                "Config: failed to read {}: {e}{}",
                path.display(),
                permission_hint(&e, path)
            );
            eprintln!("game: {note}");
            return vec![note];
        }
    };

    match toml::from_str::<FileConfig>(&text) {
        Ok(cfg) => {
            let mut notes = apply(&cfg, app);
            notes.insert(0, format!("Config: loaded {}", path.display()));
            if let Ok(doc) = text.parse::<DocumentMut>() {
                let unknown = unknown_keys(&doc);
                if !unknown.is_empty() {
                    notes.push(format!(
                        "Config: ignoring unknown keys in {}: {} (typo, or written by a newer version)",
                        path.display(),
                        unknown.join(", ")
                    ));
                }
            }
            notes
        }
        Err(e) => {
            let note = format!(
                "Config: failed to parse {}: {e}. Using built-in defaults.",
                path.display()
            );
            eprintln!("game: {note}");
            vec![note]
        }
    }
}

/// Write [`DEFAULT_CONFIG`] to `path`, creating parent directories.
fn create_default(path: &Path) -> Vec<String> {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            let note = format!(
                "Config: failed to create {}: {e}{}",
                parent.display(),
                permission_hint(&e, parent)
            );
            eprintln!("game: {note}");
            return vec![note];
        }
    }
    match std::fs::write(path, DEFAULT_CONFIG) {
        Ok(()) => vec![format!(
            "Config: created default config at {}",
            path.display()
        )],
        Err(e) => {
            let note = format!(
                "Config: failed to write {}: {e}{}",
                path.display(),
                permission_hint(&e, path)
            );
            eprintln!("game: {note}");
            vec![note]
        }
    }
}

/// Parse [`DEFAULT_CONFIG`] into an editable document.
pub fn default_document() -> DocumentMut {
    DEFAULT_CONFIG
        .parse::<DocumentMut>()
        .expect("DEFAULT_CONFIG is valid TOML")
}

/// Read the config file for editing, falling back to the defaults.
///
/// Returns the document and an optional warning to show in the editor, either because the existing file could not be parsed or because it contains keys this version does not know (which are kept as they are).
pub fn load_document(path: &Path) -> (DocumentMut, Option<String>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (default_document(), None);
    };
    match text.parse::<DocumentMut>() {
        Ok(doc) => {
            let unknown = unknown_keys(&doc);
            let warning = if unknown.is_empty() {
                None
            } else {
                Some(format!(
                    "Ignoring unknown keys: {} (typo, or written by a newer version)",
                    unknown.join(", ")
                ))
            };
            (doc, warning)
        }
        Err(e) => (
            default_document(),
            Some(format!(
                "{} could not be parsed ({e}), showing defaults",
                path.display()
            )),
        ),
    }
}

/// Write an edited document back to `path`, creating parent directories.
pub fn save_document(doc: &DocumentMut, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "failed to create {}: {e}{}",
                parent.display(),
                permission_hint(&e, parent)
            )
        })?;
    }
    std::fs::write(path, doc.to_string()).map_err(|e| {
        format!(
            "failed to write {}: {e}{}",
            path.display(),
            permission_hint(&e, path)
        )
    })
}

/// TOML table for one Legion profile, with a stable field order.
pub fn profile_table(profile: &GameProfile) -> Table {
    let mut table = Table::new();
    table.insert("name", Item::Value(Value::from(profile.name.clone())));
    table.insert("app_id", Item::Value(Value::from(profile.app_id.clone())));
    table.insert("exe", Item::Value(Value::from(profile.exe.clone())));
    table.insert("enabled", Item::Value(Value::from(profile.enabled)));
    table.insert(
        "memory_mib",
        Item::Value(Value::from(i64::from(profile.memory_mib))),
    );
    table.insert(
        "render_node",
        Item::Value(Value::from(profile.render_node.clone())),
    );
    table.insert("fullscreen", Item::Value(Value::from(profile.fullscreen)));
    table.insert(
        "startup_ui",
        Item::Value(Value::from(profile.startup_ui.clone())),
    );
    table.insert(
        "startup_timeout_seconds",
        Item::Value(Value::from(i64::from(profile.startup_timeout_seconds))),
    );
    table
}

/// Read one profile table, filling in defaults for keys the user left out.
///
/// Hand written configs may use a minimal `[[legion_games]]` entry, and the editor shows the same defaults the launcher would apply.
pub fn profile_from_table(table: &Table) -> GameProfile {
    let mut profile = GameProfile::default();
    if let Some(value) = table.get("name").and_then(Item::as_str) {
        profile.name = value.to_string();
    }
    if let Some(value) = table.get("app_id").and_then(Item::as_str) {
        profile.app_id = value.to_string();
    }
    if let Some(value) = table.get("exe").and_then(Item::as_str) {
        profile.exe = value.to_string();
    }
    if let Some(value) = table.get("enabled").and_then(Item::as_bool) {
        profile.enabled = value;
    }
    if let Some(value) = table.get("memory_mib").and_then(Item::as_integer) {
        profile.memory_mib = u32::try_from(value).unwrap_or(profile.memory_mib);
    }
    if let Some(value) = table.get("render_node").and_then(Item::as_str) {
        profile.render_node = value.to_string();
    }
    if let Some(value) = table.get("fullscreen").and_then(Item::as_bool) {
        profile.fullscreen = value;
    }
    if let Some(value) = table.get("startup_ui").and_then(Item::as_str) {
        profile.startup_ui = value.to_string();
    }
    if let Some(value) = table
        .get("startup_timeout_seconds")
        .and_then(Item::as_integer)
    {
        profile.startup_timeout_seconds =
            u32::try_from(value).unwrap_or(profile.startup_timeout_seconds);
    }
    profile
}

/// Every profile in a document, with defaults filled in.
///
/// Both the `[[legion_games]]` form written by `-C` and a hand written inline array are read.
pub fn profiles_in(doc: &DocumentMut) -> Vec<GameProfile> {
    match doc.get("legion_games") {
        Some(Item::ArrayOfTables(tables)) => tables.iter().map(profile_from_table).collect(),
        Some(Item::Value(Value::Array(array))) => array
            .iter()
            .filter_map(Value::as_inline_table)
            .map(|inline| profile_from_table(&inline.clone().into_table()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Mutable `[[legion_games]]` table, converting the default empty array on first use.
///
/// The value is replaced in place so the key keeps its comments, and an inline array is converted rather than dropped.
pub fn profile_tables_mut(doc: &mut DocumentMut) -> &mut ArrayOfTables {
    if !doc
        .get("legion_games")
        .is_some_and(Item::is_array_of_tables)
    {
        let mut converted = ArrayOfTables::new();
        if let Some(array) = doc.get("legion_games").and_then(Item::as_array) {
            for value in array {
                if let Some(inline) = value.as_inline_table() {
                    converted.push(inline.clone().into_table());
                }
            }
        }
        match doc.as_table_mut().get_key_value_mut("legion_games") {
            Some((mut key, item)) => {
                // The array syntax puts a space before `=`; an array of tables must not keep it before `]]`.
                key.leaf_decor_mut().set_suffix("");
                *item = Item::ArrayOfTables(converted);
            }
            None => {
                doc.as_table_mut()
                    .insert("legion_games", Item::ArrayOfTables(converted));
            }
        }
    }
    doc.get_mut("legion_games")
        .and_then(Item::as_array_of_tables_mut)
        .expect("legion_games was just made an array of tables")
}

/// Append a profile, keeping the comments attached to `legion_games`.
pub fn push_profile(doc: &mut DocumentMut, profile: &GameProfile) -> usize {
    let tables = profile_tables_mut(doc);
    tables.push(profile_table(profile));
    tables.len() - 1
}

/// Set one field of a profile table, keeping the comments around the value.
pub fn set_profile_field(table: &mut Table, key: &str, new: Value) {
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(value) => {
            let decor = value.decor().clone();
            *value = new;
            *value.decor_mut() = decor;
        }
        None => {
            table.insert(key, Item::Value(new));
        }
    }
}

/// Put `legion_games` back to the empty array the default config uses.
pub fn clear_profiles(doc: &mut DocumentMut) {
    doc["legion_games"] = Item::Value(Value::Array(Array::new()));
    // An array keeps the spacing around `=` that the array-of-tables form drops.
    if let Some(mut key) = doc.as_table_mut().key_mut("legion_games") {
        key.leaf_decor_mut().set_suffix(" ");
    }
}

/// Append a generated profile to the config file, keeping comments and formatting.
///
/// Used for the automatic profile created on the first Legion launch of a game.
pub fn add_profile(profile: &GameProfile) -> Result<(), String> {
    let path = config_path().ok_or("could not resolve a config directory")?;
    append_profile_at(&path, profile)
}

/// [`add_profile`] for an explicit path.
///
/// A config that exists but cannot be parsed is left alone instead of being replaced with defaults, so a broken file never loses the user's settings.
fn append_profile_at(path: &Path, profile: &GameProfile) -> Result<(), String> {
    let mut doc = if path.is_file() {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        text.parse::<DocumentMut>()
            .map_err(|e| format!("{} could not be parsed: {e}", path.display()))?
    } else {
        default_document()
    };
    push_profile(&mut doc, profile);
    save_document(&doc, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("game_cfg_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn default_config_matches_builtin_defaults() {
        let cfg: FileConfig = toml::from_str(DEFAULT_CONFIG).expect("template parses");
        let mut app = App::default();
        let notes = apply(&cfg, &mut app);
        assert!(notes.is_empty(), "unexpected notes: {notes:?}");
        assert_eq!(app, App::default(), "template drifted from App::default");
    }

    #[test]
    fn partial_config_overrides_only_listed_fields() {
        let cfg: FileConfig = toml::from_str("mangohud = false\ngamemode = true").unwrap();
        let mut app = App::default();
        apply(&cfg, &mut app);

        assert!(!app.mangohud);
        assert!(app.gamemode);
        assert!(app.protonhax, "unlisted fields keep their defaults");
    }

    #[test]
    fn permission_errors_get_a_chown_hint() {
        let err = std::io::Error::from_raw_os_error(13);
        let hint = permission_hint(&err, Path::new("/home/u/.config/game-launcher/config.toml"));
        assert!(hint.contains("chown"), "{hint}");
        assert!(hint.contains("/home/u/.config/game-launcher"), "{hint}");

        let missing = std::io::Error::from_raw_os_error(2);
        assert!(permission_hint(&missing, Path::new("/tmp/x")).is_empty());
    }

    #[test]
    fn unknown_keys_are_ignored_and_reported() {
        let dir = scratch("unknown");
        let path = dir.join("config.toml");
        std::fs::write(&path, "mangohud = false\nnewer_setting = 1\n").unwrap();

        let mut app = App::default();
        let notes = load_from(&mut app, &path);

        assert!(!app.mangohud, "known keys still apply");
        assert!(
            notes.iter().any(|n| n.contains("newer_setting")),
            "unknown key is reported: {notes:?}"
        );
        assert!(notes[0].contains("loaded"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_config_without_newer_keys_still_parses() {
        let dir = scratch("older");
        let path = dir.join("config.toml");
        std::fs::write(&path, "mangohud = false\ngamemode = true\n").unwrap();

        let mut app = App::default();
        let notes = load_from(&mut app, &path);

        assert!(!app.mangohud);
        assert!(app.gamemode);
        assert_eq!(app.wayland_monitor, "");
        assert!(!notes.iter().any(|n| n.contains("unknown")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn onlinefix_and_modding_add_their_dll_overrides() {
        let cfg: FileConfig =
            toml::from_str("onlinefix = true\nmodding_support = true\ndll_overrides = [\"x=n,b\"]")
                .unwrap();
        let mut app = App::default();
        apply(&cfg, &mut app);

        let list = &app.winedlloverrides_list;
        assert!(list.iter().any(|d| d == "x=n,b"));
        assert!(list.iter().any(|d| d == "OnlineFix64=n,b"));
        assert!(list.iter().any(|d| d == "dwmapi=n,b"));
    }

    #[test]
    fn exports_become_custom_exports() {
        let cfg: FileConfig = toml::from_str("exports = [\"FOO=1\", \"LD_PRELOAD=\"]").unwrap();
        let mut app = App::default();
        apply(&cfg, &mut app);

        assert_eq!(app.custom_exports.len(), 2);
        assert_eq!(app.custom_exports[0].name, "FOO");
        assert_eq!(app.custom_exports[0].value, "1");
        assert_eq!(app.custom_exports[1].value, crate::config::EMPTY_MARKER);
    }

    #[test]
    fn invalid_logging_level_keeps_default_and_warns() {
        let cfg: FileConfig = toml::from_str("logging_level = 5").unwrap();
        let mut app = App::default();
        let notes = apply(&cfg, &mut app);

        assert_eq!(app.logging_level, 0);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("logging_level"));
    }

    #[test]
    fn wayland_default_is_applied_and_validated() {
        let cfg: FileConfig = toml::from_str("wayland_default = \"x11\"").unwrap();
        let mut app = App::default();
        assert!(apply(&cfg, &mut app).is_empty());
        assert_eq!(app.wayland_default, "x11");

        let cfg: FileConfig = toml::from_str("wayland_default = \"bogus\"").unwrap();
        let mut app = App::default();
        let notes = apply(&cfg, &mut app);
        assert_eq!(app.wayland_default, "auto");
        assert!(notes.iter().any(|n| n.contains("wayland_default")));
    }

    #[test]
    fn load_creates_default_config_when_missing() {
        let dir = scratch("create");
        let path = dir.join("config.toml");

        let mut app = App::default();
        let notes = load_from(&mut app, &path);

        assert!(path.is_file());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
        assert!(notes[0].contains("created default config"));
        // Built-in defaults are unchanged by the creation.
        assert_eq!(app, App::default());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_reads_values_and_leaves_user_file_alone() {
        let dir = scratch("read");
        let path = dir.join("config.toml");
        let text = "# mine\nmangohud = false\ngamemode = true\n";
        std::fs::write(&path, text).unwrap();

        let mut app = App::default();
        let notes = load_from(&mut app, &path);

        assert!(!app.mangohud);
        assert!(app.gamemode);
        assert!(notes[0].contains("loaded"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_reports_parse_errors_and_keeps_defaults() {
        let dir = scratch("bad");
        let path = dir.join("config.toml");
        std::fs::write(&path, "this is not toml").unwrap();

        let mut app = App::default();
        let notes = load_from(&mut app, &path);

        assert_eq!(app, App::default());
        assert!(notes[0].contains("failed to parse"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_document_round_trips_and_keeps_comments() {
        let dir = scratch("save");
        let path = dir.join("sub").join("config.toml");
        let (mut doc, warning) = load_document(&path);
        assert!(warning.is_none());

        doc["mangohud"] = toml_edit::value(false);
        save_document(&doc, &path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# game-launcher configuration."));
        let cfg: FileConfig = toml::from_str(&text).unwrap();
        assert_eq!(cfg.mangohud, Some(false));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn generated_profiles_append_without_overwriting_a_broken_config() {
        let dir = scratch("append_profile");
        let path = dir.join("config.toml");
        let profile = GameProfile {
            name: "Generated".to_string(),
            ..GameProfile::default()
        };

        append_profile_at(&path, &profile).unwrap();
        let (doc, warning) = load_document(&path);
        assert!(warning.is_none());
        assert_eq!(profiles_in(&doc), vec![profile]);

        std::fs::write(&path, "this is not toml").unwrap();
        assert!(append_profile_at(&path, &GameProfile::default()).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "this is not toml",
            "a config that cannot be parsed is never replaced"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn minimal_profile_entries_get_the_struct_defaults() {
        let cfg: FileConfig =
            toml::from_str("[[legion_games]]\nname = \"Minimal\"\napp_id = \"42\"\n").unwrap();
        let mut app = App::default();
        apply(&cfg, &mut app);

        assert_eq!(app.legion_games.len(), 1);
        let profile = &app.legion_games[0];
        assert_eq!(profile.name, "Minimal");
        assert_eq!(profile.app_id, "42");
        assert!(profile.enabled);
        assert_eq!(profile.memory_mib, 12288);
        assert_eq!(profile.startup_ui, "auto");
        assert_eq!(profile.startup_timeout_seconds, 180);
    }

    #[test]
    fn profile_tables_round_trip_and_extend_the_empty_array() {
        let mut doc = default_document();
        let profile = GameProfile {
            name: "Round trip".to_string(),
            app_id: "123".to_string(),
            exe: "/games/Round/game.exe".to_string(),
            enabled: false,
            memory_mib: 16384,
            render_node: "/dev/dri/renderD128".to_string(),
            fullscreen: true,
            startup_ui: "log".to_string(),
            startup_timeout_seconds: 300,
        };
        assert_eq!(push_profile(&mut doc, &profile), 0);

        assert_eq!(profiles_in(&doc), vec![profile]);
        let rendered = doc.to_string();
        assert!(rendered.contains("[[legion_games]]"));
        assert!(
            rendered.contains("# Per-game Legion profiles"),
            "the template comments stay with the setting: {rendered}"
        );

        clear_profiles(&mut doc);
        assert!(profiles_in(&doc).is_empty());
        let rendered = doc.to_string();
        assert!(rendered.contains("legion_games = []"));
        assert!(
            rendered.contains("# Per-game Legion profiles"),
            "removing the last profile keeps the template comments: {rendered}"
        );
    }

    #[test]
    fn inline_profile_arrays_are_read_and_converted_without_losing_entries() {
        let mut doc = "legion_games = [{ name = \"Inline\", app_id = \"7\" }]\n"
            .parse::<DocumentMut>()
            .unwrap();
        let profiles = profiles_in(&doc);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Inline");
        assert_eq!(profiles[0].app_id, "7");

        assert_eq!(push_profile(&mut doc, &GameProfile::default()), 1);

        let rendered = doc.to_string();
        assert!(rendered.contains("[[legion_games]]"), "{rendered}");
        let names: Vec<String> = profiles_in(&doc)
            .into_iter()
            .map(|profile| profile.name)
            .collect();
        assert_eq!(names, vec!["Inline".to_string(), String::new()]);
    }

    #[test]
    fn profile_validation_reports_and_corrects_bad_values() {
        let cfg: FileConfig = toml::from_str(
            r#"
[[legion_games]]
name = "Bad"
app_id = "abc"
memory_mib = 1024
startup_ui = "sometimes"
startup_timeout_seconds = 5
"#,
        )
        .unwrap();
        let mut app = App::default();
        let notes = apply(&cfg, &mut app);

        let profile = &app.legion_games[0];
        assert_eq!(
            profile.app_id, "",
            "a non-numeric App ID matcher is dropped"
        );
        assert_eq!(profile.memory_mib, 12288);
        assert_eq!(profile.startup_ui, "auto");
        assert_eq!(profile.startup_timeout_seconds, 180);
        assert_eq!(notes.len(), 4, "one note per corrected value: {notes:?}");
    }
}
