# game-launcher

A fast Steam launch wrapper, written in Rust.

It wraps the game command Steam hands to it (`%command%`), applies optional tools (GameMode, MangoHud, ProtonHax, Gamescope, wezterm),
sets a curated set of Proton/DXVK/VKD3D environment variables, detects the GPU vendor and Wayland, and writes structured per game logs to `~/logs/game`.

Optionally: stages the game into a RAM disk, launches background mods.

The Cargo package is named `game-launcher`; the built binary is named `game` (see `[[bin]]` in `Cargo.toml`).

## Build

Requires a Rust toolchain.

```sh
cargo build --release
```
binary is located at target/release/game

Optionally symlink it onto your local bin (add this to your path):

```sh
ln -s $PWD/target/release/game ~/.local/bin/game
```
symlink for dxvk as well:
```
ln -s $PWD/dxvk ~/.config/dxvk
```
symlink for yesyes fix for slssteam, it's loaded from $HOME/scripts, because I'm lazy and never changed it, oops!
```
mkdir ~/scripts
ln -s $PWD/fix.so ~/scripts/fix.so
```

### Handheld / Steam Deck gaming mode

Gaming mode (the gamescope session) does not source your shell profiles, so `~/.local/bin` may not be on `PATH` and Steam will fail to find the `game` binary.
If a game fails to start and no log appears in `~/logs/game/`, this is why.
Use the absolute path in Launch Options:
```
/home/deck/.local/bin/game -Fo -- %command%
```
or add `~/.local/bin` to the session environment:
```
mkdir -p ~/.config/environment.d
echo 'PATH=$HOME/.local/bin:$PATH' > ~/.config/environment.d/10-local-bin.conf
```

Gaming mode is detected automatically and MangoHud is skipped, since the Steam
overlay already provides a HUD there. Use `-H` to force MangoHud on anyway.

## Usage

```
game [options] [VAR=VALUE ...] -- %command%
```

In Steam, set the game's Launch Options to, for example:

```
game -- %command%
game PROTON_NO_ESYNC=1 -- %command%
game -s -m LD_PRELOAD=fixes.so -- %command%
```

### Environment variables

Any token before `--` shaped like `NAME=value` is exported for the game and
recorded in the log with its before/after value. An empty value unsets the
variable:

```
game LD_PRELOAD= -- %command%      # unsets LD_PRELOAD
game FLAG=1 OTHER=2 -- %command%   # exports both
```

### Options

Enabled by default (use the flag to disable):

| Flag | Effect |
| ---- | ------ |
| `-g` | Disable GameMode |
| `-h` | Disable MangoHud |
| `-H` | Force MangoHud on (even in gaming mode) |
| `-p` | Disable ProtonHax |
| `-W` | Force Wayland (override GPU detection) |
| `-X` | Force disable Wayland |

Disabled by default (use the flag to enable):

| Flag | Effect |
| ---- | ------ |
| `-P` | Enable Pressure Vessel elimination |
| `-L` | Enable SDL3 elimination in the Steam runtime (`STEAM_COMPAT_RUNTIME_SDL3=0`) |
| `-s` | Enable Gamescope (X11 backend) |
| `-S` | Enable Gamescope (Wayland backend) |
| `-w` | Run in wezterm |
| `-o` | Enable OnlineFix DLL overrides |
| `-e` | Kill mod processes on exit |
| `-f` | Enable LSFG-VK |
| `-m` | Enable modding support (adds winhttp override) |
| `-F` | Enable LD_AUDIT with `$HOME/scripts/fix.so` (merges with user-set LD_AUDIT) |
| `-V` | Enable custom vkd3d-proton loading |

Valued flags:

| Flag | Effect |
| ---- | ------ |
| `-l LEVEL` | Logging level: `-1` silent, `0` normal, `1` verbose |
| `-u MOD` | Add a background mod command (repeatable) |
| `-r EXE` | Replace the launched executable |
| `-d DLLS` | Add DLL overrides, semicolon separated (`dinput8=n,b;dxgi=n,b`) |
| `-R` | Stage the game into a RAM disk |
| `-i N` | Number of instances (accepted; currently inert) |

Short flags may be bundled (`-ghk`) and valued flags accept attached or
separate arguments (`-l1` or `-l 1`).

### Missing wrapper tools

Before launch, each enabled wrapper (`gamemoderun`, `mangohud`, `protonhax`, `gamescope`, `wezterm`) is checked for on `PATH`.
If a wrapper is not installed it is skipped rather than causing a launch failure, and a `Wrapper not found, skipping: <name>` line is written to the log.

## Logging

Logs live under `$HOME/logs/game/`.

- One folder per game, named by Steam App ID when available, otherwise by the process/game name.
  For example `~/logs/game/4521640/`.
- Each launch writes a new timestamped log: `"<appid> <name> <YYYYmmdd_HHMMSS>.log"`.
- At most 3 plain `.log` files are kept per folder. On the next launch, older logs are compressed to `<name>.log.tar.gz` and the originals are removed.
  Archives are not deleted automatically.
- On a non-zero exit the active log is renamed to `"... (CRASHED: <code>).log"`.

Logging levels:

- `-l -1` silent: the game runs with inherited stdio and nothing is captured.
- `-l 0` normal (default): output is captured, consecutive duplicate lines are collapsed (`[xN]`), and written to the log.
- `-l 1` verbose: each line is prefixed with an elapsed timestamp, echoed to the terminal, de-duplicated by message, and written to the log.

### Notifications

A desktop notification (via `notify-send`, best effort) is sent when:

- the game exits non-zero (a game crash), or
- the wrapper cannot start the command (a `game` project failure).

## RAM disk (`-R`)

When enabled and the current directory is under a Steam `.../common/...` path,
the game directory is copied into a tmpfs mount, bind mounted in place, and synced back/unmounted on exit. These steps use `sudo` for mount, rsync, and umount.
Without `-R`, setup is skipped.

## Mods (`-u`)

Each `-u "command"` runs in the background via `bash -c`.
With logging enabled, each mod gets its own log file next to the main log.
With `-e`, mod processes are terminated when the wrapper exits.

## Development

```sh
cargo test
cargo clippy --all-targets
```
