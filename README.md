# game-launcher

A Steam launch wrapper.

It wraps the game command Steam hands to it (`%command%`), \
Applies tools (GameMode, MangoHud, ProtonHax), \
Sets a curated set of Proton/DXVK/VKD3D environment variables (src/wrappers.rs), \
Chooses Wayland or X11 from `wayland_default` in the config (by default: NVIDIA uses Wayland, everything else is x11), \
and writes structured per game logs to `~/logs/game`.

Optionally: launches background mods, and runs Proton games inside [Legion KVM](#legion-kvm--k) when `-K` or a per-game profile enables it.

Defaults for the tools come from `~/.config/game-launcher/config.toml`, \
written on first run; command line flags override it. See [Configuration](#configuration).

The Cargo package is named `game-launcher`; the built binary is named `game` (see `[[bin]]` in `Cargo.toml`).

NVIDIA NOTE: If you're using nvidia, this launcher makes your games launch in wayland because performance is usually better in wayland. \
To keep them on X11 instead, set `wayland_default = "x11"` in the config or pass `-X`. \
But there are issues with this like: tray icons not going into tray and some apps (launchers) being a white screen. \
To fix this, use this proton-cachyos fork that fixes wayland issues: https://github.com/nanomatters/proton-cachyos/releases/ \
If you're having an issue with your games opening on a different monitor: set the primary monitor with `-M DP-1` (or `wayland_monitor` in the config, `-C` lists the detected monitors). \
This sets `WAYLANDDRV_PRIMARY_MONITOR` and only applies while Wayland is enabled.

## Automatic Install

The preferred way to install is the curl installer. \
It downloads the latest release binaries, installs `game` and, with `--legion`, `legion` next to each other, \
installs the dxvk config to `~/.config/dxvk/dxvk.conf` \
and refreshes the payload cache in `~/.config/game-launcher/payloads`.
All requested binaries are downloaded and checked for a valid ELF header before anything is installed, so a missing or broken asset leaves the existing install untouched.

Root install (preferred):

```sh
curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/install.sh | sh
```

Installs `game` to `/usr/local/bin`, plus `legion` when `--legion` is given. \
This is the recommended way to install, because Steam Deck gaming mode (the gamescope session) does not source your shell profiles, \
so `~/.local/bin` won't be in `PATH` and Steam will fail to find the `game` binary. \
With the root install you can use `game -- %command%` in Launch Options without specifying the entire path.

User install:

```sh
curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/install.sh | sh -s -- --user
```

Installs `game` to `~/.local/bin`, plus `legion` when `--legion` is given. Make sure `~/.local/bin` is on your `PATH`. \
Gaming mode may not find it, prefer the root install.

Legion is opt-in. To install it alongside `game`, pass `--legion`:

```sh
curl -fsSL https://raw.githubusercontent.com/xamionex/game-launcher/main/install.sh | sh -s -- --legion
```

For a user install, pass both flags: `sh -s -- --user --legion`.

Legion KVM is licensed under AGPL-3.0-only and runs as a separate program, so its license, notice and source location are published with each release. \
The installer prints the pinned source URL, and `legion --version` reports the build. \
The release is built from [github.com/brcly/legion-kvm](https://github.com/brcly/legion-kvm) at commit `0197712f1a12ce70411d575b33306349c7b19c37`.

An installed copy updates itself with `game -U`. \
It reports the version it is running, compares it with the newest release, and then runs this same installer for the copy that is running: \
the root install when it started from `/usr/local/bin`, the user install when it started from `~/.local/bin`, and otherwise whichever of the two is installed (the root install, the README's preferred way, when neither is). \
A build that is newer than the newest release is left alone, so a development build is not downgraded.

`-U` passes `--legion` when `legion` is installed next to `game`, so an opted-in Legion install stays updated while a game-only install stays game-only.

When both `/usr/local/bin/game` and `~/.local/bin/game` exist, the installer reports the other copy and asks whether to remove it, since `PATH` order decides which one runs. \
Answer n to keep it, and make sure the directory of the install you want comes first in `PATH`.

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

`cargo build` only produces `game`. \
For `-K`, install Legion with the [curl installer](#automatic-install) and `--legion`, put a packaged `legion` next to the `game` binary, put it on `PATH`, or set `legion_path` in the config.

netsock and the game loaders (LinUwUx.so and Artitux.so) are embedded in the binary and self-extract on first use, no setup needed. \
netsock goes to `$HOME/.config/SLSsteam/tools/netsock/`, LinUwUx to `$HOME/.local/share/linuwux/` and Artitux to `$HOME/.local/share/artitux/`. \
All of them are also fetched from their upstream releases into `~/.config/game-launcher/payloads/`, see [Payloads](#payloads).

Hypervisor setup and usage guide: https://cs.rin.ru/forum/viewtopic.php?f=20&t=160056 \
Hypervisor requires mangohud to be disabled sometimes to fully work (`-vH`); `-h` handles the kernel module, only needed on AMD Ryzen on AM4 (older than 7000) and the Steam Deck, see [Hypervisor kernel modules](#hypervisor-kernel-modules).

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

Gaming mode is detected automatically and MangoHud is skipped, since the Steam overlay already provides a HUD there; `-Y` forces it back on.

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

| Flag | Effect                                                                                                                                           |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `-g` | Disable GameMode (GameMode is also skipped automatically on BORE kernels and when ananicy-cpp is running, since both conflict with its renicing) |
| `-H` | Disable MangoHud                                                                                                                                 |
| `-Y` | Force MangoHud on even in gaming mode, where it is skipped by default                                                                            |
| `-p` | Disable ProtonHax                                                                                                                                |
| `-W` | Force Wayland (overrides `wayland_default`)                                                                                                      |
| `-X` | Force disable Wayland (overrides `wayland_default`)                                                                                              |

Disabled by default (use the flag to enable):

| Flag | Effect                                                                                                                                                                                   |
| ---- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `-P` | Enable Pressure Vessel elimination                                                                                                                                                       |
| `-L` | Enable SDL3 elimination in the Steam runtime (`STEAM_COMPAT_RUNTIME_SDL3=0`)                                                                                                             |
| `-s` | Enable Gamescope (X11 backend)                                                                                                                                                           |
| `-S` | Enable Gamescope (Wayland backend)                                                                                                                                                       |
| `-w` | Run in wezterm                                                                                                                                                                           |
| `-o` | Enable OnlineFix DLL overrides (WINEDLLOVERRIDES="OnlineFix64=n,b;SteamOverlay64=n,b;winmm=n,b;dnet=n,b;steam_api64=n,b")                                                                |
| `-e` | Kill mod processes on exit                                                                                                                                                               |
| `-f` | Enable LSFG-VK                                                                                                                                                                           |
| `-m` | Enable modding support (WINEDLLOVERRIDES="dwmapi=n,b;winhttp=n,b;winmm=n,b;version=n,b")                                                                                                 |
| `-F` | Enable LD_AUDIT with `$HOME/.config/SLSsteam/tools/netsock/netsock.so` (installed from the payload cache if missing, merges with user-set LD_AUDIT)                                      |
| `-V` | Enable custom vkd3d-proton loading                                                                                                                                                       |
| `-h` | Hypervisor kernel modules: unload KVM (killing any running virtual machines), load `cpuid_fault_emulation` before the game, and restore the previous module state when it exits, as root |
| `-v` | Enable hypervisor loader: `LD_PRELOAD=$HOME/.local/share/linuwux/LinUwUx.so` and `PROTON_DISABLE_LSTEAMCLIENT=0` (self-extracts if missing)                                              |
| `-a` | Enable the Artitux loader for Artifact releases: `LD_PRELOAD=$HOME/.local/share/artitux/Artitux.so` (self-extracts if missing, replaces `-v` when both are enabled)                        |
| `-E` | Enable EOS-Proxy: replaces the game's `EOSSDK-Win64-Shipping.dll` with the proxy, skipped when the game is already patched                                                               |
| `-K` | Run a Proton game inside Legion KVM for this launch, overriding a profile's `enabled = false`; errors on a native Linux game                                                            |

Valued flags:

| Flag         | Effect                                                                                                                                |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------- |
| `-l LEVEL`   | Logging level: `-1` silent, `0` normal, `1` verbose                                                                                   |
| `-u MOD`     | Add a background mod command (repeatable)                                                                                             |
| `-r EXE`     | Replace the launched executable                                                                                                       |
| `-d DLLS`    | Add DLL overrides, semicolon separated (`dinput8=n,b;dxgi=n,b`)                                                                       |
| `-M MONITOR` | Set the primary monitor for the Wine Wayland driver (`WAYLANDDRV_PRIMARY_MONITOR`, e.g. `DP-1`); only applied when Wayland is enabled |
| `-i N`       | Number of instances (accepted; currently inert)                                                                                       |

Short flags may be bundled (`-ghk`) and valued flags accept attached or
separate arguments (`-l1` or `-l 1`).

Actions:

| Flag | Effect                                                                                                                               |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `-C` | Open the interactive config editor (`~/.config/game-launcher/config.toml`) and exit, ignoring any game command                       |
| `-k` | Open the launch options generator, copy the resulting Steam launch options line to the clipboard and exit, ignoring any game command |
| `-U` | Update the installed launcher to the newest release and exit, ignoring any game command                                              |

### Missing wrapper tools

Before launch, each enabled wrapper (`gamemoderun`, `mangohud`, `protonhax`, `gamescope`, `wezterm`) is checked for on `PATH`. \
If a wrapper is not installed it is skipped rather than causing a launch failure, and a `Wrapper not found, skipping: <name>` line is written to the log.

GameMode is also skipped when the system would fight it: \
with the BORE scheduler active (`kernel.sched_bore = 1`, or a `-bore` kernel) \
or when ananicy-cpp is running, since both renice processes themselves.

## Hypervisor kernel modules

`-h` prepares the kernel for the hypervisor before the game and undoes it after. This is only needed on AMD Ryzen on AM4 (older than 7000) and the Steam Deck:

- before the game: unload `kvm_amd`/`kvm`, then load `cpuid_fault_emulation`,
- after the game: unload `cpuid_fault_emulation` and load KVM again when it was running before.
- `cpuid_fault_emulation` and KVM must never be loaded at the same time: the guide is clear that it crashes the system. Every step therefore checks `/sys/module` first, stops at the first failure, and unloads `cpuid_fault_emulation` before KVM is reloaded.
- when both were loaded before the launch, only KVM is restored; the unsafe combination is not recreated.
- a step that fails is rolled back to the state found before the launch and reported with a notification; the game still starts.
- everything runs as root through `modprobe`: when the launcher runs in a terminal, `sudo` asks there; from Steam (no terminal) it uses a cached credential, otherwise the polkit dialog (`pkexec`, the KDE/GNOME authentication window) or a graphical password prompt (`zenity`/`kdialog`/`yad` feeding `sudo -S`). The password is asked once per phase, and polkit/sudo may remember it, so the restore at game exit often does not ask again. Unloading KVM kills any running virtual machines.

`-h` is independent of `-v` and `-a`: `-h` only touches the kernel modules, while the loaders only go into `LD_PRELOAD`, so setups with their own module handling can use `-v` or `-a` alone.

## Artitux (`-a`)

[Artitux](https://github.com/Lumice/artitux) is a fork of LinUwUx for Artifact releases. \
It loads through `LD_PRELOAD` like LinUwUx, but it covers a narrower set of games, so it does not replace `-v` for games that already work with LinUwUx; that is also what Artitux's own README recommends. \
`-a` puts `$HOME/.local/share/artitux/Artitux.so` in `LD_PRELOAD` and self-extracts it when missing, following the same payload cache and installer refresh as LinUwUx. \
Artitux sets `PROTON_DISABLE_LSTEAMCLIENT=1` itself, while `-v` sets it to `0`. \
When both `-a` and `-v` are enabled, `-a` wins because the two loaders interpose the same symbols.

## Legion KVM (`-K`)

Legion runs Windows applications through Proton inside a Linux KVM guest, using Mesa Venus over virtio-GPU. \
`-K` enables it for one launch and overrides a matching profile's `enabled = false`; without `-K`, a matching profile decides and `legion = true` in the config enables it for games that have no profile. \
A native Linux game cannot be wrapped: `-K` stops with an error instead of silently running it on the host, while a profile-driven Legion launch falls back to the host with a log line.

Requirements:

- access to `/dev/kvm` and your GPU render node,
- a Wayland desktop session,
- a host Vulkan driver with the external-memory and synchronization capabilities Venus needs; an AMD kernel driver alone is not enough,
- the `legion` binary, which the [installer](#automatic-install) puts next to `game`.

`LEGION_RENDER_NODE` can select a render node such as `/dev/dri/renderD128`, but the guest Vulkan vendor and device must match it: a missing driver or a conflicting `VK_DRIVER_FILES` fails the launch instead of silently using another GPU. \
Other GPU and driver combinations, and 32-bit games, still need qualification. \
See the [Legion documentation](https://github.com/brcly/legion-kvm) for the current support matrix.

Before the game starts, the launcher runs `legion doctor` and `legion inspect-launch -- <Proton command>` and stops with Legion's own output if either fails. \
The second check sees the unwrapped Proton command, before host wrappers make it opaque.

Legion wraps the Proton command, so the wrapper chain keeps its order:

```
gamemoderun ... wezterm ... gamescope ... legion mangohud <Proton command>
```

Host wrappers stay outside the VM and `mangohud` sits inside Legion's own argument list, where Legion reads it as a request for the guest overlay. \
Flags that do not apply inside the VM are disabled before the command is built, so they cannot leak into Proton: `-v`, `-a`, `-h`, `-p`, `-F`, `-f`, `-L` and `-P`. \
In particular, `-P` is dropped before it can strip the Steam runtime. \
GameMode, Gamescope and wezterm keep working because they run on the host, and `-E` still patches the shared game directory.

Legion reads its settings from the environment: `LEGION_MEMORY_MIB`, `LEGION_RENDER_NODE`, `LEGION_FULLSCREEN`, `LEGION_STARTUP_UI` and `LEGION_STARTUP_TIMEOUT_SECONDS`. \
Legion forwards only its whitelisted host variables into the guest, so unrelated host variables do not cross.

### Legion profiles

Each game gets a profile in `~/.config/game-launcher/config.toml`, stored as `[[legion_games]]`:

```toml
[[legion_games]]
name = "My game"
app_id = "123456"
exe = "game.exe"
enabled = true
memory_mib = 12288
render_node = ""
fullscreen = false
startup_ui = "auto"
startup_timeout_seconds = 180
```

| Key                       | Meaning                                                                                                             |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `name`                    | Label shown in the profile list. Empty falls back to the executable or App ID.                                      |
| `app_id`                  | Steam App ID matcher, matched first. Digits only; empty disables this matcher.                                      |
| `exe`                     | Executable matcher, matched second by file name (case-insensitive). A full path is accepted.                        |
| `enabled`                 | Whether Legion runs this game. `-K` overrides `false` for one launch.                                               |
| `memory_mib`              | Guest RAM, 4096 through 65536. Out-of-range values fall back to 12288 with a log line.                              |
| `render_node`             | DRM render node passed as `LEGION_RENDER_NODE`; empty leaves Legion's automatic selection alone.                    |
| `fullscreen`              | `LEGION_FULLSCREEN=1` when true.                                                                                    |
| `startup_ui`              | `auto`, `window`, `log` or `off`.                                                                                   |
| `startup_timeout_seconds` | Seconds to the first frame, 30 through 900. Out-of-range values fall back to 180 with a log line.                    |

Profiles are matched by App ID first and executable file name second, so the same game works from Steam and from a direct `.exe` launch. \
The first Legion launch of a game without a profile generates one from the game name, App ID and executable, appends it to the config with comments preserved, and uses it immediately. \
That profile is normal user data: `-C` edits or removes it and the launcher never overwrites it.

In `-C`, the `Game profiles` row opens the Legion game list. \
`a` adds a profile and opens its form, `enter` edits, `d` removes, and `s` saves without leaving. \
In the form, up/down selects a field, space or enter toggles a boolean or cycles `startup_ui`, left/right cycles `startup_ui` backwards or forwards, and enter on a text or number value opens inline editing. \
`esc` goes back one level without discarding what is already in the document; the last removal restores `legion_games = []`. \
The list marks profiles that have neither matcher, because those never apply automatically.

A global `legion_path` selects a different executable. \
Empty means next to the running `game` binary, then `PATH`, then `~/.local/bin/legion` and `/usr/local/bin/legion`. \
A configured path may start with `~` or be a bare command name.

## Configuration

Defaults live in `~/.config/game-launcher/config.toml`. \
The file is written with commented defaults the first time the launcher runs and is never overwritten afterwards, so edits survive. \
Values set there are the base for every launch, command line flags override them, and the list settings (`dll_overrides`, `mods`, `exports`) are appended to by their flags.

Use `-C` to edit it in a terminal (TUI):

- up/down (or `j`/`k`) select, enter/space toggles a boolean,
- choice values such as the log level and the Wayland default cycle with the left/right arrows or space,
- enter on a value opens inline editing, enter commits and esc cancels,
- enter on a list opens the list editor: enter edits an entry, `a` adds, `d` deletes.
  Entries are one item each, for example `dinput8=n,b` in `dll_overrides`,
  a command like `./mod-loader.sh` in `mods`, and `PROTON_NO_ESYNC=1` in `exports`
  (an empty value, `VAR=`, unsets a variable).
  The editor shows the example for each list.
- enter on `Game profiles` opens the [Legion profile list](#legion-profiles), with its own add, edit and remove keys.
- `s` saves, `q` quits from the main form (with a save/discard prompt when there are unsaved changes), and `ctrl+c` quits at once without saving.

Saving rewrites the file through `toml_edit`, so comments and formatting are kept.
`-C` is an action: it ignores any game command given alongside it.

A config file that cannot be parsed is ignored as a whole, with the reason printed to stderr and written to the launch log; the built-in defaults are used instead. \
Keys the launcher does not know are ignored (and reported in the log), so a config written by a newer version or a typo never takes the whole file down. \
`-C` needs a terminal, so run it from a shell rather than from Steam launch options.

The `wayland_default` setting picks Wayland or X11 for every launch: `auto` (the default) enables Wayland on NVIDIA and X11 otherwise, while `wayland` and `x11` force that choice. \
The `-W`/`-X` flags and `wayland_force_enable`/`wayland_force_disable` still win over it, and `-C` cycles the three values. \
The setting has no flag of its own.

The `wayland_monitor` setting (`-M`) picks the monitor games open on when the Wine Wayland driver is in use. \
The editor only shows the row when Wayland is enabled (the `wayland_default` choice, or `wayland_force_enable`/`wayland_force_disable`), and only lets you pick from the outputs it detects through `wayland-info`, `wlr-randr`, `hyprctl`, `swaymsg`, `kscreen-doctor` or the DRM connectors. \
The launcher exports it as `WAYLANDDRV_PRIMARY_MONITOR` and, when Wayland is off, ignores it with a log line.

Next to the picker the editor shows `this terminal: <output>`, so the names can be told apart (KWin, Hyprland and Sway can report which output the focused window is on). \
On the monitor row, `i` refreshes that and lists every detected output with its resolution and position, marking the one the editor is running on.

The `legion` setting enables Legion for every Proton launch that has no matching profile; `-K` enables it for one launch. \
`legion_path` selects the Legion executable, and the per-game settings live in `[[legion_games]]`, see [Legion KVM](#legion-kvm--k). \
The defaults file shows the profile schema, and `-C` writes profiles without losing its comments.

## Launch options generator (`-k`)

`-k` opens the same kind of form as `-C`, but it builds the line you paste into a game's Launch Options in Steam instead of saving anything:

```
game -s -m -u ./mod-loader.sh PROTON_NO_ESYNC=1 -- %command%
```

The rows start from your config, and the config file is only read, so the tool is safe to open at any time. \
Only what you change becomes a flag, which is what per game tweaks need: uncheck MangoHud for one game and the line grows `-H`, enable modding support and it grows `-m`, force MangoHud back on in gaming mode and it grows `-Y`. \
Changing nothing gives a bare `game -- %command%`, and the line is shown in the footer while you edit.

- `enter` or `space` toggles a boolean, cycles a choice, or opens the value and list editors, exactly like `-C`;
- `p` switches between `game` and the absolute path of the running binary, for setups where `PATH` does not have it (a root install makes `game` enough, gaming mode may need the path);
- `s` copies the line without leaving, `q` (or `esc`) copies it and exits, `ctrl+c` exits without copying;
- changes a flag cannot express are listed in the footer: a boolean your config turned off cannot be turned back on by a flag, a value with no flag such as `wayland_default` can only be changed there, and a list entry can be added by a flag but not removed, so those cases point at `-C`.

Legion profiles are hidden from `-k`, because no launch flag can create or select one; use `-C`. \
`-K` in a hand written launch line still enables Legion for that one launch.

The copy is done with the first of `wl-copy`, `xclip`, `xsel` or `pbcopy` that works. \
The line is always printed to the terminal as well, so a machine without a clipboard tool still shows it.

## Payloads

Four payloads are fetched from their upstream releases and cached in
`~/.config/game-launcher/payloads/`:

| File                        | Source                                                                                     | Used by |
| --------------------------- | ------------------------------------------------------------------------------------------ | ------- |
| `EOSSDK-Win64-Shipping.dll` | [eos-proxy](https://github.com/yesyes0649/eos-proxy) releases                              | `-E`    |
| `netsock.so`                | [steamnetsock-patch](https://github.com/yesyes0649/steamnetsock-patch) releases (`fix.so`) | `-F`    |
| `LinUwUx.so`                | [linuwux-runtime](https://github.com/brcly/linuwux-runtime) releases (`LinUwUx.so`)        | `-v`    |
| `Artitux.so`                | [artitux](https://github.com/Lumice/artitux) releases (`Artitux.so`)                       | `-a`    |

Each payload is downloaded once. \
If the download fails (offline, GitHub unreachable, curl missing) the copy embedded in the binary is used instead, and a failed download is not cached, so a later launch can still pick up the latest release. \
Delete the cached file to force a refresh. \
The [installer](#automatic-install) also refreshes all four payloads on every install or reinstall.

The copies embedded in the binary are kept current by the test suite.
`cargo test` fetches every upstream release and compares it with the file in the repository: an outdated copy is replaced with the upstream one and the test fails, so the update gets committed and the next build embeds it.
When GitHub or curl cannot be reached the payloads it could not check are reported (run `cargo test -- --nocapture` to see them) and the test passes, so an offline test run still works.

The loaders are only written to their target path (`LinUwUx.so` to `~/.local/share/linuwux`, `Artitux.so` to `~/.local/share/artitux`, `netsock.so` to `~/.config/SLSsteam/tools/netsock`) when that file is missing, so an already extracted loader is left alone.
Delete it to let the next launch extract the cached, up to date copy.

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
- With Legion, the log also records the session directory and the phases from its `proton-phases.json`. \
  A failed phase marks the launch as failed and names the phase in the notification, because Legion's exit status describes the VM session rather than the game.

Logging levels:

- `-l -1` silent: the game runs with inherited stdio and nothing is captured.
- `-l 0` normal (default): output is captured, consecutive duplicate lines are collapsed (`[xN]`), and written to the log.
- `-l 1` verbose: each line is prefixed with an elapsed timestamp, echoed to the terminal, de-duplicated by message, and written to the log.

### Notifications

A desktop notification (via `notify-send`, best effort) is sent when:

- the game exits non-zero (a game crash), or
- the wrapper cannot start the command (a `game` project failure), or
- the hypervisor module setup, rollback or restore (the `-h` flag) fails.

## Mods (`-u`)

Each `-u "command"` runs in the background via `bash -c`. \
With logging enabled, each mod gets its own log file next to the main log. \
With `-e`, mod processes are terminated when the wrapper exits.

## Development

```sh
cargo test
cargo clippy --all-targets
```
