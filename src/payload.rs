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
        app.log(&format!(
            "Payload: failed to create {}: {e}{}",
            dir.display(),
            config_file::permission_hint(&e, dir)
        ));
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

    /// Name and upstream URL of every payload bundled in the binary.
    fn bundled_payloads() -> [(&'static str, &'static str); 3] {
        [
            (crate::eos::DLL_NAME, crate::eos::DLL_URL),
            (crate::wrappers::NETSOCK_NAME, crate::wrappers::NETSNOCK_URL),
            (crate::wrappers::LINUWUX_NAME, crate::wrappers::LINUWUX_URL),
        ]
    }

    /// Result of asking GitHub for the current release asset.
    enum Fetched {
        /// The asset was downloaded.
        Bytes(Vec<u8>),
        /// GitHub could not be reached, so nothing can be said about the payload.
        Unreachable,
        /// GitHub answered but the asset could not be fetched, for example because the URL is wrong.
        Failed(String),
    }

    /// Download `url` with curl, telling an offline machine apart from a broken URL.
    fn fetch_release_asset(url: &str) -> Fetched {
        let dir = std::env::temp_dir().join(format!("game_payload_check_{}", std::process::id()));
        if std::fs::create_dir_all(&dir).is_err() {
            return Fetched::Unreachable;
        }
        let path = dir.join("asset");
        let output = Command::new("curl")
            .args(["-fsSL", "--connect-timeout", "10", "--max-time", "60", "-o"])
            .arg(&path)
            .arg(url)
            .output();
        let result = match output {
            Ok(output) if output.status.success() => match std::fs::read(&path) {
                Ok(bytes) if !bytes.is_empty() => Fetched::Bytes(bytes),
                _ => Fetched::Failed("the download was empty".to_string()),
            },
            // 6, 7 and 28 are curl's "could not resolve host", "could not connect" and "timed out".
            Ok(output) if matches!(output.status.code(), Some(6 | 7 | 28)) => Fetched::Unreachable,
            Ok(output) => Fetched::Failed(format!(
                "curl exited with {:?}: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            // No curl at all: the launcher cannot download payloads here either.
            Err(_) => Fetched::Unreachable,
        };
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    /// Repository a `releases/latest/download/...` asset URL belongs to.
    fn release_repo(url: &str) -> String {
        url.split('/').take(5).collect::<Vec<_>>().join("/")
    }

    /// Newest release tag of the repository `url` points at, for the failure message.
    ///
    /// curl follows the `releases/latest` redirect and reports where it ended up, so no API call or JSON parsing is needed.
    fn upstream_tag(url: &str) -> Option<String> {
        let output = Command::new("curl")
            .args([
                "-fsSL",
                "-o",
                "/dev/null",
                "-w",
                "%{url_effective}",
                "--max-time",
                "20",
            ])
            .arg(format!("{}/releases/latest", release_repo(url)))
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let effective = String::from_utf8_lossy(&output.stdout).trim().to_string();
        effective.rsplit("/tag/").next().map(str::to_string)
    }

    #[test]
    fn release_urls_point_at_their_repositories() {
        assert_eq!(
            release_repo(crate::eos::DLL_URL),
            "https://github.com/yesyes0649/eos-proxy"
        );
        assert_eq!(
            release_repo(crate::wrappers::NETSNOCK_URL),
            "https://github.com/yesyes0649/steamnetsock-patch"
        );
        assert_eq!(
            release_repo(crate::wrappers::LINUWUX_URL),
            "https://github.com/brcly/linuwux-runtime"
        );
    }

    /// The copies bundled in the binary have to be the current upstream release.
    ///
    /// An outdated copy is replaced with the upstream asset and the test fails, so the update is committed and the binary embeds it.
    /// When GitHub or curl is unavailable the payloads it could not check are reported and the test passes, so an offline `cargo test` still works.
    #[test]
    fn bundled_payloads_match_the_upstream_release() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut updated: Vec<String> = Vec::new();
        let mut unreachable: Vec<String> = Vec::new();

        for (name, url) in bundled_payloads() {
            match fetch_release_asset(url) {
                Fetched::Bytes(fresh) => {
                    let path = root.join(name);
                    let old = std::fs::read(&path).ok();
                    if old.as_deref() == Some(fresh.as_slice()) {
                        continue;
                    }
                    // A redirect to an error page would be fetched happily, so refuse to vendor something that is not the same kind of file.
                    if let Some(old) = &old {
                        if old.len() >= 4 && fresh.len() >= 4 && old[..4] != fresh[..4] {
                            panic!(
                                "{name}: the upstream asset is not the same kind of file ({url})"
                            );
                        }
                    }
                    match std::fs::write(&path, &fresh) {
                        Ok(()) => updated.push(match upstream_tag(url) {
                            Some(tag) => format!("{name} -> {tag}"),
                            None => name.to_string(),
                        }),
                        Err(e) => panic!("{name} is outdated and could not be updated: {e}"),
                    }
                }
                Fetched::Unreachable => unreachable.push(name.to_string()),
                Fetched::Failed(message) => panic!("{name}: {message} ({url})"),
            }
        }

        if !unreachable.is_empty() {
            eprintln!(
                "payload check skipped, no network or curl: {}",
                unreachable.join(", ")
            );
        }
        assert!(
            updated.is_empty(),
            "outdated payloads were replaced with the upstream release: {}. Commit them and run the tests again so the binary embeds the new copies.",
            updated.join(", ")
        );
    }
}
