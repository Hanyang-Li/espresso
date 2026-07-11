# Espresso Rust Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rework `espresso` from a `caffeinate` wrapper into direct IOKit control plus a root LaunchDaemon that owns the global `SleepDisabled` flag, so an active session never idle-sleeps on battery or AC and a closed lid only turns off the screen.

**Architecture:** The foreground CLI holds a per-process `PreventUserIdleSystemSleep` IOKit assertion (no root, kernel auto-releases on crash) and opens a `Hold` connection to the daemon. The daemon (root, launchd socket-activated) reference-counts client connections, drives `SleepDisabled`, watches the lid, and runs `pmset displaysleepnow` on close. IPC is a unix domain socket; the daemon coordinator runs single-threaded over an mpsc event channel so there is no accept-versus-exit race.

**Tech Stack:** Rust 2024; `chrono` (time parsing), `crossterm` (raw-mode progress), `clap` (arg parsing), `core-foundation` + `core-foundation-sys` (CF types), `libc` (unix socket fd / launchd), `anyhow` (CLI errors).

## Global Constraints

- Target platform: macOS 26.5+ on Apple Silicon (arm64). Error at runtime on other platforms.
- Rust edition 2024. Bump crate version to `0.2.0`.
- Only ONE subprocess spawn is allowed anywhere: `pmset displaysleepnow` (lid-close screen-off). Everything else is direct FFI.
- The foreground CLI holds the assertion; the daemon owns ONLY `SleepDisabled`.
- LaunchDaemon label: `tech.fintopia.espresso.daemon`. Plist path: `/Library/LaunchDaemons/tech.fintopia.espresso.daemon.plist`. Socket path: `/var/run/espresso.sock` (mode 0666). Idle-grace: 60s. Lid poll interval: 2s.
- `SleepDisabled` semantics are last-writer-wins (documented): espresso owns it while any session is active and clears it when the last session ends.
- The daemon coordinator must be single-threaded (all state mutation in one thread consuming an mpsc channel).

---

### Task 1: Dependencies and timer-argument parsing

Adds the `-t` value classifier (positive-integer seconds vs. existing datetime formats) to the time library, plus the new crate dependencies.

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/time.rs`
- Test: `src/time.rs` (inline `#[cfg(test)]` module) and `tests/time_parsing.rs`

**Interfaces:**
- Consumes: `parse_target_time(input: &str, now: DateTime<Local>) -> Result<DateTime<Local>, ParseTargetError>` (existing).
- Produces:
  - `pub enum TimerArgError { Empty, NotPositive(String), Unparseable(String), Target(ParseTargetError) }` with `Display` + `std::error::Error`.
  - `pub fn parse_timer_arg(value: &str, now: DateTime<Local>) -> Result<std::time::Duration, TimerArgError>`.

- [ ] **Step 1: Add dependencies to `Cargo.toml`**

Set the version and add the new deps under `[dependencies]`:

```toml
[package]
name = "espresso"
version = "0.2.0"
edition = "2024"

[dependencies]
anyhow = "1.0"
chrono = { version = "0.4", features = ["clock"] }
crossterm = "0.29"
clap = { version = "4", features = ["derive"] }
core-foundation = "0.10"
core-foundation-sys = "0.8"
libc = "0.2"
```

- [ ] **Step 2: Write failing tests for `parse_timer_arg`**

Append to `src/time.rs`:

```rust
#[cfg(test)]
mod timer_arg_tests {
    use super::*;
    use chrono::TimeZone;
    use std::time::Duration;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 7, 11, 12, 0, 0).single().unwrap()
    }

    #[test]
    fn positive_integer_is_countdown_seconds() {
        assert_eq!(parse_timer_arg("3600", now()), Ok(Duration::from_secs(3600)));
    }

    #[test]
    fn zero_is_rejected() {
        assert_eq!(parse_timer_arg("0", now()), Err(TimerArgError::NotPositive("0".into())));
    }

    #[test]
    fn negative_is_rejected() {
        assert_eq!(parse_timer_arg("-5", now()), Err(TimerArgError::NotPositive("-5".into())));
    }

    #[test]
    fn empty_is_rejected() {
        assert_eq!(parse_timer_arg("   ", now()), Err(TimerArgError::Empty));
    }

    #[test]
    fn clock_time_is_target() {
        // 13:00 is one hour after the fixed now of 12:00.
        assert_eq!(parse_timer_arg("13:00", now()), Ok(Duration::from_secs(3600)));
    }

    #[test]
    fn junk_is_unsupported_format() {
        assert!(matches!(parse_timer_arg("abc", now()), Err(TimerArgError::Target(_))));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib timer_arg_tests`
Expected: FAIL — `parse_timer_arg` / `TimerArgError` not found.

- [ ] **Step 4: Implement `TimerArgError` and `parse_timer_arg`**

Add near the top of `src/time.rs` (after the existing `use` lines add `use std::time::Duration;`), and place this after the `ParseTargetError` definitions:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerArgError {
    Empty,
    NotPositive(String),
    Unparseable(String),
    Target(ParseTargetError),
}

impl fmt::Display for TimerArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "-t requires a value"),
            Self::NotPositive(v) => write!(f, "-t countdown seconds must be greater than 0: {v}"),
            Self::Unparseable(v) => write!(f, "-t value is not a valid number: {v}"),
            Self::Target(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TimerArgError {}

pub fn parse_timer_arg(value: &str, now: DateTime<Local>) -> Result<Duration, TimerArgError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(TimerArgError::Empty);
    }

    if trimmed.bytes().all(|b| b.is_ascii_digit()) {
        let secs: u64 = trimmed
            .parse()
            .map_err(|_| TimerArgError::Unparseable(trimmed.to_string()))?;
        if secs == 0 {
            return Err(TimerArgError::NotPositive(trimmed.to_string()));
        }
        return Ok(Duration::from_secs(secs));
    }

    if let Ok(n) = trimmed.parse::<i64>() {
        if n <= 0 {
            return Err(TimerArgError::NotPositive(trimmed.to_string()));
        }
    }

    match parse_target_time(trimmed, now) {
        Ok(target) => {
            let millis = target.signed_duration_since(now).num_milliseconds().max(1) as u64;
            Ok(Duration::from_millis(millis))
        }
        Err(e) => Err(TimerArgError::Target(e)),
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib timer_arg_tests`
Expected: PASS (6 tests).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/time.rs
git commit -m "feat: add -t timer argument parsing and deps"
```

---

### Task 2: CLI argument model and mode resolution

Parses argv with clap into a raw shape, then resolves it to a `Mode` with a pure, table-tested function. Encodes the `-t`/command precedence and no-args-error rules.

**Files:**
- Create: `src/cli.rs`
- Modify: `src/lib.rs`
- Test: `src/cli.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `parse_timer_arg`, `TimerArgError` (Task 1).
- Produces:
  - `pub struct Cli { pub time: Option<String>, pub rest: Vec<String> }` (clap `Parser`).
  - `pub enum Mode { Timer(Duration), Command(Vec<String>), DaemonInstall, DaemonUninstall, DaemonStatus, DaemonRuntime }`.
  - `pub enum CliError { NoArgs, UnknownDaemonSub(String), Timer(TimerArgError) }` with `Display`.
  - `pub fn resolve_mode(cli: Cli, now: DateTime<Local>) -> Result<Mode, CliError>`.

- [ ] **Step 1: Register the module**

In `src/lib.rs` add:

```rust
pub mod cli;
pub mod progress;
pub mod time;
```

- [ ] **Step 2: Write failing table tests**

Create `src/cli.rs` with only the tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};
    use std::time::Duration;

    fn now() -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2026, 7, 11, 12, 0, 0).single().unwrap()
    }

    fn cli(args: &[&str]) -> Cli {
        let mut full = vec!["espresso"];
        full.extend_from_slice(args);
        <Cli as clap::Parser>::try_parse_from(full).expect("parse")
    }

    #[test]
    fn no_args_is_error() {
        assert!(matches!(resolve_mode(cli(&[]), now()), Err(CliError::NoArgs)));
    }

    #[test]
    fn t_seconds_is_timer() {
        assert_eq!(resolve_mode(cli(&["-t", "60"]), now()), Ok(Mode::Timer(Duration::from_secs(60))));
    }

    #[test]
    fn positional_is_command() {
        assert_eq!(
            resolve_mode(cli(&["npm", "run", "build"]), now()),
            Ok(Mode::Command(vec!["npm".into(), "run".into(), "build".into()]))
        );
    }

    #[test]
    fn command_ignores_t_without_error() {
        assert_eq!(
            resolve_mode(cli(&["-t", "60", "sleep", "1"]), now()),
            Ok(Mode::Command(vec!["sleep".into(), "1".into()]))
        );
    }

    #[test]
    fn daemon_status_subcommand() {
        assert_eq!(resolve_mode(cli(&["daemon", "status"]), now()), Ok(Mode::DaemonStatus));
    }

    #[test]
    fn daemon_unknown_subcommand_errors() {
        assert!(matches!(
            resolve_mode(cli(&["daemon", "frobnicate"]), now()),
            Err(CliError::UnknownDaemonSub(_))
        ));
    }

    #[test]
    fn hidden_daemon_runtime() {
        assert_eq!(resolve_mode(cli(&["__daemon"]), now()), Ok(Mode::DaemonRuntime));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib cli::tests`
Expected: FAIL — `Cli` / `resolve_mode` not found.

- [ ] **Step 4: Implement the CLI model and resolver**

Prepend to `src/cli.rs` (above the test module):

```rust
use crate::time::{parse_timer_arg, TimerArgError};
use chrono::{DateTime, Local};
use clap::Parser;
use std::fmt;
use std::time::Duration;

/// espresso — keep this Mac awake; closing the lid only turns off the screen.
#[derive(Parser, Debug)]
#[command(name = "espresso", version, trailing_var_arg = true)]
pub struct Cli {
    /// Countdown seconds (>0) or a target time (HH:mm, yyyy-MM-dd HH:mm, ...).
    #[arg(short = 't', long = "time")]
    pub time: Option<String>,

    /// A `daemon <sub>` management command, or the command to run while active.
    #[arg(allow_hyphen_values = true)]
    pub rest: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Timer(Duration),
    Command(Vec<String>),
    DaemonInstall,
    DaemonUninstall,
    DaemonStatus,
    DaemonRuntime,
}

#[derive(Debug)]
pub enum CliError {
    NoArgs,
    UnknownDaemonSub(String),
    Timer(TimerArgError),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoArgs => write!(
                f,
                "usage: espresso -t <seconds|time> | espresso <command...> | espresso daemon <install|uninstall|status>"
            ),
            Self::UnknownDaemonSub(s) => {
                write!(f, "unknown daemon subcommand: '{s}' (expected install|uninstall|status)")
            }
            Self::Timer(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CliError {}

pub fn resolve_mode(cli: Cli, now: DateTime<Local>) -> Result<Mode, CliError> {
    if let Some(first) = cli.rest.first() {
        if first == "__daemon" {
            return Ok(Mode::DaemonRuntime);
        }
        if first == "daemon" {
            return match cli.rest.get(1).map(String::as_str) {
                Some("install") => Ok(Mode::DaemonInstall),
                Some("uninstall") => Ok(Mode::DaemonUninstall),
                Some("status") => Ok(Mode::DaemonStatus),
                Some(other) => Err(CliError::UnknownDaemonSub(other.to_string())),
                None => Err(CliError::UnknownDaemonSub(String::new())),
            };
        }
        // Positional command present: -t is ignored, no error.
        return Ok(Mode::Command(cli.rest));
    }

    match cli.time {
        Some(v) => parse_timer_arg(&v, now).map(Mode::Timer).map_err(CliError::Timer),
        None => Err(CliError::NoArgs),
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib cli::tests`
Expected: PASS (7 tests).

- [ ] **Step 6: Commit**

```bash
git add src/cli.rs src/lib.rs
git commit -m "feat: add clap CLI model and mode resolution"
```

---

### Task 3: IPC wire protocol

Text-line protocol shared by client and daemon. Pure encode/decode with round-trip tests.

**Files:**
- Create: `src/ipc.rs`
- Modify: `src/lib.rs`
- Test: `src/ipc.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub const SOCKET_PATH: &str = "/var/run/espresso.sock";`
  - `pub enum ClientMsg { Hold, Query }`
  - `pub struct StatusInfo { pub refcount: u32, pub sleep_disabled: bool, pub lid_closed: bool, pub pid: u32, pub version: String }`
  - `pub enum ServerMsg { Ok, Status(StatusInfo) }`
  - `pub fn encode_client(m: &ClientMsg) -> String`
  - `pub fn decode_client(line: &str) -> Result<ClientMsg, IpcError>`
  - `pub fn encode_server(m: &ServerMsg) -> String`
  - `pub fn decode_server(line: &str) -> Result<ServerMsg, IpcError>`
  - `pub enum IpcError { Malformed(String) }` with `Display`.

- [ ] **Step 1: Register the module**

In `src/lib.rs` add `pub mod ipc;` (keep modules alphabetical: `cli, ipc, progress, time`).

- [ ] **Step 2: Write failing round-trip tests**

Create `src/ipc.rs` with the tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_round_trip() {
        for m in [ClientMsg::Hold, ClientMsg::Query] {
            let line = encode_client(&m);
            assert_eq!(decode_client(line.trim_end()), Ok(m));
        }
    }

    #[test]
    fn status_round_trip() {
        let info = StatusInfo {
            refcount: 2,
            sleep_disabled: true,
            lid_closed: false,
            pid: 4821,
            version: "0.2.0".into(),
        };
        let line = encode_server(&ServerMsg::Status(info.clone()));
        assert_eq!(decode_server(line.trim_end()), Ok(ServerMsg::Status(info)));
    }

    #[test]
    fn ok_round_trip() {
        let line = encode_server(&ServerMsg::Ok);
        assert_eq!(decode_server(line.trim_end()), Ok(ServerMsg::Ok));
    }

    #[test]
    fn malformed_client_rejected() {
        assert!(matches!(decode_client("NONSENSE"), Err(IpcError::Malformed(_))));
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib ipc::tests`
Expected: FAIL — types not found.

- [ ] **Step 4: Implement the protocol**

Prepend to `src/ipc.rs`:

```rust
use std::fmt;

pub const SOCKET_PATH: &str = "/var/run/espresso.sock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Hold,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusInfo {
    pub refcount: u32,
    pub sleep_disabled: bool,
    pub lid_closed: bool,
    pub pid: u32,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    Ok,
    Status(StatusInfo),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcError {
    Malformed(String),
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(s) => write!(f, "malformed IPC message: {s}"),
        }
    }
}

impl std::error::Error for IpcError {}

pub fn encode_client(m: &ClientMsg) -> String {
    match m {
        ClientMsg::Hold => "HOLD\n".to_string(),
        ClientMsg::Query => "QUERY\n".to_string(),
    }
}

pub fn decode_client(line: &str) -> Result<ClientMsg, IpcError> {
    match line.trim() {
        "HOLD" => Ok(ClientMsg::Hold),
        "QUERY" => Ok(ClientMsg::Query),
        other => Err(IpcError::Malformed(other.to_string())),
    }
}

pub fn encode_server(m: &ServerMsg) -> String {
    match m {
        ServerMsg::Ok => "OK\n".to_string(),
        ServerMsg::Status(s) => format!(
            "STATUS refcount={} sleep_disabled={} lid_closed={} pid={} version={}\n",
            s.refcount,
            s.sleep_disabled as u8,
            s.lid_closed as u8,
            s.pid,
            s.version,
        ),
    }
}

pub fn decode_server(line: &str) -> Result<ServerMsg, IpcError> {
    let line = line.trim();
    if line == "OK" {
        return Ok(ServerMsg::Ok);
    }
    let rest = line
        .strip_prefix("STATUS ")
        .ok_or_else(|| IpcError::Malformed(line.to_string()))?;

    let mut refcount = None;
    let mut sleep_disabled = None;
    let mut lid_closed = None;
    let mut pid = None;
    let mut version = None;

    for field in rest.split_whitespace() {
        let (k, v) = field
            .split_once('=')
            .ok_or_else(|| IpcError::Malformed(field.to_string()))?;
        match k {
            "refcount" => refcount = v.parse().ok(),
            "sleep_disabled" => sleep_disabled = Some(v == "1"),
            "lid_closed" => lid_closed = Some(v == "1"),
            "pid" => pid = v.parse().ok(),
            "version" => version = Some(v.to_string()),
            _ => {}
        }
    }

    Ok(ServerMsg::Status(StatusInfo {
        refcount: refcount.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
        sleep_disabled: sleep_disabled.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
        lid_closed: lid_closed.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
        pid: pid.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
        version: version.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
    }))
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib ipc::tests`
Expected: PASS (4 tests).

- [ ] **Step 6: Commit**

```bash
git add src/ipc.rs src/lib.rs
git commit -m "feat: add unix-socket IPC wire protocol"
```

---

### Task 4: Reference-count state machine

The pure logic that turns Hold-open / Hold-close / grace-elapsed events into daemon actions. This is where the async-correctness argument becomes tests.

**Files:**
- Create: `src/refcount.rs`
- Modify: `src/lib.rs`
- Test: `src/refcount.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub enum Action { SetSleepDisabled(bool), StartLidWatch, StopLidWatch, StartGraceTimer, CancelGraceTimer, Exit }`
  - `pub struct RefcountState { /* private */ }`
  - `impl RefcountState { pub fn new() -> Self; pub fn on_hold_open(&mut self) -> Vec<Action>; pub fn on_hold_close(&mut self) -> Vec<Action>; pub fn on_grace_elapsed(&mut self) -> Vec<Action>; pub fn count(&self) -> u32 }`

- [ ] **Step 1: Register the module**

In `src/lib.rs` add `pub mod refcount;` (alphabetical: `cli, ipc, progress, refcount, time`).

- [ ] **Step 2: Write failing transition tests**

Create `src/refcount.rs` with the tests first:

```rust
#[cfg(test)]
mod tests {
    use super::Action::*;
    use super::*;

    #[test]
    fn first_hold_enables_and_watches() {
        let mut s = RefcountState::new();
        assert_eq!(s.on_hold_open(), vec![SetSleepDisabled(true), StartLidWatch]);
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn second_hold_is_noop() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        assert_eq!(s.on_hold_open(), vec![]);
        assert_eq!(s.count(), 2);
    }

    #[test]
    fn last_close_disables_and_starts_grace() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        assert_eq!(
            s.on_hold_close(),
            vec![SetSleepDisabled(false), StopLidWatch, StartGraceTimer]
        );
        assert_eq!(s.count(), 0);
    }

    #[test]
    fn non_last_close_is_noop() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        s.on_hold_open();
        assert_eq!(s.on_hold_close(), vec![]);
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn grace_elapsed_at_zero_exits() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        s.on_hold_close();
        assert_eq!(s.on_grace_elapsed(), vec![Exit]);
    }

    #[test]
    fn hold_during_grace_cancels_and_reenables() {
        let mut s = RefcountState::new();
        s.on_hold_open();
        s.on_hold_close(); // enters grace
        assert_eq!(
            s.on_hold_open(),
            vec![CancelGraceTimer, SetSleepDisabled(true), StartLidWatch]
        );
        // A stale grace-elapsed after re-enable must be ignored.
        assert_eq!(s.on_grace_elapsed(), vec![]);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib refcount::tests`
Expected: FAIL — types not found.

- [ ] **Step 4: Implement the state machine**

Prepend to `src/refcount.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    SetSleepDisabled(bool),
    StartLidWatch,
    StopLidWatch,
    StartGraceTimer,
    CancelGraceTimer,
    Exit,
}

#[derive(Debug, Default)]
pub struct RefcountState {
    count: u32,
    in_grace: bool,
}

impl RefcountState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn on_hold_open(&mut self) -> Vec<Action> {
        self.count += 1;
        if self.count != 1 {
            return vec![];
        }
        let mut actions = Vec::new();
        if self.in_grace {
            self.in_grace = false;
            actions.push(Action::CancelGraceTimer);
        }
        actions.push(Action::SetSleepDisabled(true));
        actions.push(Action::StartLidWatch);
        actions
    }

    pub fn on_hold_close(&mut self) -> Vec<Action> {
        self.count = self.count.saturating_sub(1);
        if self.count != 0 {
            return vec![];
        }
        self.in_grace = true;
        vec![
            Action::SetSleepDisabled(false),
            Action::StopLidWatch,
            Action::StartGraceTimer,
        ]
    }

    pub fn on_grace_elapsed(&mut self) -> Vec<Action> {
        if self.count == 0 && self.in_grace {
            vec![Action::Exit]
        } else {
            vec![]
        }
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib refcount::tests`
Expected: PASS (6 tests).

- [ ] **Step 6: Commit**

```bash
git add src/refcount.rs src/lib.rs
git commit -m "feat: add daemon reference-count state machine"
```

---

### Task 5: IOKit power assertion (idle-sleep)

Thin FFI wrapper for the CLI-held `PreventUserIdleSystemSleep` assertion. Verified by a smoke test (no unit test — requires the real framework).

**Files:**
- Create: `src/assertion.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct SleepAssertion { /* private */ }`
  - `impl SleepAssertion { pub fn prevent_idle_sleep(reason: &str) -> std::io::Result<Self> }`
  - `Drop for SleepAssertion` releases the assertion.

- [ ] **Step 1: Register the module**

In `src/lib.rs` add `pub mod assertion;` (alphabetical order).

- [ ] **Step 2: Implement the assertion wrapper**

Create `src/assertion.rs`:

```rust
use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation_sys::string::CFStringRef;
use std::io;

#[allow(non_upper_case_globals)]
const kIOPMAssertionLevelOn: u32 = 255;

type IOPMAssertionID = u32;
type IOReturn = i32;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CFStringRef,
        assertion_level: u32,
        assertion_name: CFStringRef,
        assertion_id: *mut IOPMAssertionID,
    ) -> IOReturn;
    fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
}

/// Holds a `PreventUserIdleSystemSleep` assertion for the process lifetime.
/// The kernel also releases it automatically if the process dies.
pub struct SleepAssertion {
    id: IOPMAssertionID,
}

impl SleepAssertion {
    pub fn prevent_idle_sleep(reason: &str) -> io::Result<Self> {
        let assertion_type = CFString::new("PreventUserIdleSystemSleep");
        let name = CFString::new(reason);
        let mut id: IOPMAssertionID = 0;
        let rc = unsafe {
            IOPMAssertionCreateWithName(
                assertion_type.as_concrete_TypeRef(),
                kIOPMAssertionLevelOn,
                name.as_concrete_TypeRef(),
                &mut id,
            )
        };
        if rc != 0 {
            return Err(io::Error::other(format!(
                "IOPMAssertionCreateWithName failed: {rc:#x}"
            )));
        }
        Ok(Self { id })
    }
}

impl Drop for SleepAssertion {
    fn drop(&mut self) {
        unsafe {
            IOPMAssertionRelease(self.id);
        }
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly (links `IOKit`).

- [ ] **Step 4: Smoke-test the assertion manually**

Add a temporary check and observe it in `pmset`:

Run:
```bash
cargo run --quiet -- __smoke_assertion &
sleep 1
pmset -g assertions | grep -i PreventUserIdleSystemSleep
```

Since `__smoke_assertion` is not yet wired, instead verify with a throwaway `examples/` binary:

Create `examples/smoke_assertion.rs`:
```rust
fn main() {
    let _a = espresso::assertion::SleepAssertion::prevent_idle_sleep("espresso smoke").unwrap();
    println!("assertion held; check `pmset -g assertions` then press enter");
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf).unwrap();
}
```
Run:
```bash
cargo run --example smoke_assertion &
sleep 1
pmset -g assertions | grep -i PreventUserIdleSystemSleep
```
Expected: a line showing `PreventUserIdleSystemSleep 1` attributed to the process. Then `kill %1`.

- [ ] **Step 5: Commit**

```bash
git add src/assertion.rs src/lib.rs examples/smoke_assertion.rs
git commit -m "feat: add PreventUserIdleSystemSleep assertion wrapper"
```

---

### Task 6: IOKit SleepDisabled control and read

Thin FFI for setting and reading the global `SleepDisabled` flag (root to set).

**Files:**
- Create: `src/power.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces:
  - `pub fn set_sleep_disabled(disabled: bool) -> std::io::Result<()>` (requires root to succeed).
  - `pub fn read_sleep_disabled() -> std::io::Result<bool>` (no root; returns current flag, false if absent).

- [ ] **Step 1: Register the module**

In `src/lib.rs` add `pub mod power;` (alphabetical order).

- [ ] **Step 2: Implement set/read**

Create `src/power.rs`:

```rust
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_foundation_sys::base::CFTypeRef;
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::string::CFStringRef;
use std::io;

type IOReturn = i32;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPMSetSystemPowerSetting(key: CFStringRef, value: CFTypeRef) -> IOReturn;
    fn IOPMCopySystemPowerSettings() -> CFDictionaryRef;
}

pub fn set_sleep_disabled(disabled: bool) -> io::Result<()> {
    let key = CFString::new("SleepDisabled");
    let value = CFBoolean::from(disabled);
    let rc = unsafe {
        IOPMSetSystemPowerSetting(key.as_concrete_TypeRef(), value.as_CFTypeRef())
    };
    if rc != 0 {
        return Err(io::Error::other(format!(
            "IOPMSetSystemPowerSetting(SleepDisabled) failed: {rc:#x} (requires root)"
        )));
    }
    Ok(())
}

pub fn read_sleep_disabled() -> io::Result<bool> {
    let dict_ref = unsafe { IOPMCopySystemPowerSettings() };
    if dict_ref.is_null() {
        return Ok(false);
    }
    let dict: CFDictionary =
        unsafe { CFDictionary::wrap_under_create_rule(dict_ref) };
    let key = CFString::new("SleepDisabled");
    match dict.find(key.as_concrete_TypeRef() as *const _) {
        Some(value) => {
            let ty = unsafe { CFType::wrap_under_get_rule(*value) };
            let b = ty
                .downcast::<CFBoolean>()
                .map(|b| b == CFBoolean::true_value())
                .unwrap_or(false);
            Ok(b)
        }
        None => Ok(false),
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 4: Smoke-test set/read manually**

Create `examples/smoke_power.rs`:
```rust
fn main() {
    println!("before: SleepDisabled = {}", espresso::power::read_sleep_disabled().unwrap());
    espresso::power::set_sleep_disabled(true).expect("set true (run with sudo)");
    println!("after set true: SleepDisabled = {}", espresso::power::read_sleep_disabled().unwrap());
    espresso::power::set_sleep_disabled(false).unwrap();
    println!("after set false: SleepDisabled = {}", espresso::power::read_sleep_disabled().unwrap());
}
```
Run:
```bash
cargo build --example smoke_power
sudo ./target/debug/examples/smoke_power
pmset -g | grep -i SleepDisabled
```
Expected: prints `false`, `true`, `false`; the flag returns to 0 at the end.

- [ ] **Step 5: Commit**

```bash
git add src/power.rs src/lib.rs examples/smoke_power.rs
git commit -m "feat: add SleepDisabled set/read via IOKit"
```

---

### Task 7: Lid state read

Reads `AppleClamshellState` from the IORegistry. Absent property (desktops) reads as open.

**Files:**
- Create: `src/lid.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `pub fn lid_closed() -> std::io::Result<bool>`.

- [ ] **Step 1: Register the module**

In `src/lib.rs` add `pub mod lid;` (alphabetical order).

- [ ] **Step 2: Implement the lid read**

Create `src/lid.rs`:

```rust
use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease, CFTypeRef};
use core_foundation_sys::number::{kCFBooleanTrue, CFBooleanRef};
use core_foundation_sys::string::CFStringRef;
use std::io;
use std::os::raw::c_char;

type IOOptionBits = u32;
type IOReturn = i32;
type MachPort = u32;
type IoObject = u32;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut core_foundation_sys::dictionary::__CFDictionary;
    fn IOServiceGetMatchingService(
        main_port: MachPort,
        matching: *const core_foundation_sys::dictionary::__CFDictionary,
    ) -> IoObject;
    fn IORegistryEntryCreateCFProperty(
        entry: IoObject,
        key: CFStringRef,
        allocator: core_foundation_sys::base::CFAllocatorRef,
        options: IOOptionBits,
    ) -> CFTypeRef;
    fn IOObjectRelease(object: IoObject) -> IOReturn;
}

pub fn lid_closed() -> io::Result<bool> {
    unsafe {
        let matching = IOServiceMatching(c"IOPMrootDomain".as_ptr());
        if matching.is_null() {
            return Err(io::Error::other("IOServiceMatching(IOPMrootDomain) returned null"));
        }
        // IOServiceGetMatchingService consumes the matching dictionary reference.
        let service = IOServiceGetMatchingService(0, matching);
        if service == 0 {
            return Err(io::Error::other("IOPMrootDomain service not found"));
        }
        let key = CFString::new("AppleClamshellState");
        let prop = IORegistryEntryCreateCFProperty(
            service,
            key.as_concrete_TypeRef(),
            kCFAllocatorDefault,
            0,
        );
        IOObjectRelease(service);
        if prop.is_null() {
            // No clamshell (e.g. desktop Mac): treat as open.
            return Ok(false);
        }
        let is_true = prop as CFBooleanRef == kCFBooleanTrue;
        CFRelease(prop);
        Ok(is_true)
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 4: Smoke-test manually**

Create `examples/smoke_lid.rs`:
```rust
fn main() {
    for _ in 0..10 {
        println!("lid_closed = {}", espresso::lid::lid_closed().unwrap());
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
```
Run `cargo run --example smoke_lid`, then close and open the lid (connect an external display/keyboard so the machine stays usable, or observe via SSH). Expected: value flips to `true` while closed, `false` while open.

- [ ] **Step 5: Commit**

```bash
git add src/lid.rs src/lib.rs examples/smoke_lid.rs
git commit -m "feat: add AppleClamshellState lid read"
```

---

### Task 8: Daemon coordinator and runtime

Wires the socket, the reference-count state machine, power/lid FFI, and `pmset displaysleepnow` into a single-threaded coordinator fed by an mpsc event channel. launchd hands the listening socket via `launch_activate_socket`, with a bind fallback for manual runs.

**Files:**
- Create: `src/daemon.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `ipc::{ClientMsg, ServerMsg, StatusInfo, SOCKET_PATH, decode_client, encode_server, encode_client}`, `refcount::{RefcountState, Action}`, `power::{set_sleep_disabled, read_sleep_disabled}`, `lid::lid_closed`.
- Produces:
  - `pub fn run() -> anyhow::Result<()>` — the daemon entry point (`espresso __daemon`).
  - `pub fn query_status() -> std::io::Result<Option<ipc::StatusInfo>>` — client-side helper: connect, send `QUERY`, parse the reply; `Ok(None)` if the socket is absent/unreachable.
  - `pub fn hold_connection() -> std::io::Result<std::os::unix::net::UnixStream>` — client-side: connect and send `HOLD`, returning the open stream (dropping it releases the hold). `Err` if the socket is absent.

- [ ] **Step 1: Register the module**

In `src/lib.rs` add `pub mod daemon;` (alphabetical order).

- [ ] **Step 2: Implement the client helpers and the coordinator**

Create `src/daemon.rs`:

```rust
use crate::ipc::{
    decode_client, decode_server, encode_client, encode_server, ClientMsg, ServerMsg, StatusInfo,
    SOCKET_PATH,
};
use crate::lid::lid_closed;
use crate::power::set_sleep_disabled;
use crate::refcount::{Action, RefcountState};
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::FromRawFd;
use std::os::raw::c_char;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const GRACE: Duration = Duration::from_secs(60);
const LID_POLL: Duration = Duration::from_secs(2);
const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------- client helpers ----------

pub fn hold_connection() -> std::io::Result<UnixStream> {
    let mut stream = UnixStream::connect(SOCKET_PATH)?;
    stream.write_all(encode_client(&ClientMsg::Hold).as_bytes())?;
    stream.flush()?;
    Ok(stream)
}

pub fn query_status() -> std::io::Result<Option<StatusInfo>> {
    let mut stream = match UnixStream::connect(SOCKET_PATH) {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };
    stream.write_all(encode_client(&ClientMsg::Query).as_bytes())?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    match decode_server(&line) {
        Ok(ServerMsg::Status(info)) => Ok(Some(info)),
        _ => Ok(None),
    }
}

// ---------- daemon side ----------

enum Event {
    HoldOpened,
    HoldClosed,
    GraceElapsed(u64),
    Query(Sender<StatusInfo>),
}

pub fn run() -> Result<()> {
    // Crash recovery: clear any stale flag we might have left set.
    let _ = set_sleep_disabled(false);

    let listener = obtain_listener().context("failed to obtain daemon socket")?;
    let (tx, rx) = mpsc::channel::<Event>();

    // Accept loop: each Hold connection becomes a thread that reports open/close;
    // each Query connection is answered from a fresh status snapshot.
    {
        let tx = tx.clone();
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { continue };
                let tx = tx.clone();
                thread::spawn(move || handle_connection(conn, tx));
            }
        });
    }

    coordinator(rx, tx);
    Ok(())
}

fn handle_connection(mut conn: UnixStream, tx: Sender<Event>) {
    let mut reader = BufReader::new(match conn.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    match decode_client(&line) {
        Ok(ClientMsg::Hold) => {
            if tx.send(Event::HoldOpened).is_err() {
                return;
            }
            let _ = conn.write_all(encode_server(&ServerMsg::Ok).as_bytes());
            let _ = conn.flush();
            // Block until the client goes away (EOF or error), including crash.
            let mut buf = [0u8; 64];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = tx.send(Event::HoldClosed);
        }
        Ok(ClientMsg::Query) => {
            let (rtx, rrx) = mpsc::channel::<StatusInfo>();
            if tx.send(Event::Query(rtx)).is_ok() {
                if let Ok(info) = rrx.recv() {
                    let _ = conn.write_all(encode_server(&ServerMsg::Status(info)).as_bytes());
                    let _ = conn.flush();
                }
            }
        }
        Err(_) => {}
    }
}

fn coordinator(rx: Receiver<Event>, tx: Sender<Event>) {
    let mut state = RefcountState::new();
    let mut grace_gen: u64 = 0;
    let lid_stop = Arc::new(AtomicBool::new(false));

    while let Ok(event) = rx.recv() {
        let actions = match event {
            Event::HoldOpened => state.on_hold_open(),
            Event::HoldClosed => state.on_hold_close(),
            Event::GraceElapsed(gen) => {
                if gen == grace_gen {
                    state.on_grace_elapsed()
                } else {
                    vec![]
                }
            }
            Event::Query(reply) => {
                let info = StatusInfo {
                    refcount: state.count(),
                    sleep_disabled: state.count() > 0,
                    lid_closed: lid_closed().unwrap_or(false),
                    pid: std::process::id(),
                    version: VERSION.to_string(),
                };
                let _ = reply.send(info);
                vec![]
            }
        };

        for action in actions {
            match action {
                Action::SetSleepDisabled(v) => {
                    if let Err(e) = set_sleep_disabled(v) {
                        eprintln!("espresso daemon: set_sleep_disabled({v}) failed: {e}");
                    }
                }
                Action::StartLidWatch => {
                    lid_stop.store(false, Ordering::SeqCst);
                    spawn_lid_watch(lid_stop.clone());
                }
                Action::StopLidWatch => {
                    lid_stop.store(true, Ordering::SeqCst);
                }
                Action::StartGraceTimer => {
                    grace_gen += 1;
                    let gen = grace_gen;
                    let tx = tx.clone();
                    thread::spawn(move || {
                        thread::sleep(GRACE);
                        let _ = tx.send(Event::GraceElapsed(gen));
                    });
                }
                Action::CancelGraceTimer => {
                    grace_gen += 1; // invalidates the pending timer's generation
                }
                Action::Exit => {
                    let _ = set_sleep_disabled(false);
                    std::process::exit(0);
                }
            }
        }
    }
}

fn spawn_lid_watch(stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut was_closed = false;
        while !stop.load(Ordering::SeqCst) {
            let closed = lid_closed().unwrap_or(false);
            if closed && !was_closed {
                display_sleep_now();
            }
            was_closed = closed;
            thread::sleep(LID_POLL);
        }
    });
}

fn display_sleep_now() {
    // The single permitted subprocess: turn the display off on Apple Silicon.
    let _ = std::process::Command::new("pmset")
        .arg("displaysleepnow")
        .status();
}

// ---------- launchd socket activation ----------

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    fn launch_activate_socket(name: *const c_char, fds: *mut *mut i32, count: *mut usize) -> i32;
}

fn obtain_listener() -> Result<UnixListener> {
    // Preferred path: launchd handed us the listening socket.
    unsafe {
        let mut fds: *mut i32 = std::ptr::null_mut();
        let mut count: usize = 0;
        let rc = launch_activate_socket(c"Listener".as_ptr(), &mut fds, &mut count);
        if rc == 0 && count > 0 && !fds.is_null() {
            let fd = *fds;
            libc::free(fds as *mut libc::c_void);
            return Ok(UnixListener::from_raw_fd(fd));
        }
    }
    // Fallback for manual runs: bind the path ourselves.
    let _ = std::fs::remove_file(SOCKET_PATH);
    let listener = UnixListener::bind(SOCKET_PATH)
        .with_context(|| format!("failed to bind {SOCKET_PATH}"))?;
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o666);
    std::fs::set_permissions(SOCKET_PATH, mode).ok();
    Ok(listener)
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 4: Manual end-to-end daemon smoke test**

Run the daemon in its bind-fallback mode and drive it with two holds:

```bash
sudo ./target/debug/espresso __daemon &          # after `cargo build`
sleep 1
# In another shell, use the example client from Task 10, or netcat-style check:
printf 'QUERY\n' | nc -U /var/run/espresso.sock
```
Expected: a `STATUS refcount=0 sleep_disabled=0 ...` line. (Full hold behaviour is exercised in Task 10.) Then `sudo kill %1` and confirm `pmset -g | grep SleepDisabled` shows `0`.

- [ ] **Step 5: Commit**

```bash
git add src/daemon.rs src/lib.rs
git commit -m "feat: add daemon coordinator, socket activation, and client helpers"
```

---

### Task 9: Daemon install / uninstall / status

Writes and loads the LaunchDaemon plist, and renders `daemon status` from the install layer plus a `Query`.

**Files:**
- Create: `src/install.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `daemon::query_status`, `power::read_sleep_disabled`, `lid::lid_closed`, `ipc::{SOCKET_PATH}`.
- Produces:
  - `pub const LABEL: &str`, `pub const PLIST_PATH: &str`.
  - `pub fn install() -> anyhow::Result<()>` (requires root; writes plist + `launchctl bootstrap`).
  - `pub fn uninstall() -> anyhow::Result<()>` (requires root; `launchctl bootout` + remove plist).
  - `pub fn is_installed() -> bool` (plist file present).
  - `pub fn print_status() -> anyhow::Result<()>`.
  - `pub fn ensure_installed_interactive() -> anyhow::Result<bool>` — if not installed, prompt and run `sudo espresso daemon install`; return whether the daemon is now installed.

- [ ] **Step 1: Register the module**

In `src/lib.rs` add `pub mod install;` (alphabetical order).

- [ ] **Step 2: Implement install/uninstall/status**

Create `src/install.rs`:

```rust
use crate::daemon::query_status;
use crate::ipc::SOCKET_PATH;
use crate::power::read_sleep_disabled;
use anyhow::{bail, Context, Result};
use std::io::Write;
use std::process::Command;

pub const LABEL: &str = "tech.fintopia.espresso.daemon";
pub const PLIST_PATH: &str = "/Library/LaunchDaemons/tech.fintopia.espresso.daemon.plist";

fn plist_contents(program: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>
        <string>__daemon</string>
    </array>
    <key>Sockets</key>
    <dict>
        <key>Listener</key>
        <dict>
            <key>SockPathName</key>
            <string>{SOCKET_PATH}</string>
            <key>SockPathMode</key>
            <integer>438</integer>
        </dict>
    </dict>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
</dict>
</plist>
"#
    )
}

pub fn is_installed() -> bool {
    std::path::Path::new(PLIST_PATH).exists()
}

fn require_root() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("this command requires root; run: sudo espresso daemon install");
    }
    Ok(())
}

pub fn install() -> Result<()> {
    require_root()?;
    let program = std::env::current_exe()
        .context("failed to resolve current executable path")?
        .to_string_lossy()
        .into_owned();
    std::fs::write(PLIST_PATH, plist_contents(&program))
        .with_context(|| format!("failed to write {PLIST_PATH}"))?;
    let status = Command::new("launchctl")
        .args(["bootstrap", "system", PLIST_PATH])
        .status()
        .context("failed to run launchctl bootstrap")?;
    if !status.success() {
        // Already bootstrapped is acceptable; report others.
        eprintln!("espresso: launchctl bootstrap returned {status} (may already be loaded)");
    }
    println!("espresso daemon installed ({LABEL}).");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    require_root()?;
    let _ = Command::new("launchctl")
        .args(["bootout", &format!("system/{LABEL}")])
        .status();
    let _ = std::fs::remove_file(PLIST_PATH);
    let _ = std::fs::remove_file(SOCKET_PATH);
    println!("espresso daemon uninstalled.");
    Ok(())
}

pub fn print_status() -> Result<()> {
    if !is_installed() {
        println!("espresso daemon status");
        println!("  Installed:  no");
        println!("  → run `espresso daemon install` (needs sudo) to enable lid-closed keep-awake");
        return Ok(());
    }

    let registered = Command::new("launchctl")
        .args(["print", &format!("system/{LABEL}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    println!("espresso daemon status");
    println!("  Installed:        yes   {PLIST_PATH}");
    println!(
        "  Registered:       {}   launchd: system/{LABEL}",
        if registered { "yes" } else { "no" }
    );

    match query_status()? {
        Some(info) => {
            println!("  Running:          yes   (pid {})", info.pid);
            println!("  Active sessions:  {}", info.refcount);
            if info.sleep_disabled {
                println!(
                    "  SleepDisabled:    1     (espresso: {} active sessions)",
                    info.refcount
                );
            } else {
                report_flag_without_daemon()?;
            }
            println!("  Lid:              {}", if info.lid_closed { "closed" } else { "open" });
            println!("  Socket:           {SOCKET_PATH} (present)");
            println!("  Version:          daemon {} / cli {}", info.version, env!("CARGO_PKG_VERSION"));
        }
        None => {
            println!("  Running:          no (idle, starts on demand)");
            report_flag_without_daemon()?;
        }
    }
    Ok(())
}

fn report_flag_without_daemon() -> Result<()> {
    if read_sleep_disabled()? {
        println!("  SleepDisabled:    1     (set by another process, not espresso)");
    } else {
        println!("  SleepDisabled:    0");
    }
    Ok(())
}

/// If the daemon is not installed, prompt and (on yes) run `sudo espresso daemon install`.
/// Returns whether the daemon is installed afterwards.
pub fn ensure_installed_interactive() -> Result<bool> {
    if is_installed() {
        return Ok(true);
    }
    eprint!(
        "espresso needs a one-time privileged setup to keep the Mac awake with the lid closed.\n\
         Install the helper now with sudo? [y/N] "
    );
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).ok();
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        return Ok(false);
    }
    let program = std::env::current_exe()?.to_string_lossy().into_owned();
    let status = Command::new("sudo")
        .args([&program, "daemon", "install"])
        .status()
        .context("failed to run sudo espresso daemon install")?;
    Ok(status.success() && is_installed())
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 4: Manual install/status/uninstall test**

```bash
cargo build
sudo ./target/debug/espresso daemon install
./target/debug/espresso daemon status      # Installed: yes; Running: no (idle)
sudo ./target/debug/espresso daemon uninstall
./target/debug/espresso daemon status      # Installed: no
```
Expected: status transitions as annotated; no `SleepDisabled` left at 1.

- [ ] **Step 5: Commit**

```bash
git add src/install.rs src/lib.rs
git commit -m "feat: add daemon install/uninstall/status"
```

---

### Task 10: Foreground session

Runs a session for timer or command mode: acquire the assertion, open the hold connection (with degrade-on-missing-daemon), then either render the progress bar or wait on the child, cleaning up on every exit path.

**Files:**
- Create: `src/session.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `assertion::SleepAssertion`, `daemon::hold_connection`, `install::ensure_installed_interactive`, `progress::{progress_snapshot, format_progress_line}`.
- Produces:
  - `pub fn run_timer(total: std::time::Duration) -> anyhow::Result<()>`
  - `pub fn run_command(argv: Vec<String>) -> anyhow::Result<i32>` (returns child exit code).

- [ ] **Step 1: Register the module**

In `src/lib.rs` add `pub mod session;` (alphabetical order).

- [ ] **Step 2: Implement the session (moving progress rendering out of `main.rs`)**

Create `src/session.rs`:

```rust
use crate::assertion::SleepAssertion;
use crate::daemon::hold_connection;
use crate::install::ensure_installed_interactive;
use crate::progress::{format_progress_line, progress_snapshot};
use anyhow::{Context, Result};
use crossterm::{
    cursor::MoveToColumn,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    style::Print,
    terminal::{self, Clear, ClearType},
};
use std::io::{stdout, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::{Duration, Instant};

/// Acquire the idle-sleep assertion and, if possible, a daemon hold connection.
/// Returns the assertion (always) and the hold stream (None if degraded).
fn start_keepawake() -> Result<(SleepAssertion, Option<UnixStream>)> {
    let assertion = SleepAssertion::prevent_idle_sleep("espresso session")
        .context("failed to create idle-sleep assertion")?;

    let installed = ensure_installed_interactive().unwrap_or(false);
    let hold = if installed {
        match hold_connection() {
            Ok(stream) => Some(stream),
            Err(e) => {
                eprintln!("espresso: could not reach daemon ({e}); lid-close will still sleep");
                None
            }
        }
    } else {
        eprintln!("espresso: daemon not installed; idle-sleep prevented, but lid-close will still sleep");
        None
    };
    Ok((assertion, hold))
}

pub fn run_timer(total: Duration) -> Result<()> {
    let (_assertion, _hold) = start_keepawake()?;
    let _raw = RawModeGuard::enable()?;
    let start = Instant::now();
    let mut out = stdout();

    loop {
        let elapsed = start.elapsed().min(total);
        render(&mut out, elapsed, total)?;
        if elapsed >= total {
            break;
        }
        if should_quit()? {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    execute!(out, Print("\n"))?;
    Ok(())
    // _assertion drop releases the assertion; _hold drop closes the socket (daemon decrements).
}

pub fn run_command(argv: Vec<String>) -> Result<i32> {
    let (_assertion, _hold) = start_keepawake()?;
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .spawn()
        .with_context(|| format!("failed to spawn command: {}", argv[0]))?;
    let status = child.wait().context("failed to wait for command")?;
    Ok(status.code().unwrap_or(1))
}

fn should_quit() -> Result<bool> {
    if !event::poll(Duration::from_millis(0)).context("failed to poll terminal input")? {
        return Ok(false);
    }
    match event::read().context("failed to read terminal input")? {
        Event::Key(e)
            if e.kind == KeyEventKind::Press
                && matches!(e.code, KeyCode::Char('q') | KeyCode::Char('Q')) =>
        {
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn render(out: &mut impl Write, elapsed: Duration, total: Duration) -> Result<()> {
    let width = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
    let sample = progress_snapshot(elapsed, total, 1);
    let meta = 2 + sample.percentage_label.chars().count() + 2 + sample.remaining_label.chars().count();
    let bar_width = width.saturating_sub(meta).max(1);
    let snapshot = progress_snapshot(elapsed, total, bar_width);
    execute!(out, MoveToColumn(0), Clear(ClearType::CurrentLine), Print(format_progress_line(&snapshot)))?;
    out.flush()?;
    Ok(())
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> Result<Self> {
        terminal::enable_raw_mode().context("failed to enable terminal raw mode")?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds cleanly.

- [ ] **Step 4: Manual session smoke test (with daemon installed)**

```bash
sudo ./target/debug/espresso daemon install
./target/debug/espresso -t 5              # progress bar for 5s
# In another shell during the run:
./target/debug/espresso daemon status     # Active sessions: 1, SleepDisabled: 1
```
Expected: bar fills over 5s; during the run status shows 1 active session and `SleepDisabled: 1`; after it ends, status shows 0 and `SleepDisabled: 0` (within the 60s grace the daemon stays running but the flag is already 0).

- [ ] **Step 5: Commit**

```bash
git add src/session.rs src/lib.rs
git commit -m "feat: add foreground session (timer + command modes)"
```

---

### Task 11: Wire `main.rs` to the mode dispatch

Replace the caffeinate-wrapper `main.rs` with a thin dispatcher over `resolve_mode`.

**Files:**
- Modify: `src/main.rs` (full rewrite)

**Interfaces:**
- Consumes: `cli::{Cli, Mode, resolve_mode}`, `session::{run_timer, run_command}`, `daemon::run`, `install::{install, uninstall, print_status}`.

- [ ] **Step 1: Rewrite `src/main.rs`**

Replace the entire file with:

```rust
use anyhow::Result;
use chrono::Local;
use clap::Parser;
use espresso::cli::{resolve_mode, Cli, Mode};
use espresso::{daemon, install, session};

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            1
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32> {
    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("espresso only runs on macOS");

    let cli = Cli::parse();
    match resolve_mode(cli, Local::now()) {
        Ok(Mode::Timer(total)) => {
            session::run_timer(total)?;
            Ok(0)
        }
        Ok(Mode::Command(argv)) => session::run_command(argv),
        Ok(Mode::DaemonInstall) => {
            install::install()?;
            Ok(0)
        }
        Ok(Mode::DaemonUninstall) => {
            install::uninstall()?;
            Ok(0)
        }
        Ok(Mode::DaemonStatus) => {
            install::print_status()?;
            Ok(0)
        }
        Ok(Mode::DaemonRuntime) => {
            daemon::run()?;
            Ok(0)
        }
        Err(e) => {
            eprintln!("error: {e}");
            Ok(2)
        }
    }
}
```

- [ ] **Step 2: Verify it builds and all unit tests pass**

Run: `cargo build && cargo test`
Expected: build succeeds; all library unit tests pass; existing `tests/time_parsing.rs` and `tests/progress.rs` still pass.

- [ ] **Step 3: Verify argument errors**

Run:
```bash
./target/debug/espresso            # exit 2, prints usage
./target/debug/espresso -t 0       # exit 2, "-t countdown seconds must be greater than 0"
./target/debug/espresso -t abc     # exit 2, unsupported format
```
Expected: each prints the matching error to stderr and exits non-zero.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: dispatch main over resolved CLI mode"
```

---

### Task 12: End-to-end verification and docs

Full real-machine verification of the headline behavior, plus a short README note on the daemon and the `SleepDisabled` clobber semantics.

**Files:**
- Modify: `README.md` (create if absent)

**Interfaces:** none.

- [ ] **Step 1: Run the full test suite and build**

Run: `cargo test && cargo build --release`
Expected: all tests pass; release build succeeds.

- [ ] **Step 2: Verify idle-sleep prevention (assertion path)**

Run `./target/release/espresso -t 30` and, in another shell, `pmset -g assertions | grep -i PreventUserIdleSystemSleep`.
Expected: the assertion is present for the duration and gone afterwards.

- [ ] **Step 3: Verify lid-close screen-off without sleep on battery**

With the daemon installed, unplug power, run `./target/release/espresso -t 120`, close the lid for ~10s, then reopen.
Expected: the screen turns off on close (within ~2s) but the machine does not sleep (SSH session stays alive / work continues); `pmset -g | grep SleepDisabled` reads `1` during the session and `0` after it ends.

- [ ] **Step 4: Verify command mode and exit-code propagation**

Run:
```bash
./target/release/espresso -- bash -c 'sleep 2; exit 7'; echo "exit=$?"
```
Expected: `exit=7`; during the 2s, `espresso daemon status` shows 1 active session.

- [ ] **Step 5: Write the README note**

Create or update `README.md` with a section:

```markdown
## Daemon

`espresso` keeps the Mac awake with a per-process idle-sleep assertion. To also
keep it awake with the lid closed (screen off, no sleep, even on battery), a small
root helper is required:

    sudo espresso daemon install     # one time
    espresso daemon status
    sudo espresso daemon uninstall

The helper is launched on demand by launchd and self-exits when no sessions remain.

### SleepDisabled is global

Lid-closed wake relies on the kernel `SleepDisabled` flag, which is a single global
switch with no owner. espresso sets it while any session is active and clears it when
the last one ends. This is last-writer-wins: it can override a `SleepDisabled` you set
manually via `pmset` or that another app set, and vice versa.
```

- [ ] **Step 6: Commit**

```bash
git add README.md
git commit -m "docs: document espresso daemon and SleepDisabled semantics"
```

---

## Self-Review

**Spec coverage:**
- Idle-sleep prevention (battery + AC) → Task 5 (assertion) + Task 10 (held per session).
- Lid-close screen-off without sleep → Task 6 (SleepDisabled), Task 7 (lid), Task 8 (`pmset displaysleepnow` on edge).
- `-t` countdown vs target, other values error → Task 1.
- `-t`/command mutual exclusion with `-t` ignored → Task 2.
- Positional command like caffeinate → Task 2 (dispatch) + Task 10 (`run_command`).
- Progress bar only in timer mode → Task 10 (`run_timer` renders; `run_command` does not).
- No-args error → Task 2 (`CliError::NoArgs`) + Task 11 (exit 2).
- Auto-start daemon on first use + explicit install → Task 9 (`install`, `ensure_installed_interactive`) + Task 10 (invoked from `start_keepawake`).
- disablesleep off when time up / process exits, daemon idle 60s then self-exit → Task 4 (state machine) + Task 8 (coordinator, grace timer, `Exit`).
- Async correctness (single coordinator, generation-cancelled grace, startup reset) → Task 4 + Task 8.
- `daemon status` two-layer output + ownership labeling → Task 9.
- Unix socket + launchd Sockets activation, Hold/Query intents → Task 3 (protocol) + Task 8 (activation, connection handling).
- Graceful degradation when daemon absent/declined → Task 10 (`start_keepawake`).
- Testing strategy (pure logic unit-tested; FFI smoke-tested) → Tasks 1–4 unit tests, Tasks 5–9 smoke tests, Task 12 e2e.

**Placeholder scan:** No `TBD`/`TODO`/"handle edge cases"/"similar to Task N"; every code step contains complete code.

**Type consistency:** `parse_timer_arg`/`TimerArgError` (T1) used in `resolve_mode` (T2); `Cli`/`Mode`/`resolve_mode` (T2) used in `main` (T11); `ClientMsg`/`ServerMsg`/`StatusInfo`/`SOCKET_PATH` (T3) used in `daemon` (T8) and `install` (T9); `RefcountState`/`Action` (T4) used in `daemon` (T8); `SleepAssertion` (T5), `set_sleep_disabled`/`read_sleep_disabled` (T6), `lid_closed` (T7) used in `daemon`/`session`/`install`; `hold_connection`/`query_status`/`daemon::run` (T8) used in `session`/`install`/`main`; `run_timer`/`run_command` (T10) used in `main`. Names and signatures match across tasks.
