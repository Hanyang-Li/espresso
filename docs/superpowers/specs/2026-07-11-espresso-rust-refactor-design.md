# Espresso Rust Refactor Design

Date: 2026-07-11
Target platform: macOS 26.5+ on Apple Silicon (arm64), Rust 2024.

## Goal

Refactor the `espresso` CLI so that, while active, the Mac never idle-sleeps on
either battery or AC power, and closing the lid only turns off the screen without
sleeping — until the espresso session ends. Replace the current "shell out to
`caffeinate -dimsu -t`" implementation with direct IOKit control plus a small
privileged coordinator daemon.

## Why a daemon is required

- Idle-sleep prevention is done with a per-process IOKit power assertion
  (`kIOPMAssertionTypePreventUserIdleSystemSleep`). This needs no root and is
  released automatically by the kernel when the holding process dies.
- Lid-closed ("clamshell") sleep on battery cannot be blocked by any public power
  assertion. The only mechanism is the kernel `SleepDisabled` flag, set via
  `IOPMSetSystemPowerSetting("SleepDisabled", …)`, which requires root.
- `SleepDisabled` is a single global boolean with no owner and no reference count.
  Multiple espresso instances must coordinate so that one instance exiting does
  not clear the flag while another still needs it. A single privileged daemon that
  reference-counts client connections provides that coordination and confines root
  to one place.

## Behavior

### Argument modes (dispatch precedence)

| Command line | Mode |
|---|---|
| `espresso daemon <install\|uninstall\|status>` | daemon management subcommand |
| `espresso __daemon` | hidden runtime mode, launched only by launchd |
| positional command present (`espresso npm run build`, or `espresso -t 3600 npm run build`) | **command mode**: `-t` is ignored if also present (no error); no progress bar |
| `-t <value>` only | **timer mode**: countdown or target time; progress bar shown |
| no arguments | error + usage (exit code 2) |

### `-t <value>` classification

1. Positive integer (`> 0`) → countdown in seconds.
2. Otherwise, if it matches an existing time format
   (`HH:mm`, `HH:mm:ss`, `MM-dd HH:mm`, `MM-dd HH:mm:ss`, `yyyy-MM-dd HH:mm`,
   `yyyy-MM-dd HH:mm:ss`) → target time; must be later than now.
3. Otherwise (including `0`, negative, non-numeric junk, or a past target) → error.

### Session behavior (all active modes)

While a session is active, the foreground CLI:

- holds one `PreventUserIdleSystemSleep` assertion (covers idle sleep on battery and AC);
- opens a `Hold` connection to the daemon for the whole session (this is the daemon's
  reference-count token, and the trigger for `SleepDisabled`);
- timer mode: enables terminal raw mode and renders the single-line progress bar,
  exiting on timer completion, `q`, or Ctrl-C;
- command mode: spawns the child with inherited stdio, waits for it, and propagates
  its exit code; no raw mode, no progress bar.

On every exit path — normal, `q`, signal, or crash — the assertion is released
(explicitly, or by the kernel on crash) and the daemon connection is closed (the
daemon decrements its reference count on EOF).

### Daemon not installed

When the CLI cannot reach the socket, it prompts (on first use) to install the
daemon via `espresso daemon install` (which requires sudo). If the user declines,
the session degrades gracefully: idle-sleep prevention still works via the
assertion, and a warning states that lid-close will still sleep because the daemon
is not installed. (This is distinct from the no-arguments case, which always errors.)

## Architecture

Single binary with three runtime forms plus a shared library crate.

```
espresso (single binary)
├── CLI foreground : espresso [-t ...] / espresso <cmd...> / espresso daemon <sub>
├── daemon form    : espresso __daemon   (socket-activated by launchd, runs as root)
└── library (lib)  : reusable, unit-testable logic
```

### Modules

| Module | Responsibility | Depends on |
|---|---|---|
| `cli.rs` | Argument parsing and mode dispatch; `-t` classification; `-t`-vs-command precedence. Uses clap (`trailing_var_arg` + `daemon` subcommand + `-t` option). | time |
| `time.rs` (exists) | Target-time parsing (reused); **new**: positive-integer countdown-seconds parsing. | chrono |
| `progress.rs` (exists) | Progress bar and remaining-time formatting (reused). | — |
| `session.rs` | Foreground session lifecycle: connect to daemon, hold assertion, run the selected mode, clean up on exit. | ipc, assertion |
| `assertion.rs` | IOKit FFI for `IOPMAssertionCreateWithName` / `IOPMAssertionRelease` (`PreventUserIdleSystemSleep`). | IOKit |
| `ipc.rs` | Unix-socket wire protocol; frame definitions shared by client and server; connection intents (`Hold` / `Query`). | std |
| `daemon.rs` | Daemon main loop: reference count, `SleepDisabled` control, lid watch + screen-off, startup reset, idle self-exit. | ipc, lid, power |
| `power.rs` | IOKit FFI for `IOPMSetSystemPowerSetting("SleepDisabled", …)` (root). | IOKit |
| `lid.rs` | Read/subscribe `AppleClamshellState` from the IORegistry (IOKit FFI). | IOKit |
| `install.rs` | Write/load/unload the LaunchDaemon plist; `daemon install/uninstall/status`; first-use auto-prompt. | std |

Boundaries: `assertion`, `power`, and `lid` are thin FFI wrappers, each with a
single small unsafe surface. `ipc` defines the protocol and is shared by both ends.
`daemon` (server) and `session` (client) do not reach into each other's internals;
they communicate only through `ipc` frames. Pure logic (parsing, progress, protocol
codec, the reference-count state machine) is unit-testable; FFI and launchd side
effects are confined to a few modules.

## Component split rationale

The foreground CLI holds the idle-sleep assertion (no root, kernel auto-releases on
crash); the daemon owns only `SleepDisabled`. Putting the assertion in the CLI keeps
the root surface minimal and enables the graceful-degradation story. The daemon owns
exactly one global flag — there is no "daemon's own assertion" to track.

## IPC and reference counting

Chosen mechanism: **unix domain socket + launchd `Sockets` activation**.

Rationale (versus XPC/MachServices): crash-safe reference counting, on-demand launch,
and socket-file management are equivalent between the two. XPC additionally offers
free idle-exit (`EnableTransactions`), typed message serialization, and stronger
caller authentication (`audit_token`/entitlements). Against that, libxpc in Rust
requires Objective-C block-based FFI (`block2`/`dispatch2`), a larger unsafe surface,
and less mature crates, and is not unit-testable. Because the daemon's actions
(setting `SleepDisabled`, running `pmset displaysleepnow`) are not per-caller
dangerous, the weaker socket-based authentication (`getpeereid()` if needed) is
acceptable, and the smaller/testable Rust surface wins.

### Connection intents

- `Hold` — a session token. Increments the reference count and drives `SleepDisabled`.
  Held open for the session's lifetime; close/EOF (including crash) releases it.
- `Query` — read-only status. Does **not** increment the reference count and does
  **not** touch `SleepDisabled`. Used by `daemon status`.

## Daemon lifecycle (state machine)

launchd holds the listening socket and socket-activates the daemon on the first
connection. A single-threaded event loop serializes connection accounting and the
exit decision, which removes the accept-versus-exit race.

```
startup           : SleepDisabled = 0 (crash recovery), refcount = 0, then accept
refcount 0 -> 1   : IOPMSetSystemPowerSetting("SleepDisabled", 1); start lid watch
refcount 1 -> 0   : SleepDisabled = 0; stop lid watch; start 60s idle-grace timer
  new Hold in grace: cancel timer; back to the 0 -> 1 path
  grace elapses at 0: exit(0)  → launchd relaunches on the next connection
lid watch (refcount > 0): AppleClamshellState rising edge to Yes → `pmset displaysleepnow`
```

The socket-activation fd is retrieved from launchd via `launch_activate_socket("Listener")`.
Defaults: idle grace 60s, socket path `/var/run/espresso.sock` (mode 0666 so
non-root clients can connect).

### Async correctness notes

- The reference-count check and the exit decision are serialized in one event loop;
  there is no time-of-check/time-of-use race with incoming connections.
- If the daemon exits at the exact moment a client connects, launchd relaunches a
  fresh daemon; the client's connection lands there, and the new daemon sets
  `SleepDisabled = 1` on the 0 → 1 transition. The sub-second window where the flag
  may read 0 is harmless (idle sleep does not occur within a second; clamshell sleep
  matters only if the lid is closed at that instant, and the next tick corrects it).
- IOKit assertions are reference-counted per holding process by the kernel and are
  released automatically when that process dies, including `SIGKILL`. Explicit
  release on graceful shutdown is hygiene, not a correctness dependency; scanning for
  "leftover" assertions is unnecessary.
- `SleepDisabled` has no owner. espresso's semantics are last-writer-wins: it owns
  the flag while any session is active and clears it when the last session ends.
  This can clobber a `SleepDisabled` set manually via `pmset` or by another app; this
  limitation is documented.

## `daemon status` output

Two layers, so that status never activates keep-awake:

- Install/registration layer (no side effects): plist file presence and
  `launchctl print system/<label>`. No connection, no activation.
- Runtime layer: a `Query` connection. This may socket-activate the daemon, but
  because it does not hold, the reference count stays 0, `SleepDisabled` is untouched,
  and the daemon self-exits after the grace period.

Example (installed, with active sessions):

```
espresso daemon status
  Installed:        yes   /Library/LaunchDaemons/local.espresso.daemon.plist
  Registered:       yes   launchd: system/local.espresso.daemon
  Running:          yes   (pid 4821, up 3m12s)
  Active sessions:  2
  SleepDisabled:    1     (espresso: 2 active sessions)
  Lid:              open
  Socket:           /var/run/espresso.sock (present)
  Version:          daemon 0.2.0 / cli 0.2.0
```

Ownership labeling:

| Actual state | Displayed |
|---|---|
| daemon running and refcount > 0 | `SleepDisabled: 1 (espresso: N active sessions)` |
| `SleepDisabled = 1` but daemon not running / refcount 0 | `SleepDisabled: 1 (set by another process, not espresso)` |
| `SleepDisabled = 0` | `SleepDisabled: 0` |

Not installed:

```
espresso daemon status
  Installed:  no
  → run `espresso daemon install` (needs sudo) to enable lid-closed keep-awake
```

## Screen-off on lid close

On Apple Silicon the classic IOKit screen-off path (`IODisplayWrangler` +
`IORequestIdle`) does not exist. The daemon therefore shells out to
`pmset displaysleepnow` on the clamshell-closed edge. This is the only place that
spawns a subprocess; everything else is direct FFI. `pmset displaysleepnow` needs no
root and reliably turns off the display on Apple Silicon; while the lid is closed
there is no input, so the display stays off, and it wakes normally when the lid opens.

## Error handling

- Parse errors (bad `-t`, no arguments, past target time): stderr + usage, exit code 2.
- Assertion creation failure: hard error and exit — idle-sleep prevention is the
  baseline function, so continuing is meaningless.
- daemon not installed: first-use auto-prompt to install; if declined, degrade to
  assertion-only with a warning; the session continues.
- `pmset displaysleepnow` failure, or `SleepDisabled` set failure inside the daemon:
  log and continue best-effort; do not kill the session.
- Ctrl-C / SIGTERM in the CLI: release the assertion, close the socket, restore the
  terminal, then exit.
- Command mode: propagate the child's exit code.
- Platform: macOS/arm64 first; on a non-macOS platform, error at runtime.

## Testing

Pure-Rust unit tests (no root, no hardware):

- `time`: countdown integer parsing (positive / zero / negative / non-numeric),
  target formats (reused), future validation.
- `cli`: table-driven — given argv → resolved `Mode` enum, including "-t ignored when
  a command is present" and "no arguments errors".
- `progress`: existing tests reused.
- `ipc`: frame encode/decode round-trip.
- daemon reference-count state machine: extract the pure logic (`RefcountState` with
  transitions 0→1, 1→0, grace, new-connection-during-grace) and assert the emitted
  action sequence (`SetSleepDisabled(bool)`, `StartLid`, `StopLid`, `Exit`) without
  real IOKit or sockets. This turns the async correctness argument into tests.

FFI modules (`assertion`, `power`, `lid`) stay thin and are verified by smoke tests
and manual runs; no logic lives in them.

Final verification: `cargo test`, `cargo build`, and a real run on the machine
(idle-sleep prevention, lid-close screen-off without sleep on battery, daemon
install/status/uninstall).

## Out of scope (YAGNI)

- Keeping the display awake while the lid is open (normal display idle behavior is
  left intact and is battery-friendly).
- Per-caller authorization beyond same-host connection.
- Non-macOS or Intel-specific screen-off paths.
