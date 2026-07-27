//! Command-line parsing.
//!
//! Ports `parse_flags`/`print_help` from `game.sh`. The legacy `-v VAR=VALUE`
//! flag is replaced by positional `KEY=VALUE` tokens (e.g.
//! `game FLAG=1 -- %command%`); both feed the same custom-export pipeline that
//! is logged and applied just before launch.

use crate::config::{App, CustomExport, EMPTY_MARKER, MODDING_DLLS, ONLINEFIX_DLLS};

/// Why parsing failed, mirroring the original's two error paths.
#[derive(Debug)]
pub enum ParseError {
    /// Print full usage/help and exit (unknown flag, missing value, no args).
    Usage,
    /// Print a specific message and exit (e.g. an invalid `-l` value).
    Invalid(String),
}

/// Short flags that take a value argument.
fn takes_value(c: char) -> bool {
    matches!(c, 'i' | 'l' | 'u' | 'r' | 'd')
}

/// True when `token` looks like an environment assignment `NAME=...`.
fn is_assignment(token: &str) -> bool {
    let mut chars = token.char_indices();
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    for (i, c) in chars {
        if c == '=' {
            return i > 0;
        }
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
    }
    false
}

/// Build a [`CustomExport`] from a `NAME=VALUE` string, capturing the current
/// value of the variable for before/after logging.
fn make_export(assignment: &str) -> CustomExport {
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

/// Apply a single short flag character to `app`. Returns `Err` for unknown flags.
fn apply_bool_flag(app: &mut App, c: char) -> Result<(), ParseError> {
    match c {
        'g' => app.gamemode = false,
        'h' => app.mangohud = false,
        'p' => app.protonhax = false,
        'P' => app.pressure_vessel = true,
        's' => app.gamescope = true,
        'S' => app.gamescope_wayland = true,
        'w' => app.wezterm = true,
        'W' => app.wayland_force_enable = true,
        'X' => app.wayland_force_disable = true,
        'V' => app.enable_custom_vkd3d = true,
        'o' => {
            app.onlinefix = true;
            app.winedlloverrides_list
                .extend(ONLINEFIX_DLLS.iter().map(|s| s.to_string()));
        }
        'e' => app.cleanup_mods_on_exit = true,
        'f' => app.lsfg = true,
        'm' => {
            app.modding_support = true;
            app.winedlloverrides_list
                .extend(MODDING_DLLS.iter().map(|s| s.to_string()));
        }
        'L' => app.disable_sdl3 = true,
        'R' => app.use_ramdisk = true,
        'F' => app.fix_audit = true,
        _ => return Err(ParseError::Usage),
    }
    Ok(())
}

/// Apply a valued short flag. Returns `Err` on invalid values.
fn apply_value_flag(app: &mut App, c: char, value: String) -> Result<(), ParseError> {
    match c {
        // The original does not validate -i; keep it lenient and inert.
        'i' => app.instances = value.parse().unwrap_or(app.instances),
        'l' => {
            let invalid = || {
                ParseError::Invalid(format!(
                    "Invalid log level for -l: {value}. Must be -1, 0, or 1."
                ))
            };
            let level: i32 = value.parse().map_err(|_| invalid())?;
            if !(-1..=1).contains(&level) {
                return Err(invalid());
            }
            app.logging_level = level;
        }
        'u' => app.mods_to_launch.push(value),
        'r' => app.replacement_exe = value,
        'd' => {
            app.winedlloverrides_list
                .extend(value.split(';').map(|s| s.to_string()));
        }
        _ => return Err(ParseError::Usage),
    }
    Ok(())
}

/// Parse the argument vector (excluding the program name) into `app`.
///
/// Returns `Err(message)` when usage is invalid; callers should print help and
/// exit non-zero. Everything after the first `--` becomes the original command.
pub fn parse_flags(app: &mut App, args: &[String]) -> Result<(), ParseError> {
    if args.is_empty() {
        return Err(ParseError::Usage);
    }

    // Split at the first `--`; the remainder is the verbatim command.
    let dd = args.iter().position(|a| a == "--");
    let (before, after): (&[String], Option<&[String]>) = match dd {
        Some(i) => (&args[..i], Some(&args[i + 1..])),
        None => (args, None),
    };

    let mut i = 0;
    while i < before.len() {
        let token = &before[i];

        if token == "-" {
            // Lone dash: not a flag. In `--` mode it is ignored; without `--`
            // it begins the command.
            if after.is_some() {
                i += 1;
                continue;
            }
            break;
        }

        if let Some(rest) = token.strip_prefix('-') {
            // Short-flag cluster, e.g. `-gh`, `-l1`, or `-l` + next token.
            let chars: Vec<char> = rest.chars().collect();
            let mut ci = 0;
            while ci < chars.len() {
                let c = chars[ci];
                if takes_value(c) {
                    let attached: String = chars[ci + 1..].iter().collect();
                    let value = if !attached.is_empty() {
                        attached
                    } else {
                        i += 1;
                        before.get(i).cloned().ok_or(ParseError::Usage)?
                    };
                    apply_value_flag(app, c, value)?;
                    break;
                } else {
                    apply_bool_flag(app, c)?;
                    ci += 1;
                }
            }
            i += 1;
        } else if is_assignment(token) {
            app.custom_exports.push(make_export(token));
            i += 1;
        } else {
            // A bare token. Without `--`, it starts the command; with `--` the
            // command comes from after the separator, so stray tokens are
            // ignored (matching getopts stopping at the first non-option).
            if after.is_none() {
                app.original_cmd = before[i..].to_vec();
            }
            break;
        }
    }

    if let Some(rest) = after {
        app.original_cmd = rest.to_vec();
    }

    Ok(())
}

/// Print usage to stderr and best-effort desktop notification.
pub fn print_help(prog: &str) {
    crate::config::notify("Invalid Usage, Check your flags", "");

    let onlinefix = ONLINEFIX_DLLS.join(";");
    eprintln!("Usage: {prog} [options] [VAR=VALUE ...] -- %command%");
    eprintln!();
    eprintln!("== Enabled by default (can be disabled) ==");
    eprintln!("  -g            Disable GameMode");
    eprintln!("  -h            Disable MangoHud");
    eprintln!("  -p            Disable ProtonHax");
    eprintln!("  -W            Force Wayland (overrides GPU detection)");
    eprintln!("  -X            Force disable Wayland");
    eprintln!();
    eprintln!("== Disabled by default (can be enabled) ==");
    eprintln!("  -P            Enable Pressure Vessel elimination");
    eprintln!("  -L            Enable SDL3 elimination in Steam runtime (sets STEAM_COMPAT_RUNTIME_SDL3=0)");
    eprintln!("  -s            Enable Gamescope (X11 backend)");
    eprintln!("  -S            Enable Gamescope (Wayland backend)");
    eprintln!("  -w            Run in wezterm");
    eprintln!("  -o            Enable OnlineFix (WINEDLLOVERRIDES={onlinefix})");
    eprintln!("  -e            Cleanup mods on exit");
    eprintln!("  -f            Enable LSFG-VK");
    eprintln!("  -m            Enable modding support (adds winhttp override)");
    eprintln!(
        "  -F            Enable LD_AUDIT with $HOME/scripts/fix.so (merges with user-set LD_AUDIT)"
    );
    eprintln!("  -V            Enable custom vkd3d-proton loading (~/Projects/vkd3d-proton/build/vkd3d-proton-master)");
    eprintln!();
    eprintln!("== Flags that accept values or lists ==");
    eprintln!("  -l LEVEL      Set logging level (-1: silent, 0: normal, 1: verbose)");
    eprintln!("  -u MOD        Add a mod to be launched (can be used multiple times)");
    eprintln!("  -r EXE        Replace the default executable");
    eprintln!("  -d DLLS       Add DLL overrides (semicolon-separated, e.g. dinput8=n,b;dxgi=n,b)");
    eprintln!("  -R            Load the game into a RAM disk");
    eprintln!("  -i N          Number of instances");
    eprintln!();
    eprintln!("== Environment variables ==");
    eprintln!("  VAR=VALUE     Export a custom environment variable (e.g. LD_PRELOAD=fixes.so)");
    eprintln!("                Use VAR= (empty value) to unset a variable.");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  {prog} -s -m -d 'dinput8=n,b;dxgi=n,b' LD_PRELOAD=fixes.so -- %command%");
    eprintln!("  {prog} -p -e -u someMod -u anotherMod -- %command%");
    eprintln!("  {prog} PROTON_NO_ESYNC=1 -- %command%");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn assignment_detection() {
        assert!(is_assignment("FOO=1"));
        assert!(is_assignment("LD_PRELOAD=a.so"));
        assert!(is_assignment("EMPTY="));
        assert!(!is_assignment("=novar"));
        assert!(!is_assignment("-flag"));
        assert!(!is_assignment("/path/to/game"));
        assert!(!is_assignment("1BAD=x"));
    }

    #[test]
    fn positional_assignment_and_command_split() {
        let mut app = App::default();
        parse_flags(
            &mut app,
            &v(&["FLAG=1", "LD_PRELOAD=x.so", "--", "/bin/game", "--arg"]),
        )
        .unwrap();

        assert_eq!(app.custom_exports.len(), 2);
        assert_eq!(app.custom_exports[0].name, "FLAG");
        assert_eq!(app.custom_exports[0].value, "1");
        assert_eq!(app.original_cmd, v(&["/bin/game", "--arg"]));
    }

    #[test]
    fn empty_value_becomes_marker() {
        let mut app = App::default();
        parse_flags(&mut app, &v(&["LD_PRELOAD=", "--", "x"])).unwrap();
        assert_eq!(app.custom_exports[0].value, EMPTY_MARKER);
    }

    #[test]
    fn bundled_boolean_flags() {
        let mut app = App::default();
        parse_flags(&mut app, &v(&["-gh", "--", "x"])).unwrap();
        assert!(!app.gamemode);
        assert!(!app.mangohud);
    }

    #[test]
    fn log_level_attached_and_separate() {
        let mut app = App::default();
        parse_flags(&mut app, &v(&["-l1", "--", "x"])).unwrap();
        assert_eq!(app.logging_level, 1);

        let mut app2 = App::default();
        parse_flags(&mut app2, &v(&["-l", "-1", "--", "x"])).unwrap();
        assert_eq!(app2.logging_level, -1);
    }

    #[test]
    fn invalid_log_level_is_reported() {
        let mut app = App::default();
        match parse_flags(&mut app, &v(&["-l", "5", "--", "x"])) {
            Err(ParseError::Invalid(_)) => {}
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn unknown_flag_and_no_args_request_usage() {
        let mut app = App::default();
        assert!(matches!(
            parse_flags(&mut app, &v(&["-z", "--", "x"])),
            Err(ParseError::Usage)
        ));
        let mut app2 = App::default();
        assert!(matches!(
            parse_flags(&mut app2, &[]),
            Err(ParseError::Usage)
        ));
    }

    #[test]
    fn dll_overrides_split_on_semicolons() {
        let mut app = App::default();
        parse_flags(&mut app, &v(&["-d", "dinput8=n,b;dxgi=n,b", "--", "x"])).unwrap();
        assert_eq!(app.winedlloverrides_list, v(&["dinput8=n,b", "dxgi=n,b"]));
    }

    #[test]
    fn command_without_double_dash() {
        let mut app = App::default();
        parse_flags(&mut app, &v(&["-g", "FOO=1", "/bin/game", "arg"])).unwrap();
        assert!(!app.gamemode);
        assert_eq!(app.custom_exports.len(), 1);
        assert_eq!(app.original_cmd, v(&["/bin/game", "arg"]));
    }

    #[test]
    fn repeated_mods_collected() {
        let mut app = App::default();
        parse_flags(&mut app, &v(&["-u", "modA", "-u", "modB", "--", "x"])).unwrap();
        assert_eq!(app.mods_to_launch, v(&["modA", "modB"]));
    }

    #[test]
    fn fix_audit_flag_enables_ld_audit_merge() {
        let mut app = App::default();
        assert!(!app.fix_audit);
        parse_flags(&mut app, &v(&["-F", "--", "x"])).unwrap();
        assert!(app.fix_audit);
    }

    #[test]
    fn pressure_vessel_disabled_by_default_enabled_by_flag() {
        let mut app = App::default();
        assert!(!app.pressure_vessel);
        parse_flags(&mut app, &v(&["-P", "--", "x"])).unwrap();
        assert!(app.pressure_vessel);
    }

    #[test]
    fn sdl3_disabled_by_default_enabled_by_flag() {
        let mut app = App::default();
        assert!(!app.disable_sdl3);
        parse_flags(&mut app, &v(&["-L", "--", "x"])).unwrap();
        assert!(app.disable_sdl3);
    }

    #[test]
    fn speedhack_flag_removed() {
        // `-k` used to disable speedhack; the layer is removed entirely now,
        // so the flag is unknown and requests usage.
        let mut app = App::default();
        assert!(matches!(
            parse_flags(&mut app, &v(&["-k", "--", "x"])),
            Err(ParseError::Usage)
        ));
    }
}
