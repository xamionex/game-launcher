//! Legion KVM integration.
//!
//! A launch is a Legion launch when `legion = true` (or `-K`) applies and a profile does not disable it.
//! Profiles are matched by Steam App ID first and executable file name second, and a missing profile is generated and appended to the config on the first Legion launch of a game.
//! Legion wraps the Proton command itself; host wrappers stay outside it, which is done in [`crate::wrappers`].

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::config::{
    file_name, App, GameProfile, LEGION_MEMORY_DEFAULT_MIB, LEGION_MEMORY_MAX_MIB,
    LEGION_MEMORY_MIN_MIB, LEGION_TIMEOUT_DEFAULT_SECONDS, LEGION_TIMEOUT_MAX_SECONDS,
    LEGION_TIMEOUT_MIN_SECONDS,
};
use crate::config_file;

/// Result of looking at the newest Legion session after a launch.
#[derive(Debug)]
pub struct SessionSummary {
    pub dir: PathBuf,
    pub phases: Vec<Phase>,
}

/// One entry from Legion's `frames/proton-phases.json`.
#[derive(Debug, Deserialize)]
pub struct Phase {
    pub phase: String,
    pub success: bool,
    #[serde(default)]
    pub elapsed_ms: u64,
}

impl SessionSummary {
    /// Name of the first unsuccessful phase.
    pub fn failed_phase(&self) -> Option<&str> {
        self.phases
            .iter()
            .find(|phase| !phase.success)
            .map(|phase| phase.phase.as_str())
    }

    /// True when every recorded phase succeeded.
    pub fn succeeded(&self) -> bool {
        self.failed_phase().is_none()
    }

    /// Outcome of the game itself, or `None` when the report is incomplete.
    ///
    /// A report that stops after `prefix_setup` proves the game never ran, so the session exit status has to decide.
    pub fn game_outcome(&self) -> Option<bool> {
        if self.phases.is_empty() {
            return None;
        }
        if self.failed_phase().is_some() {
            return Some(false);
        }
        self.phases
            .iter()
            .any(|phase| phase.phase == "workload")
            .then_some(true)
    }
}

/// Resolve the profile for this launch and generate one when Legion is enabled for an unknown game.
pub fn resolve(app: &mut App) -> Result<(), String> {
    resolve_with(app, &mut config_file::add_profile)
}

/// [`resolve`] with an injectable profile writer, used by tests to avoid touching the user config.
fn resolve_with(
    app: &mut App,
    save: &mut dyn FnMut(&GameProfile) -> Result<(), String>,
) -> Result<(), String> {
    let app_id = if app.appid.is_empty() {
        crate::logging::resolve_appid(app)
    } else {
        app.appid.clone()
    };
    let exe = game_exe(app);
    let matched = find_profile(&app.legion_games, &app_id, &exe);

    let enabled = match matched {
        Some(index) if !app.legion_explicit => app.legion_games[index].enabled,
        _ => app.legion,
    };
    app.legion = enabled;
    app.legion_profile = matched;

    if !enabled {
        return Ok(());
    }
    if !app.isproton {
        app.legion = false;
        if app.legion_explicit {
            return Err(
                "-K needs a Proton/Wine game launch; Legion does not wrap native Linux games"
                    .to_string(),
            );
        }
        app.log("Legion skipped: this launch is not a Proton/Wine game");
        return Ok(());
    }

    suppress_conflicts(app);

    if matched.is_none() {
        let profile = generated_profile(app, &app_id, &exe);
        app.log(&format!(
            "Legion: creating profile {:?} for the next launch",
            profile.display_name()
        ));
        if let Err(error) = save(&profile) {
            app.log(&format!(
                "Legion: could not save the generated profile: {error}"
            ));
        }
        app.legion_games.push(profile);
        app.legion_profile = Some(app.legion_games.len() - 1);
    }

    if let Some(profile) = app
        .legion_profile
        .and_then(|index| app.legion_games.get(index))
    {
        app.log(&format!(
            "Legion: using profile {:?} ({} MiB, render node {:?})",
            profile.display_name(),
            profile.memory_mib,
            if profile.render_node.is_empty() {
                "auto"
            } else {
                profile.render_node.as_str()
            }
        ));
    }
    Ok(())
}

/// Match an App ID first, then an executable file name.
fn find_profile(games: &[GameProfile], app_id: &str, exe: &str) -> Option<usize> {
    if !app_id.is_empty() {
        if let Some(index) = games
            .iter()
            .position(|game| !game.app_id.is_empty() && game.app_id == app_id)
        {
            return Some(index);
        }
    }
    if !exe.is_empty() {
        let exe = file_name(exe);
        return games.iter().position(|game| {
            !game.exe.is_empty() && file_name(&game.exe).eq_ignore_ascii_case(exe)
        });
    }
    None
}

/// Game executable file name from the launcher command, replacement or last token.
fn game_exe(app: &App) -> String {
    for arg in &app.original_cmd {
        if arg.to_ascii_lowercase().ends_with(".exe") {
            return file_name(arg).to_string();
        }
    }
    if !app.replacement_exe.trim().is_empty() {
        return file_name(app.replacement_exe.trim()).to_string();
    }
    app.original_cmd
        .last()
        .map(|last| file_name(last).to_string())
        .unwrap_or_default()
}

/// Build a default profile for a launch that has no profile yet.
fn generated_profile(app: &App, app_id: &str, exe: &str) -> GameProfile {
    let exe_stem = Path::new(exe)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(exe);
    let raw_name = if !app.game_name.trim().is_empty() {
        app.game_name.clone()
    } else if !exe_stem.is_empty() {
        exe_stem.to_string()
    } else if !app_id.is_empty() {
        format!("Steam {app_id}")
    } else {
        "Legion game".to_string()
    };
    GameProfile {
        name: clean_name(&raw_name),
        app_id: app_id.to_string(),
        exe: exe.to_string(),
        ..GameProfile::default()
    }
}

/// Keep profile labels on one line and within a reasonable length.
fn clean_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    let mut label: String = cleaned.chars().take(80).collect();
    if label.is_empty() {
        label = "Legion game".to_string();
    }
    label
}

/// Drop flags that do not apply inside the VM.
///
/// Runs before `build_command` so flags that shape the Proton command, such as Pressure Vessel, never change it.
fn suppress_conflicts(app: &mut App) {
    let mut suppressed = Vec::new();
    if app.hypervisor {
        app.hypervisor = false;
        suppressed.push("LinuwUx loader (-v)");
    }
    if app.hypervisor_modules {
        app.hypervisor_modules = false;
        suppressed.push("hypervisor kernel modules (-h)");
    }
    if app.protonhax {
        app.protonhax = false;
        suppressed.push("ProtonHax (-p)");
    }
    if app.fix_audit {
        app.fix_audit = false;
        suppressed.push("netsock LD_AUDIT (-F)");
    }
    if app.lsfg {
        app.lsfg = false;
        suppressed.push("LSFG-VK (-f)");
    }
    if app.disable_sdl3 {
        app.disable_sdl3 = false;
        suppressed.push("SDL3 elimination (-L)");
    }
    if app.pressure_vessel {
        app.pressure_vessel = false;
        suppressed.push("Pressure Vessel (-P)");
    }
    for name in suppressed {
        app.log(&format!(
            "Legion: disabling {name}, it does not apply inside the VM"
        ));
    }
}

/// Validate the resolved Legion binary and prepare the launch environment.
pub fn prepare(app: &mut App) -> Result<(), String> {
    let profile = app
        .legion_profile
        .and_then(|index| app.legion_games.get(index))
        .cloned()
        .unwrap_or_default();
    let binary = resolve_binary(app)?;

    set_legion_environment(app, &profile);
    app.legion_binary = binary.clone();

    run_preflight(&binary, &[OsString::from("doctor")], "legion doctor")?;
    let mut inspect = vec![OsString::from("inspect-launch"), OsString::from("--")];
    inspect.extend(app.cmd.iter().map(OsString::from));
    run_preflight(&binary, &inspect, "legion inspect-launch")?;

    // Snapshot after the read-only checks, so only the session created by the game can be new.
    app.legion_sessions_before = session_dirs();

    app.log(&format!("Legion binary: {}", binary.display()));
    app.log(&format!("Legion profile: {}", profile.display_name()));
    Ok(())
}

/// Export the profile into the environment Legion reads at startup.
fn set_legion_environment(app: &mut App, profile: &GameProfile) {
    let memory = if (LEGION_MEMORY_MIN_MIB..=LEGION_MEMORY_MAX_MIB).contains(&profile.memory_mib) {
        profile.memory_mib
    } else {
        app.log(&format!(
            "Legion: memory_mib {} is outside {LEGION_MEMORY_MIN_MIB}..{LEGION_MEMORY_MAX_MIB}, using {LEGION_MEMORY_DEFAULT_MIB}",
            profile.memory_mib
        ));
        LEGION_MEMORY_DEFAULT_MIB
    };
    let timeout = if (LEGION_TIMEOUT_MIN_SECONDS..=LEGION_TIMEOUT_MAX_SECONDS)
        .contains(&profile.startup_timeout_seconds)
    {
        profile.startup_timeout_seconds
    } else {
        app.log(&format!(
            "Legion: startup_timeout_seconds {} is outside {LEGION_TIMEOUT_MIN_SECONDS}..{LEGION_TIMEOUT_MAX_SECONDS}, using {LEGION_TIMEOUT_DEFAULT_SECONDS}",
            profile.startup_timeout_seconds
        ));
        LEGION_TIMEOUT_DEFAULT_SECONDS
    };
    let startup_ui = if matches!(
        profile.startup_ui.as_str(),
        "auto" | "window" | "log" | "off"
    ) {
        profile.startup_ui.clone()
    } else {
        "auto".to_string()
    };

    std::env::set_var("LEGION_MEMORY_MIB", memory.to_string());
    if !profile.render_node.trim().is_empty() {
        std::env::set_var("LEGION_RENDER_NODE", profile.render_node.trim());
    }
    std::env::set_var(
        "LEGION_FULLSCREEN",
        if profile.fullscreen { "1" } else { "0" },
    );
    std::env::set_var("LEGION_STARTUP_UI", startup_ui);
    std::env::set_var("LEGION_STARTUP_TIMEOUT_SECONDS", timeout.to_string());
}

/// Locate the Legion executable.
fn resolve_binary(app: &App) -> Result<PathBuf, String> {
    let configured = app.legion_path.trim();
    if !configured.is_empty() {
        let expanded = expand_home(configured);
        if is_executable(&expanded) {
            return Ok(expanded);
        }
        if !configured.contains('/') {
            if let Some(path) = which(configured) {
                return Ok(path);
            }
        }
        return Err(format!(
            "legion_path {:?} does not point to an executable. Install Legion or fix legion_path in config.toml.",
            configured
        ));
    }

    if let Some(sibling) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join("legion")))
    {
        if is_executable(&sibling) {
            return Ok(sibling);
        }
    }
    if let Some(path) = which("legion") {
        return Ok(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let path = PathBuf::from(home).join(".local/bin/legion");
        if is_executable(&path) {
            return Ok(path);
        }
    }
    let system = PathBuf::from("/usr/local/bin/legion");
    if is_executable(&system) {
        return Ok(system);
    }
    Err("Legion is enabled but the legion binary was not found. Run install.sh, or set legion_path in config.toml.".to_string())
}

/// Expand a leading `~` so `legion_path = "~/bin/legion"` works.
fn expand_home(path: &str) -> PathBuf {
    let Some(home) = std::env::var_os("HOME") else {
        return PathBuf::from(path);
    };
    let home = PathBuf::from(home);
    if path == "~" {
        home
    } else if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(path)
    }
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

/// Run a read-only Legion command and surface its output when it fails.
fn run_preflight(binary: &Path, args: &[OsString], label: &str) -> Result<(), String> {
    let output = Command::new(binary)
        .args(args)
        .output()
        .map_err(|error| format!("could not run {label}: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(stderr);
    }
    if text.chars().count() > 4000 {
        text = text.chars().take(4000).collect();
        text.push_str("\n...");
    }
    Err(format!("{label} failed ({}):\n{text}", output.status))
}

/// Root of Legion's per-run directories.
fn session_root() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(base.join("legion").join("runs"))
}

/// Session directories that currently exist.
fn session_dirs() -> Vec<PathBuf> {
    let Some(root) = session_root() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect()
}

/// Find the session created by this launch and read its phase report.
pub fn session_summary(app: &App) -> Option<SessionSummary> {
    let mut new: Vec<PathBuf> = session_dirs()
        .into_iter()
        .filter(|path| !app.legion_sessions_before.contains(path))
        .collect();
    new.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    });
    let dir = new.pop()?;
    let phases = std::fs::read(dir.join("vm").join("frames").join("proton-phases.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<Phase>>(&bytes).ok())
        .unwrap_or_default();
    Some(SessionSummary { dir, phases })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str, app_id: &str, exe: &str) -> GameProfile {
        GameProfile {
            name: name.to_string(),
            app_id: app_id.to_string(),
            exe: exe.to_string(),
            ..GameProfile::default()
        }
    }

    fn phase(name: &str, success: bool) -> Phase {
        Phase {
            phase: name.to_string(),
            success,
            elapsed_ms: 1,
        }
    }

    fn proton_app() -> App {
        App {
            original_cmd: vec![
                "/home/u/Proton/proton".to_string(),
                "waitforexitandrun".to_string(),
                "/games/Example/Example.exe".to_string(),
            ],
            isproton: true,
            appid: "123".to_string(),
            game_name: "Example".to_string(),
            ..App::default()
        }
    }

    #[test]
    fn app_id_match_wins_over_executable_match() {
        let games = vec![
            profile("by exe", "", "Example.exe"),
            profile("by app id", "123", "Other.exe"),
        ];
        assert_eq!(find_profile(&games, "123", "Example.exe"), Some(1));
    }

    #[test]
    fn executable_matching_is_case_insensitive_and_ignores_directories() {
        let games = vec![profile("game", "", "/games/Example/EXAMPLE.EXE")];
        assert_eq!(find_profile(&games, "", "example.exe"), Some(0));
    }

    #[test]
    fn profile_enabled_flag_controls_the_launch_without_the_flag() {
        let mut games = vec![profile("off", "123", "Example.exe")];
        games[0].enabled = false;
        let mut app = proton_app();
        app.legion_games = games;
        app.legion = true;
        resolve_with(&mut app, &mut |_| Ok(())).unwrap();
        assert!(!app.legion);
        assert_eq!(app.legion_profile, Some(0));
    }

    #[test]
    fn dash_k_wins_over_a_disabled_profile() {
        let mut games = vec![profile("off", "123", "Example.exe")];
        games[0].enabled = false;
        let mut app = proton_app();
        app.legion_games = games;
        app.legion = true;
        app.legion_explicit = true;
        resolve_with(&mut app, &mut |_| Ok(())).unwrap();
        assert!(app.legion);
    }

    #[test]
    fn first_legion_launch_generates_a_profile_and_saves_it() {
        let mut app = proton_app();
        app.legion = true;
        let mut saved = None;
        resolve_with(&mut app, &mut |game| {
            saved = Some(game.clone());
            Ok(())
        })
        .unwrap();

        let profile = saved.expect("profile was generated");
        assert_eq!(profile.app_id, "123");
        assert_eq!(profile.exe, "Example.exe");
        assert_eq!(profile.name, "Example");
        assert!(profile.enabled);
        assert_eq!(profile.memory_mib, 12288);
        assert_eq!(app.legion_games.len(), 1);
        assert_eq!(app.legion_profile, Some(0));
    }

    #[test]
    fn non_proton_launches_do_not_generate_profiles() {
        let mut app = App {
            original_cmd: vec!["/games/native".to_string()],
            legion: true,
            ..App::default()
        };
        resolve_with(&mut app, &mut |_| panic!("must not save")).unwrap();
        assert!(!app.legion);
        assert!(app.legion_games.is_empty());
    }

    #[test]
    fn explicit_legion_on_a_native_game_is_an_error() {
        let mut app = App {
            original_cmd: vec!["/games/native".to_string()],
            legion: true,
            legion_explicit: true,
            ..App::default()
        };
        let error = resolve_with(&mut app, &mut |_| Ok(())).unwrap_err();
        assert!(error.contains("Proton"), "{error}");
    }

    #[test]
    fn enabling_legion_drops_flags_that_do_not_work_inside_the_vm() {
        let mut app = proton_app();
        app.legion = true;
        app.hypervisor = true;
        app.hypervisor_modules = true;
        app.protonhax = true;
        app.fix_audit = true;
        app.lsfg = true;
        app.disable_sdl3 = true;
        app.pressure_vessel = true;

        resolve_with(&mut app, &mut |_| Ok(())).unwrap();

        assert!(app.legion);
        assert!(!app.hypervisor);
        assert!(!app.hypervisor_modules);
        assert!(!app.protonhax);
        assert!(!app.fix_audit);
        assert!(!app.lsfg);
        assert!(!app.disable_sdl3);
        assert!(!app.pressure_vessel);
    }

    #[test]
    fn a_profile_that_disables_legion_keeps_the_host_flags() {
        let mut games = vec![profile("off", "123", "Example.exe")];
        games[0].enabled = false;
        let mut app = proton_app();
        app.legion_games = games;
        app.legion = true;
        app.hypervisor = true;
        app.pressure_vessel = true;

        resolve_with(&mut app, &mut |_| Ok(())).unwrap();

        assert!(!app.legion);
        assert!(app.hypervisor, "nothing is suppressed for a host launch");
        assert!(app.pressure_vessel);
    }

    #[test]
    fn phase_reports_distinguish_missing_partial_and_complete_sessions() {
        let summary = |phases| SessionSummary {
            dir: PathBuf::from("/tmp/session"),
            phases,
        };

        let empty = summary(Vec::new());
        assert!(empty.succeeded());
        assert_eq!(empty.game_outcome(), None);

        let partial = summary(vec![phase("prefix_setup", true)]);
        assert_eq!(
            partial.game_outcome(),
            None,
            "without the workload phase the game may never have run"
        );

        let failed = summary(vec![phase("prefix_setup", true), phase("workload", false)]);
        assert_eq!(failed.failed_phase(), Some("workload"));
        assert_eq!(failed.game_outcome(), Some(false));

        let complete = summary(vec![phase("prefix_setup", true), phase("workload", true)]);
        assert_eq!(complete.game_outcome(), Some(true));
    }

    #[test]
    fn profile_labels_are_single_line_and_bounded() {
        let long = format!("{}\nsecond", "x".repeat(100));
        let cleaned = clean_name(&long);
        assert!(!cleaned.contains('\n'));
        assert_eq!(cleaned.chars().count(), 80);
    }

    #[test]
    fn an_enabled_profile_turns_legion_on_without_the_global_setting() {
        let mut app = proton_app();
        app.legion_games = vec![profile("on", "123", "Example.exe")];
        app.legion = false;

        resolve_with(&mut app, &mut |_| panic!("the profile exists")).unwrap();

        assert!(app.legion, "the profile's enabled flag decides");
        assert!(!app.legion_explicit);
    }

    #[test]
    fn the_game_executable_prefers_the_proton_command_then_the_replacement() {
        let mut app = proton_app();
        app.replacement_exe = "/elsewhere/Replacement.exe".to_string();
        assert_eq!(game_exe(&app), "Example.exe", "the command line wins");

        app.original_cmd = vec!["/usr/bin/proton".to_string(), "/games/Thing".to_string()];
        assert_eq!(
            game_exe(&app),
            "Replacement.exe",
            "the replacement fills in"
        );

        app.replacement_exe.clear();
        assert_eq!(game_exe(&app), "Thing", "the last token is the fallback");
    }

    #[test]
    fn legion_path_must_point_to_an_executable() {
        let dir = std::env::temp_dir().join(format!("game_legion_bin_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("legion");
        std::fs::write(&binary, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut app = App {
            legion_path: binary.to_string_lossy().into_owned(),
            ..App::default()
        };
        assert_eq!(resolve_binary(&app).unwrap(), binary);

        app.legion_path = dir.join("missing").to_string_lossy().into_owned();
        let error = resolve_binary(&app).unwrap_err();
        assert!(error.contains("legion_path"), "{error}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
