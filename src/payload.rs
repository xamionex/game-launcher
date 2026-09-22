//! Cache for binaries that also exist as upstream release assets.
//!
//! The cache lives at `~/.config/game-launcher/payloads/<name>`.
//! It is filled from the upstream release the first time a payload is needed, and the copy embedded in the binary is used whenever that download fails.
//! Successful downloads are cached, so the network is only used once per payload; delete the cached file to force a refresh.
//! Failed downloads are not cached.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::App;
use crate::config_file;

/// Cache path for `name`, or `None` when no config directory is available.
pub fn cache_path(name: &str) -> Option<PathBuf> {
    config_file::payloads_dir().map(|dir| dir.join(name))
}

/// Read a payload, downloading and caching it when needed.
///
/// Returns `None` only when the payload is neither cached, downloadable, nor embedded.
pub fn load(app: &App, name: &str, url: &str, bundled: &[u8]) -> Option<Vec<u8>> {
    load_with_cache(app, cache_path(name).as_deref(), name, url, bundled)
}

/// [`load`] with an explicit cache path, used by tests.
fn load_with_cache(
    app: &App,
    cache: Option<&Path>,
    name: &str,
    url: &str,
    bundled: &[u8],
) -> Option<Vec<u8>> {
    if let Some(path) = cache {
        if let Some(data) = read_non_empty(path) {
            app.log(&format!("Payload: using cached {}", path.display()));
            return Some(data);
        }
        if download(app, url, path) {
            if let Some(data) = read_non_empty(path) {
                return Some(data);
            }
        }
    }

    if bundled.is_empty() {
        app.log(&format!("Payload: no cached or bundled copy of {name}"));
        return None;
    }
    app.log(&format!("Payload: using bundled {name}"));
    Some(bundled.to_vec())
}

/// Read `path` when it exists and is not empty.
fn read_non_empty(path: &Path) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(data) if !data.is_empty() => Some(data),
        _ => None,
    }
}

/// Download `url` to `dest` via curl, writing through a `.part` file so an interrupted download never leaves a truncated payload in the cache.
fn download(app: &App, url: &str, dest: &Path) -> bool {
    let Some(dir) = dest.parent() else {
        return false;
    };
    if let Err(e) = std::fs::create_dir_all(dir) {
        app.log(&format!("Payload: failed to create {}: {e}", dir.display()));
        return false;
    }

    let Some(name) = dest.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let tmp = dir.join(format!("{name}.part"));

    app.log(&format!("Payload: downloading {url}"));
    let ok = Command::new("curl")
        .args([
            "-fsSL",
            "--connect-timeout",
            "10",
            "--max-time",
            "300",
            "-o",
        ])
        .arg(&tmp)
        .arg(url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !ok || read_non_empty(&tmp).is_none() {
        app.log(&format!("Payload: download failed for {url}, falling back"));
        let _ = std::fs::remove_file(&tmp);
        return false;
    }

    if let Err(e) = std::fs::rename(&tmp, dest) {
        app.log(&format!("Payload: failed to move download into place: {e}"));
        let _ = std::fs::remove_file(&tmp);
        return false;
    }

    app.log(&format!("Payload: cached {}", dest.display()));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("game_pay_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cached_payload_is_used_without_touching_the_network() {
        let dir = scratch("cached");
        let path = dir.join("thing.bin");
        std::fs::write(&path, b"cached").unwrap();

        let app = App::default();
        let got = load_with_cache(
            &app,
            Some(&path),
            "thing.bin",
            "file:///nonexistent/zzz",
            b"bundled",
        );
        assert_eq!(got, Some(b"cached".to_vec()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundled_payload_is_used_when_download_fails() {
        let dir = scratch("fallback");
        let path = dir.join("thing.bin");

        let app = App::default();
        // A file:// URL fails immediately, so no network is involved.
        let got = load_with_cache(
            &app,
            Some(&path),
            "thing.bin",
            "file:///nonexistent/zzz",
            b"bundled",
        );
        assert_eq!(got, Some(b"bundled".to_vec()));
        assert!(!path.exists(), "failed downloads must not be cached");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_cache_file_is_treated_as_missing() {
        let dir = scratch("empty");
        let path = dir.join("thing.bin");
        std::fs::write(&path, b"").unwrap();

        let app = App::default();
        let got = load_with_cache(
            &app,
            Some(&path),
            "thing.bin",
            "file:///nonexistent/zzz",
            b"bundled",
        );
        assert_eq!(got, Some(b"bundled".to_vec()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_bundled_payload_returns_none() {
        let app = App::default();
        assert_eq!(
            load_with_cache(&app, None, "thing.bin", "unused", b""),
            None
        );
    }
}
