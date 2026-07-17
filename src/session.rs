//! Foreground session for the two `espresso` run modes: a timer session that
//! renders a progress bar until it elapses or the user presses `q`, and a
//! command session that runs a child process to completion while keeping the
//! Mac awake.
//!
//! Both modes acquire a [`SleepAssertion`] (hard error if that fails) and
//! then attempt to register with the daemon via [`hold_connection`] so that
//! lid-close is also covered; if the daemon is not installed or unreachable,
//! the session degrades gracefully (idle-sleep is still prevented, but the
//! lid can still put the machine to sleep) rather than failing outright.

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
use std::io::{Write, stdout};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::{Duration, Instant};

/// The invocation as the user typed it, argv[0] normalized to `espresso`
/// (e.g. "espresso -- sleep 100", "espresso -t 1800"). Newlines stripped so
/// it stays on one IPC line.
fn current_command() -> String {
    let rest: Vec<String> = std::env::args().skip(1).collect();
    let joined = if rest.is_empty() {
        "espresso".to_string()
    } else {
        format!("espresso {}", rest.join(" "))
    };
    joined.replace(['\n', '\r'], " ")
}

/// Acquire the idle-sleep assertion and, if possible, a daemon hold connection.
/// Returns the assertion (always) and the hold stream (None if degraded).
fn start_keepawake() -> Result<(SleepAssertion, Option<UnixStream>)> {
    let assertion = SleepAssertion::prevent_idle_sleep("espresso session")
        .context("failed to create idle-sleep assertion")?;

    let installed = ensure_installed_interactive().unwrap_or(false);
    let hold = if installed {
        match hold_connection(&current_command()) {
            Ok(stream) => Some(stream),
            Err(e) => {
                eprintln!("espresso: could not reach daemon ({e}); lid-close will still sleep");
                None
            }
        }
    } else {
        eprintln!(
            "espresso: daemon not installed; idle-sleep prevented, but lid-close will still sleep"
        );
        None
    };
    Ok((assertion, hold))
}

/// Renders a single-line progress bar in raw mode until `total` elapses or
/// the user presses `q`/`Q`.
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

/// Spawns `argv[0]` with `argv[1..]`, inheriting stdio, and waits for it to
/// finish. Returns the child's exit code (or 1 if it terminated by signal).
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
    let meta =
        2 + sample.percentage_label.chars().count() + 2 + sample.remaining_label.chars().count();
    let bar_width = width.saturating_sub(meta).max(1);
    let snapshot = progress_snapshot(elapsed, total, bar_width);
    execute!(
        out,
        MoveToColumn(0),
        Clear(ClearType::CurrentLine),
        Print(format_progress_line(&snapshot))
    )?;
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
