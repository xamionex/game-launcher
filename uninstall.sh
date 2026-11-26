#!/usr/bin/env bash
# Uninstaller for game-launcher.
#
# Removes the files install.sh puts on disk: game, the LinUwUx loader and the
# other installer-managed files. Legion and Artitux are optional, so the
# uninstaller asks before removing them.
#
# User data is kept by default: ~/.config/game-launcher (config.toml and the payload
# cache), ~/.config/dxvk/dxvk.conf and the logs in ~/logs. Pass --purge to remove it.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/uninstall.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/uninstall.sh | sh -s -- --user
#   curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/uninstall.sh | sh -s -- --purge
#
# Root uninstall (default, preferred):
#   - removes /usr/local/bin/game
#   - asks before removing legion and artitux, including their user copies
#   - also removes the user game copy in ~/.local/bin
#   - re-runs itself with sudo when not already root
#
# User uninstall (--user):
#   - removes ~/.local/bin/game
#   - asks before removing the user legion and artitux installs
#   - a root install is reported and left alone
#
# Without a terminal to ask on, legion and artitux are removed.
#
# --purge also removes:
#   - ~/.config/game-launcher (config.toml and the payload cache)
#   - ~/.config/dxvk/dxvk.conf, unless ~/.config/dxvk is a symlink
#   - ~/.config/SLSsteam/tools/netsock/netsock.so
#   - ~/logs/game and ~/logs/ramdisk.log
#   - ~/.local/state/legion/runs (or $XDG_STATE_HOME/legion/runs)
#
# Games patched with -E keep the proxy dll and the .yes backup in their game folder.
# The uninstaller never modifies game folders.
set -eu

REPO="xamionex/game-launcher"
# Overridable for testing and forks.
RAW_URL="${GAME_LAUNCHER_RAW:-https://raw.githubusercontent.com/$REPO/main}"

info() { printf '%s\n' "==> $*"; }
warn() { printf '%s\n' "!! $*" >&2; }
die()  { printf '%s\n' "!! $*" >&2; exit 1; }

MODE="root"
PURGE=0
for arg in "$@"; do
    case "$arg" in
        --user)
            MODE="user"
            ;;
        --root)
            # Internal mode used by the sudo re-run below.
            MODE="root"
            ;;
        --purge)
            PURGE=1
            ;;
        --help|-h)
            cat <<EOF
Usage: curl -fsSL https://raw.githubusercontent.com/$REPO/main/uninstall.sh | sh
       curl -fsSL https://raw.githubusercontent.com/$REPO/main/uninstall.sh | sh -s -- [--user] [--purge]

Removes game, the LinUwUx loader and the other installer-managed files.
Asks before removing legion and artitux; answer n to keep one.
Without a terminal to ask on, both are removed.

Root uninstall (default, preferred):
  removes /usr/local/bin/game and any user copy in ~/.local/bin.
  asks about legion and artitux. Prompts for sudo when needed.

User uninstall (--user):
  removes ~/.local/bin/game and asks about the user legion and artitux installs.
  A root copy is reported and left alone.

--purge:
  also removes ~/.config/game-launcher (config.toml and payload cache),
  ~/.config/dxvk/dxvk.conf, ~/.config/SLSsteam/tools/netsock/netsock.so,
  the logs in ~/logs, and legion run state.

Without --purge the config and logs are kept for a later reinstall.
Games patched with -E keep the proxy dll and the .yes backup in their game folder.
EOF
            exit 0
            ;;
        *)
            die "unknown argument: $arg (use --user for a user uninstall, --purge to remove user data, --help for usage)"
            ;;
    esac
done

# Same target user rules as install.sh: a root uninstall acts on the invoking user's files, not root's.
if [ "$(id -u)" -eq 0 ]; then
    TARGET_USER="${SUDO_USER:-}"
    [ -n "$TARGET_USER" ] || die "running as root without SUDO_USER; run as a normal user instead"
else
    TARGET_USER="$(id -un)"
fi

TARGET_HOME="$(getent passwd "$TARGET_USER" | cut -d: -f6)"
[ -n "$TARGET_HOME" ] || die "could not resolve home directory for user $TARGET_USER"

# Re-run with sudo for the root uninstall. The script is usually piped from curl,
# where $0 is not a usable path, so fall back to downloading it again.
if [ "$MODE" = "root" ] && [ "$(id -u)" -ne 0 ]; then
    command -v sudo >/dev/null 2>&1 || die "root uninstall requested but sudo is not available; use --user instead"
    REEXEC_ARGS="--root"
    if [ "$PURGE" -eq 1 ]; then
        REEXEC_ARGS="$REEXEC_ARGS --purge"
    fi
    info "root uninstall requested, re-running with sudo"
    # When piped from curl, $0 is the interpreter ("sh" or "bash") and not this
    # script. The marker keeps a same-named file in the current directory from
    # being run with sudo by accident.
    if [ -f "$0" ] && grep -q '^# Uninstaller for game-launcher\.$' "$0" 2>/dev/null; then
        SCRIPT_PATH="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
        # REEXEC_ARGS is a list of flags and has to be split into arguments.
        # shellcheck disable=SC2086
        exec sudo TARGET_USER="$TARGET_USER" TARGET_HOME="$TARGET_HOME" \
            XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-}" XDG_STATE_HOME="${XDG_STATE_HOME:-}" \
            GAME_LAUNCHER_RAW="$RAW_URL" \
            bash "$SCRIPT_PATH" $REEXEC_ARGS
    fi
    command -v curl >/dev/null 2>&1 || die "curl is required to re-run the uninstaller with sudo"
    exec sudo TARGET_USER="$TARGET_USER" TARGET_HOME="$TARGET_HOME" \
        XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-}" XDG_STATE_HOME="${XDG_STATE_HOME:-}" \
        GAME_LAUNCHER_RAW="$RAW_URL" \
        bash -c "curl -fsSL '$RAW_URL/uninstall.sh' | bash -s -- $REEXEC_ARGS"
fi

CONFIG_HOME="${XDG_CONFIG_HOME:-$TARGET_HOME/.config}"
STATE_HOME="${XDG_STATE_HOME:-$TARGET_HOME/.local/state}"
DXVK_DIR="$TARGET_HOME/.config/dxvk"
LOGS_DIR="$TARGET_HOME/logs"

FAILED=0

remove_file() {
    path="$1"
    if [ ! -e "$path" ] && [ ! -L "$path" ]; then
        return 0
    fi
    if rm -f -- "$path"; then
        info "removed $path"
    else
        warn "could not remove $path"
        FAILED=$((FAILED + 1))
    fi
}

remove_tree() {
    path="$1"
    if [ ! -e "$path" ] && [ ! -L "$path" ]; then
        return 0
    fi
    if rm -rf -- "$path"; then
        info "removed $path"
    else
        warn "could not remove $path"
        FAILED=$((FAILED + 1))
    fi
}

# Remove a directory only when the uninstaller emptied it.
prune_dir() {
    dir="$1"
    if [ -d "$dir" ] && [ ! -L "$dir" ]; then
        rmdir -- "$dir" 2>/dev/null || true
    fi
}

remove_bin() {
    name="$1"
    if [ "$MODE" = "root" ]; then
        remove_file "/usr/local/bin/$name"
    fi
    remove_file "$TARGET_HOME/.local/bin/$name"
}

# True when this uninstall will find a binary with this name.
has_bin() {
    name="$1"
    if [ "$MODE" = "root" ] && { [ -e "/usr/local/bin/$name" ] || [ -L "/usr/local/bin/$name" ]; }; then
        return 0
    fi
    [ -e "$TARGET_HOME/.local/bin/$name" ] || [ -L "$TARGET_HOME/.local/bin/$name" ]
}

# Ask whether to remove an optional component.
#
# The script is usually piped from curl, so the answer is read from the terminal instead of stdin.
# Without a terminal the component is removed, matching the uninstaller's purpose.
ask_remove() {
    label="$1"
    if ! (exec 3</dev/tty) 2>/dev/null; then
        info "no terminal to ask on, removing $label"
        return 0
    fi
    printf 'Remove %s? [Y/n] ' "$label" >/dev/tty
    answer=""
    read -r answer </dev/tty || true
    case "$answer" in
        [Nn]*) return 1 ;;
        *) return 0 ;;
    esac
}

remove_bin "game"

if has_bin "legion"; then
    if ask_remove "legion"; then
        remove_bin "legion"
    else
        info "kept legion"
    fi
fi

if has_bin "artitux" \
    || [ -e "$TARGET_HOME/.local/share/artitux/Artitux.so" ] \
    || [ -L "$TARGET_HOME/.local/share/artitux/Artitux.so" ]; then
    if ask_remove "artitux"; then
        remove_bin "artitux"
        remove_file "$TARGET_HOME/.local/share/artitux/Artitux.so"
        prune_dir "$TARGET_HOME/.local/share/artitux"
    else
        info "kept artitux"
    fi
fi

if [ "$MODE" = "user" ]; then
    if [ -e /usr/local/bin/game ] || [ -L /usr/local/bin/game ] \
        || [ -e /usr/local/bin/legion ] || [ -L /usr/local/bin/legion ] \
        || [ -e /usr/local/bin/artitux ] || [ -L /usr/local/bin/artitux ]; then
        warn "a root install is still present in /usr/local/bin; run the uninstaller without --user to remove it"
    fi
fi

# The LinUwUx loader is refreshed by the installer and only used by this launcher,
# so it goes with it. Directories are removed only when nothing else is in them.
remove_file "$TARGET_HOME/.local/share/linuwux/LinUwUx.so"
prune_dir "$TARGET_HOME/.local/share/linuwux"
remove_file "$TARGET_HOME/.local/lib/liblinuwux.so"

if [ "$PURGE" -eq 1 ]; then
    remove_tree "$CONFIG_HOME/game-launcher"

    # dxvk may be a symlink into a repo for a manual install, and removing the
    # file through it would edit the repo. Leave the whole path alone then.
    if [ -L "$DXVK_DIR" ]; then
        warn "leaving $DXVK_DIR/dxvk.conf because $DXVK_DIR is a symlink"
    else
        remove_file "$DXVK_DIR/dxvk.conf"
        prune_dir "$DXVK_DIR"
    fi

    # netsock lives in SLSsteam's directory and may be used without this launcher,
    # so it is only removed on an explicit purge.
    remove_file "$TARGET_HOME/.config/SLSsteam/tools/netsock/netsock.so"
    prune_dir "$TARGET_HOME/.config/SLSsteam/tools/netsock"
    prune_dir "$TARGET_HOME/.config/SLSsteam/tools"
    prune_dir "$TARGET_HOME/.config/SLSsteam"

    remove_tree "$LOGS_DIR/game"
    remove_file "$LOGS_DIR/ramdisk.log"
    prune_dir "$LOGS_DIR"

    remove_tree "$STATE_HOME/legion/runs"
    prune_dir "$STATE_HOME/legion"
fi

if [ "$FAILED" -ne 0 ]; then
    die "some files could not be removed (see warnings above)"
fi

info "done"
if [ "$MODE" = "root" ]; then
    info "checked /usr/local/bin and $TARGET_HOME/.local/bin"
else
    info "checked $TARGET_HOME/.local/bin"
fi
if [ "$PURGE" -eq 1 ]; then
    info "user config, payloads, dxvk config, netsock, logs and legion run state removed"
else
    info "kept $CONFIG_HOME/game-launcher (config.toml and payloads); pass --purge to remove it"
    info "kept $DXVK_DIR/dxvk.conf and $LOGS_DIR; pass --purge to remove them"
fi
info "remove any 'game ... -- %command%' launch options from Steam before launching those games"
