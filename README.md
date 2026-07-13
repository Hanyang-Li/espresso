# espresso ☕

> Keep your Mac awake from the command line — closing the lid only turns off the screen.

**English** | [简体中文](README.zh-CN.md)

![license](https://img.shields.io/badge/license-MIT-blue.svg)
![platform](https://img.shields.io/badge/platform-macOS%20·%20Apple%20Silicon-lightgrey.svg)

`espresso` is a small macOS command-line tool that prevents your Mac from going
to sleep. It works in two layers:

1. **Idle-sleep prevention** (always on, no privileges) — while an `espresso`
   session is active, the Mac won't fall asleep from inactivity. This is like
   `caffeinate`, scoped to the session.
2. **Lid-closed keep-awake** (optional, one-time `sudo` setup) — with a small
   root helper installed, the Mac also stays awake with the lid **closed**
   (screen off, no sleep, even on battery). Without the helper, closing the lid
   still puts the Mac to sleep.

## Features

- Keep awake for a **countdown** or until a **clock time** (`-t`).
- Keep awake **while a command runs**, then exit automatically.
- Optional **lid-closed** keep-awake via an on-demand `launchd` helper.
- First-run setup prompt: if the helper isn't installed, espresso offers to
  install it for you. Decline it — or run without a terminal (scripts, CI) — and
  the session continues with idle-sleep prevention only.
- Single self-contained binary, no runtime dependencies.

## Requirements

- macOS on **Apple Silicon (arm64)**.

## Installation

```sh
curl -fsSL https://raw.githubusercontent.com/Hanyang-Li/espresso/main/install.sh | sh
```

This downloads the latest release, verifies its checksum, and installs the
binary to `/usr/local/bin/espresso`.

Optional overrides:

```sh
# pin a specific version
ESPRESSO_VERSION=v0.2.1 sh -c "$(curl -fsSL https://raw.githubusercontent.com/Hanyang-Li/espresso/main/install.sh)"

# install to a custom directory
ESPRESSO_INSTALL_DIR="$HOME/.local/bin" sh -c "$(curl -fsSL https://raw.githubusercontent.com/Hanyang-Li/espresso/main/install.sh)"
```

> The binary is downloaded via `curl`, so macOS does not quarantine it — no
> Gatekeeper prompt, and no code-signing is required.

## Usage

### Keep awake for a set time

```sh
espresso -t 1800      # 30 minutes (countdown in seconds)
espresso -t 17:00     # until 17:00 today
espresso -t 09:30:00  # until a specific time-of-day
```

A countdown accepts bare seconds, or a **future** clock/date time in any of:
`HH:MM`, `HH:MM:SS`, `MM-DD HH:MM`, `YYYY-MM-DD HH:MM`, `YYYY-MM-DD HH:MM:SS`.

A live progress bar is shown while the timer runs. Press **`q`** to stop early.

### Keep awake while a command runs

```sh
espresso npm run build
espresso -- rsync -a ./src remote:/backup   # use -- when the command has flags
```

The Mac stays awake until the command finishes, and `espresso` exits with the
command's own exit code.

### Lid-closed keep-awake (optional)

The **first time** you start a keep-awake session without the helper installed,
espresso asks whether to set it up and runs `sudo espresso daemon install` for
you if you agree. If you decline — or run in a script with no terminal — the
session continues with idle-sleep prevention only.

You can also manage the helper explicitly:

```sh
sudo espresso daemon install     # one-time setup
espresso daemon status           # inspect current state
sudo espresso daemon uninstall   # remove it
```

The helper is launched on demand by `launchd` and self-exits when no sessions
remain. Once installed, every `espresso -t …` / `espresso <command>` session
automatically gets lid-closed coverage — no extra flags.

## Commands

| Command | Description |
| --- | --- |
| `espresso -t <secs\|time>` | Keep awake for a countdown or until a clock time. |
| `espresso <command> …` | Keep awake while the command runs. |
| `espresso daemon install` | Install the lid-closed helper (requires `sudo`). |
| `espresso daemon uninstall` | Remove the helper (requires `sudo`). |
| `espresso daemon status` | Show helper and keep-awake status. |
| `espresso --version` | Print the version. |
| `espresso --help` | Show help. |

## Notes & caveats

### `SleepDisabled` is global

Lid-closed wake relies on the kernel `SleepDisabled` flag, which is a single
global switch with **no owner**. `espresso` sets it while any session is active
and clears it when the last one ends. This is **last-writer-wins**: it can
override a `SleepDisabled` you set manually via `pmset` (or that another app
set), and vice versa.

### Upgrading while the helper is installed

Re-running the install command overwrites the binary in place via an atomic
same-directory rename, so it's safe even while the daemon is running. The
running daemon keeps serving until it next idles out; the new version takes over
the next time `launchd` starts it. You only need to re-run
`sudo espresso daemon install` if you moved the binary to a different path, or
after `sudo espresso daemon uninstall`.

## Building from source

Requires a recent Rust toolchain (Rust 2024 edition).

```sh
git clone https://github.com/Hanyang-Li/espresso
cd espresso
cargo build --release
# binary at target/release/espresso

# or install straight onto your PATH:
cargo install --path .
```

Run the tests with `cargo test`.

## Uninstalling

```sh
sudo espresso daemon uninstall   # if you installed the helper
sudo rm /usr/local/bin/espresso
```

## License

[MIT](LICENSE) © Hanyang Li
