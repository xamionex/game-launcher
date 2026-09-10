#!/usr/bin/env bash
# Installer for game-launcher.
#
# Downloads the latest release binary from GitHub and installs it, plus the
# bundled dxvk config.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/install.sh | sh -s -- --user
#
# Root install (default, preferred):
#   - binary goes to /usr/local/bin/game
#   - works in Steam Deck gaming mode without specifying the full path
#   - re-runs itself with sudo when not already root
#
# User install (--user):
#   - binary goes to ~/.local/bin/game
#   - gaming mode may not find it, see the README
#
# The dxvk config always goes to the invoking user's ~/.config/dxvk/dxvk.conf,
# even for root installs.
set -eu

REPO="xamionex/game-launcher"
BIN_NAME="game"
# Overridable for testing and forks.
API="${GAME_LAUNCHER_API:-https://api.github.com/repos/$REPO/releases/latest}"
RAW_URL="${GAME_LAUNCHER_RAW:-https://raw.githubusercontent.com/$REPO/main}"

info()  { printf '%s\n' "==> $*"; }
warn()  { printf '%s\n' "!! $*" >&2; }
die()   { printf '%s\n' "!! $*" >&2; exit 1; }

# The user the install is for. For root installs this is the invoking user,
# not root, so dxvk and the sudo re-run target the right home directory.
if [ "$(id -u)" -eq 0 ]; then
    TARGET_USER="${SUDO_USER:-}"
    [ -n "$TARGET_USER" ] || die "running as root without SUDO_USER; run as a normal user instead"
else
    TARGET_USER="$(id -un)"
fi

# Resolve the target user's home directory without relying on $HOME, which
# sudo may or may not preserve.
TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
[ -n "$TARGET_HOME" ] || die "could not resolve home directory for user $TARGET_USER"

MODE="root"
if [ "${1:-}" = "--user" ]; then
    MODE="user"
elif [ "${1:-}" = "--root" ]; then
    # Internal mode used by the sudo re-run below.
    MODE="root"
elif [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
    cat <<EOF
Usage: curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | sh [-- --user]

Installs the game-launcher binary and dxvk config.

Root install (default, preferred):
  binary at /usr/local/bin/$BIN_NAME, works in Steam Deck gaming mode
  without specifying the full path. Prompts for sudo when needed.

User install (--user):
  binary at ~/.local/bin/$BIN_NAME.

The dxvk config always goes to ~/.config/dxvk/dxvk.conf for the invoking user.
EOF
    exit 0
elif [ -n "${1:-}" ]; then
    die "unknown argument: $1 (use --user for a user install, --help for usage)"
fi

# Root installs re-run themselves with sudo so the binary can be written to
# /usr/local/bin. The script is re-downloaded inside the sudo shell because
# $0 is not a usable path when the script is piped from curl.
if [ "$MODE" = "root" ] && [ "$(id -u)" -ne 0 ]; then
    if command -v sudo >/dev/null 2>&1; then
        info "root install requested, re-running with sudo"
        exec sudo TARGET_USER="$TARGET_USER" TARGET_HOME="$TARGET_HOME" \
            GAME_LAUNCHER_API="$API" GAME_LAUNCHER_RAW="$RAW_URL" \
            bash -c "curl -fsSL '$RAW_URL/install.sh' | bash -s -- --root"
    fi
    die "root install requested but sudo is not available; use --user instead"
fi

if [ "$MODE" = "root" ]; then
    BIN_DIR="/usr/local/bin"
else
    BIN_DIR="$TARGET_HOME/.local/bin"
fi
DXVK_DIR="$TARGET_HOME/.config/dxvk"

command -v curl >/dev/null 2>&1 || die "curl is required but not installed"
command -v mktemp >/dev/null 2>&1 || die "mktemp is required but not installed"

info "fetching latest release info for $REPO"
RELEASE_JSON="$(curl -fsSL "$API")" || die "failed to reach the GitHub API"

ASSET_URL="$(printf '%s' "$RELEASE_JSON" | sed -n 's/.*"browser_download_url": *"\([^"]*\/game\)".*/\1/p' | head -n 1)"
[ -n "$ASSET_URL" ] || die "no release asset named '$BIN_NAME' found; create a release first (see README)"

TAG="$(printf '%s' "$RELEASE_JSON" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
info "downloading $BIN_NAME ${TAG:+($TAG)}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

curl -fsSL -o "$TMP_DIR/$BIN_NAME" "$ASSET_URL" || die "download failed"

# Sanity check: the asset must be an executable ELF binary.
head -c 4 "$TMP_DIR/$BIN_NAME" | grep -q "$(printf '\177ELF')" || die "downloaded file is not an ELF binary"

chmod 755 "$TMP_DIR/$BIN_NAME"

info "installing binary to $BIN_DIR/$BIN_NAME"
mkdir -p "$BIN_DIR"
install -m 755 "$TMP_DIR/$BIN_NAME" "$BIN_DIR/$BIN_NAME"

info "installing dxvk config to $DXVK_DIR/dxvk.conf"
mkdir -p "$DXVK_DIR"
curl -fsSL -o "$DXVK_DIR/dxvk.conf" "$RAW_URL/dxvk/dxvk.conf" \
    || die "failed to download dxvk.conf"
chown "$TARGET_USER" "$DXVK_DIR/dxvk.conf" 2>/dev/null || true

info "done"
if [ "$MODE" = "root" ]; then
    info "installed for user $TARGET_USER at $BIN_DIR/$BIN_NAME"
    info "Steam Deck gaming mode works without specifying the full path"
else
    info "installed at $BIN_DIR/$BIN_NAME"
    info "make sure $BIN_DIR is on your PATH"
    warn "Steam Deck gaming mode may not find it; prefer the root install"
fi
