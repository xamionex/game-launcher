# game-launcher

A Steam launch wrapper.

It wraps the game command Steam hands to it (`%command%`), \
Applies tools (GameMode, MangoHud, ProtonHax), \
Sets a curated set of Proton/DXVK/VKD3D environment variables (src/wrappers.rs), \
Detects the GPU vendor and sets Wayland (amd off, nvidia on), \
and writes structured per game logs to `~/logs/game`.

Optionally: launches background mods.

Defaults for the tools come from `~/.config/game-launcher/config.toml`, \
written on first run; command line flags override it. See [Configuration](#configuration).

The Cargo package is named `game-launcher`; the built binary is named `game` (see `[[bin]]` in `Cargo.toml`).

NVIDIA NOTE: If you're using nvidia, this launcher makes your games launch in wayland because performance is usually better in wayland. \
But there are issues with this like: tray icons not going into tray and some apps (launchers) being a white screen. \
To fix this, use this proton-cachyos fork that fixes wayland issues: https://github.com/nanomatters/proton-cachyos/releases/ \
If you're having an issue with your games opening on a different monitor: set the primary monitor with `-M DP-1` (or `wayland_monitor` in the config, `-C` lists the detected monitors). \
This sets `WAYLANDDRV_PRIMARY_MONITOR` and only applies while Wayland is enabled.

## Automatic Install

The preferred way to install is the curl installer. \
It downloads the latest release binary, \
installs the dxvk config to `~/.config/dxvk/dxvk.conf` \
and refreshes the payload cache in `~/.config/game-launcher/payloads`.

Root install (preferred):

```sh
curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/install.sh | sh
```

Installs `game` to `/usr/local/bin/game`. \
This is the recommended way to install, because Steam Deck gaming mode (the gamescope session) does not source your shell profiles, \
so `~/.local/bin` won't be in `PATH` and Steam will fail to find the `game` binary. \
With the root install you can use `game -- %command%` in Launch Options without specifying the entire path.

User install:

```sh
curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/install.sh | sh -s -- --user
```

Installs `game` to `~/.local/bin/game`. Make sure `~/.local/bin` is on your `PATH`. \
Gaming mode may not find it, prefer the root install.

The dxvk config always goes to the invoking user's `~/.config/dxvk/dxvk.conf`, even for root installs.

## Manual Install

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

netsock and the hypervisor loader (liblinuwux.so) are embedded in the binary and self-extract on first use, no setup needed. \
netsock goes to `$HOME/.config/SLSsteam/tools/netsock/` and the hypervisor to `$HOME/.local/lib/`. \
Both are also fetched from their upstream releases into `~/.config/game-launcher/payloads/`, see [Payloads](#payloads).

Hypervisor setup and usage guide: https://cs.rin.ru/forum/viewtopic.php?f=20&t=160056 \
Hypervisor requires mangohud to be disabled sometimes to fully work. (-hv)

### Handheld / Steam Deck gaming mode

Gaming mode (the gamescope session) does not source your shell profiles, so `~/.local/bin` may not be on `PATH` and Steam will fail to find the `game` binary. \
If a game fails to start and no log appears in `~/logs/game/`, this is why. \
Use the absolute path in Launch Options:
```
/home/deck/.local/bin/game -Fo -- %command%
```
or install game to `/usr/local/bin` so that you can still do `game -- %command%` without the full path:
```
sudo ln -s $PWD/target/release/game /usr/local/bin/game
```

I've tried different ways to make it appear in path, this is the only one that worked in my testing, \
let me know if you find a way without root requirement.

Gaming mode is detected automatically and MangoHud is skipped, since the Steam overlay already provides a HUD there. \
Use `-H` to force MangoHud on anyway.

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

Any token before `--` shaped like `NAME=value` is exported for the game and recorded in the log with its before/after value. \
An empty value unsets the variable:

```
game LD_PRELOAD= -- %command%      # unsets LD_PRELOAD
game FLAG=1 OTHER=2 -- %command%   # exports both
```

### Options

Enabled by default (use the flag to disable):

| Flag | Effect |
| ---- | ------ |
| `-g` | Disable GameMode (GameMode is also skipped automatically on BORE kernels and when ananicy-cpp is running, since both conflict with its renicing) |
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
| `-o` | Enable OnlineFix DLL overrides (WINEDLLOVERRIDES="OnlineFix64=n,b;SteamOverlay64=n,b;winmm=n,b;dnet=n,b;steam_api64=n,b") |
| `-e` | Kill mod processes on exit |
| `-f` | Enable LSFG-VK |
| `-m` | Enable modding support (WINEDLLOVERRIDES="dwmapi=n,b;winhttp=n,b;winmm=n,b;version=n,b") |
| `-F` | Enable LD_AUDIT with `$HOME/.config/SLSsteam/tools/netsock/netsock.so` (installed from the payload cache if missing, merges with user-set LD_AUDIT) |
| `-V` | Enable custom vkd3d-proton loading |
| `-v` | Enable hypervisor loader: `LD_PRELOAD=$HOME/.local/lib/liblinuwux.so` and `PROTON_DISABLE_LSTEAMCLIENT=0` (self-extracts if missing) |
| `-E` | Enable EOS-Proxy: replaces the game's `EOSSDK-Win64-Shipping.dll` with the proxy, skipped when the game is already patched |

Valued flags:

| Flag | Effect |
| ---- | ------ |
| `-l LEVEL` | Logging level: `-1` silent, `0` normal, `1` verbose |
| `-u MOD` | Add a background mod command (repeatable) |
| `-r EXE` | Replace the launched executable |
| `-d DLLS` | Add DLL overrides, semicolon separated (`dinput8=n,b;dxgi=n,b`) |
| `-M MONITOR` | Set the primary monitor for the Wine Wayland driver (`WAYLANDDRV_PRIMARY_MONITOR`, e.g. `DP-1`); only applied when Wayland is enabled |
| `-i N` | Number of instances (accepted; currently inert) |

Short flags may be bundled (`-ghk`) and valued flags accept attached or
separate arguments (`-l1` or `-l 1`).

Actions:

| Flag | Effect |
| ---- | ------ |
| `-C` | Open the interactive config editor (`~/.config/game-launcher/config.toml`) and exit, ignoring any game command |
| `-k` | Open the launch options generator, copy the resulting Steam launch options line to the clipboard and exit, ignoring any game command |

### Missing wrapper tools

Before launch, each enabled wrapper (`gamemoderun`, `mangohud`, `protonhax`, `gamescope`, `wezterm`) is checked for on `PATH`. \
If a wrapper is not installed it is skipped rather than causing a launch failure, and a `Wrapper not found, skipping: <name>` line is written to the log.

GameMode is also skipped when the system would fight it: \
with the BORE scheduler active (`kernel.sched_bore = 1`, or a `-bore` kernel) \
or when ananicy-cpp is running, since both renice processes themselves.

## Configuration

Defaults live in `~/.config/game-launcher/config.toml`. \
The file is written with commented defaults the first time the launcher runs and is never overwritten afterwards, so edits survive. \
Values set there are the base for every launch, command line flags override them, and the list settings (`dll_overrides`, `mods`, `exports`) are appended to by their flags.

Use `-C` to edit it in a terminal (TUI):

- up/down (or `j`/`k`) select, enter/space toggles a boolean,
- choice values such as the log level cycle with the left/right arrows or space,
- enter on a value opens inline editing, enter commits and esc cancels,
- enter on a list opens the list editor: enter edits an entry, `a` adds, `d` deletes.
  Entries are one item each, for example `dinput8=n,b` in `dll_overrides`,
  a command like `./mod-loader.sh` in `mods`, and `PROTON_NO_ESYNC=1` in `exports`
  (an empty value, `VAR=`, unsets a variable).
  The editor shows the example for each list.
- `s` saves, `q` quits (with a save/discard prompt when there are unsaved changes), and `ctrl+c` quits at once without saving.

Saving rewrites the file through `toml_edit`, so comments and formatting are kept.
`-C` is an action: it ignores any game command given alongside it.

A config file that cannot be parsed is ignored as a whole, with the reason printed to stderr and written to the launch log; the built-in defaults are used instead. \
Keys the launcher does not know are ignored (and reported in the log), so a config written by a newer version or a typo never takes the whole file down. \
`-C` needs a terminal, so run it from a shell rather than from Steam launch options.

The `wayland_monitor` setting (`-M`) picks the monitor games open on when the Wine Wayland driver is in use. \
The editor only shows the row when Wayland is enabled (GPU detection, or `wayland_force_enable`/`wayland_force_disable`), and only lets you pick from the outputs it detects through `wayland-info`, `wlr-randr`, `hyprctl`, `swaymsg`, `kscreen-doctor` or the DRM connectors. \
The launcher exports it as `WAYLANDDRV_PRIMARY_MONITOR` and, when Wayland is off, ignores it with a log line.

Next to the picker the editor shows `this terminal: <output>`, so the names can be told apart (KWin, Hyprland and Sway can report which output the focused window is on). \
On the monitor row, `i` refreshes that and lists every detected output with its resolution and position, marking the one the editor is running on.

## Launch options generator (`-k`)

`-k` opens the same kind of form as `-C`, but it builds the line you paste into a game's Launch Options in Steam instead of saving anything:

```
game -s -m -u ./mod-loader.sh PROTON_NO_ESYNC=1 -- %command%
```

The rows start from your config, and the config file is only read, so the tool is safe to open at any time. \
Only what you change becomes a flag, which is what per game tweaks need: uncheck MangoHud for one game and the line grows `-h`, enable modding support and it grows `-m`. \
Changing nothing gives a bare `game -- %command%`, and the line is shown in the footer while you edit.

- `enter` or `space` toggles a boolean, cycles a choice, or opens the value and list editors, exactly like `-C`;
- `p` switches between `game` and the absolute path of the running binary, for setups where `PATH` does not have it (a root install makes `game` enough, gaming mode may need the path);
- `s` copies the line without leaving, `q` (or `esc`) copies it and exits, `ctrl+c` exits without copying;
- changes a flag cannot express are listed in the footer: a boolean your config turned off cannot be turned back on by a flag, and a list entry can be added by a flag but not removed, so those cases point at `-C`.

The copy is done with the first of `wl-copy`, `xclip`, `xsel` or `pbcopy` that works. \
The line is always printed to the terminal as well, so a machine without a clipboard tool still shows it.

## Payloads

Two payloads are fetched from their upstream releases and cached in
`~/.config/game-launcher/payloads/`:

| File | Source | Used by |
| ---- | ------ | ------- |
| `EOSSDK-Win64-Shipping.dll` | [eos-proxy](https://github.com/yesyes0649/eos-proxy) releases | `-E` |
| `netsock.so` | [steamnetsock-patch](https://github.com/yesyes0649/steamnetsock-patch) releases (`fix.so`) | `-F` |

Each payload is downloaded once. \
If the download fails (offline, GitHub unreachable, curl missing) the copy embedded in the binary is used instead, and a failed download is not cached, so a later launch can still pick up the latest release. \
Delete the cached file to force a refresh. \
The [installer](#automatic-install) also refreshes both payloads on every install or reinstall.

## EOS-Proxy (`-E`)

Some Steam games use Epic Online Services for multiplayer. `-E` installs the
[eos-proxy](https://github.com/yesyes0649/eos-proxy) dll into the game folder:

- the game's `EOSSDK-Win64-Shipping.dll` is renamed to `EOSSDK-Win64-Shipping.yes` and the proxy dll is written in its place,
- a game that already has the `.yes` backup is skipped, so later launches do nothing,
- a game without `EOSSDK-Win64-Shipping.dll` is skipped: the proxy loads the original dll and cannot work without it,
- only the 64 bit dll is handled,
- the dll comes from the payload cache, see [Payloads](#payloads).

Most games also need `ISteamUser::GetAuthTicketForWebApi` to return a non-error response before EOS networking works, \
which is what SLSsteam with FakeAppIds, `uc-online2` or gbe_fork provide; `-F` is the launcher's helper for the SLSsteam route.

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

## Mods (`-u`)

Each `-u "command"` runs in the background via `bash -c`. \
With logging enabled, each mod gets its own log file next to the main log. \
With `-e`, mod processes are terminated when the wrapper exits.

## Development

```sh
cargo test
cargo clippy --all-targets
```
