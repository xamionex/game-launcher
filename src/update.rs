//! Self update (the `-U` flag).
//!
//! `-U` reports the version this binary was built from, compares it with the newest published release, and updates the copy the user is running through the installer the README documents.
//! A build that is newer than the latest release (a development build, for instance) is left alone.
//! The install that gets updated is the one that is running when it is a known install path, otherwise the one that is installed, otherwise the README's preferred root install.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// Version this binary was built from.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Repository whose releases are installed.
const REPO: &str = "xamionex/game-launcher";
/// Location of the root install, matching `install.sh`.
const ROOT_BIN: &str = "/usr/local/bin/game";

/// Which install an update applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Install {
    Root,
    User,
}

/// A published release, as far as it matters here.
#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

/// Endpoint listing the newest release, overridable for forks and tests like in `install.sh`.
fn api_url() -> String {
    std::env::var("GAME_LAUNCHER_API")
        .unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"))
}

/// Base URL of the repository's raw files, overridable for forks and tests like in `install.sh`.
fn raw_url() -> String {
    std::env::var("GAME_LAUNCHER_RAW")
        .unwrap_or_else(|_| format!("https://raw.githubusercontent.com/{REPO}/main"))
}

/// Tag of the newest published release.
fn latest_release_tag() -> Result<String, String> {
    let output = Command::new("curl")
        .args(["-fsSL", "--connect-timeout", "10", "--max-time", "60"])
        .arg(api_url())
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not reach {} ({}); check the network and try again",
            api_url(),
            output.status
        ));
    }
    tag_from_release(&String::from_utf8_lossy(&output.stdout))
}

/// Read the tag out of a GitHub release document.
fn tag_from_release(body: &str) -> Result<String, String> {
    let release: Release =
        serde_json::from_str(body).map_err(|e| format!("unexpected release document: {e}"))?;
    if release.tag_name.is_empty() {
        return Err("the release has no tag".to_string());
    }
    Ok(release.tag_name)
}

/// Numeric parts of a version or tag, so `v0.1.10` sorts after `v0.1.9`.
fn version_parts(text: &str) -> Vec<u64> {
    text.trim()
        .trim_start_matches('v')
        .split('.')
        .map_while(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u64>().ok()
        })
        .collect()
}

/// True when `latest` is newer than `current`; `None` when either cannot be read as a version.
fn is_newer(latest: &str, current: &str) -> Option<bool> {
    let latest = version_parts(latest);
    let current = version_parts(current);
    if latest.is_empty() || current.is_empty() {
        return None;
    }
    let len = latest.len().max(current.len());
    let padded = |mut parts: Vec<u64>| {
        parts.resize(len, 0);
        parts
    };
    Some(padded(latest) > padded(current))
}

/// Install an update applies to.
fn install_target(exe: &Path, home: &Path, root_installed: bool, user_installed: bool) -> Install {
    if exe == home.join(".local/bin/game") {
        return Install::User;
    }
    if exe == Path::new(ROOT_BIN) {
        return Install::Root;
    }
    if root_installed {
        return Install::Root;
    }
    if user_installed {
        return Install::User;
    }
    // Nothing installed yet: the README's preferred way is the root install.
    Install::Root
}

/// The shell line that updates `install` through the installer, exactly as the README documents it.
fn installer_command(raw: &str, install: Install) -> String {
    match install {
        Install::User => format!("curl -fsSL {raw}/install.sh | sh -s -- --user"),
        Install::Root => format!("curl -fsSL {raw}/install.sh | sh"),
    }
}

/// Which install this process should update.
fn target() -> Result<Install, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("could not resolve the running binary: {e}"))?;
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let home = PathBuf::from(home);
    let user_bin = home.join(".local/bin/game");
    Ok(install_target(
        &exe,
        &home,
        Path::new(ROOT_BIN).is_file(),
        user_bin.is_file(),
    ))
}

/// `-U`: check the published version, then update the install the user is running.
pub fn run() -> Result<(), String> {
    println!("game {VERSION}");
    let install = target()?;
    let raw = raw_url();

    let latest = latest_release_tag()?;
    match is_newer(&latest, VERSION) {
        Some(true) => {}
        Some(false) => {
            println!("game {VERSION} is already the newest release ({latest}), nothing to update.");
            println!("Reinstall it with: {}", installer_command(&raw, install));
            return Ok(());
        }
        None => {
            println!(
                "Could not compare {VERSION} with the release {latest}; running the installer."
            )
        }
    }

    println!(
        "Updating the {} install to {latest} with the installer.",
        match install {
            Install::Root => "root",
            Install::User => "user",
        }
    );
    let status = Command::new("sh")
        .arg("-c")
        .arg(installer_command(&raw, install))
        .status()
        .map_err(|e| format!("could not run the installer: {e}"))?;
    if !status.success() {
        return Err(format!("the installer exited with {status}"));
    }
    println!("game is now {latest}.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_releases_are_detected() {
        assert_eq!(is_newer("v0.2.0", "0.2.0"), Some(false), "same version");
        assert_eq!(is_newer("v0.2.0", "0.1.9"), Some(true));
        assert_eq!(
            is_newer("v0.1.10", "0.1.9"),
            Some(true),
            "not a string compare"
        );
        assert_eq!(is_newer("0.1.9", "0.1.10"), Some(false));
        assert_eq!(
            is_newer("v1.0", "0.9.9"),
            Some(true),
            "missing parts are zero"
        );
        assert_eq!(is_newer("latest", "0.2.0"), None, "no numbers to compare");
        assert_eq!(is_newer("v0.2.0", "dev"), None);
    }

    #[test]
    fn release_documents_are_parsed() {
        let tag = tag_from_release(r#"{"tag_name": "v0.2.0", "name": "v0.2.0"}"#).unwrap();
        assert_eq!(tag, "v0.2.0");
        assert!(tag_from_release(r#"{"tag_name": ""}"#).is_err());
        assert!(tag_from_release("not json").is_err());
    }

    #[test]
    fn the_running_copy_decides_which_install_is_updated() {
        let home = Path::new("/home/deck");
        let dev = Path::new("/home/deck/Projects/game-launcher/target/release/game");

        // The copy that is running wins over what else is installed.
        assert_eq!(
            install_target(&home.join(".local/bin/game"), home, true, true),
            Install::User
        );
        assert_eq!(
            install_target(Path::new(ROOT_BIN), home, true, true),
            Install::Root
        );

        // A development build updates the install that exists.
        assert_eq!(install_target(dev, home, true, false), Install::Root);
        assert_eq!(install_target(dev, home, false, true), Install::User);

        // With nothing installed it follows the README and installs for root.
        assert_eq!(install_target(dev, home, false, false), Install::Root);
    }

    #[test]
    fn installer_commands_match_the_readme() {
        let raw = "https://raw.githubusercontent.com/xamionex/game-launcher/main";
        assert_eq!(
            installer_command(raw, Install::Root),
            "curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/install.sh | sh"
        );
        assert_eq!(
            installer_command(raw, Install::User),
            "curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/install.sh | sh -s -- --user"
        );
    }
}
