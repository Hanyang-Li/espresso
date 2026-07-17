//! LaunchDaemon lifecycle: writing/loading the plist that gives `launchd`
//! socket activation over `/var/run/espresso.sock`, and the two-layer
//! `daemon status` report (install/registration facts, then a live `Query`).

use crate::daemon::query_status;
use crate::ipc::{SOCKET_PATH, StatusInfo};
use crate::power::read_sleep_disabled;
use crate::ui::{self, Cell};
use anyhow::{Context, Result, bail};
use std::io::{IsTerminal, Write};
use std::process::Command;

pub const LABEL: &str = "local.espresso.daemon";
pub const PLIST_PATH: &str = "/Library/LaunchDaemons/local.espresso.daemon.plist";

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

/// Whether the LaunchDaemon plist is present on disk. Pure filesystem check,
/// no `launchctl` or socket I/O involved.
pub fn is_installed() -> bool {
    std::path::Path::new(PLIST_PATH).exists()
}

fn require_root() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("this command requires root; run: sudo espresso daemon install");
    }
    Ok(())
}

/// Writes the LaunchDaemon plist and bootstraps it into the system domain.
/// Requires root.
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

/// Tears the LaunchDaemon out of the system domain and removes the plist
/// (and any leftover socket file). Requires root.
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

/// Renders `espresso daemon status` without ever socket-activating the daemon.
/// Install/registration/running facts come from the plist file and
/// `launchctl print` (neither touches the socket). Only if launchd reports a
/// live instance do we open a `Query` connection — which reaches the existing
/// daemon (never spawns a new one) and never bumps the refcount.
pub fn print_status() -> Result<()> {
    let use_color = std::io::stdout().is_terminal();
    let cli_version = env!("CARGO_PKG_VERSION");

    let installed = is_installed();
    let state = if installed {
        launchd_state()
    } else {
        LaunchdState { registered: false, running: false, pid: None }
    };
    // Only touch the socket when launchd already reports a live instance.
    let info: Option<StatusInfo> = if state.running {
        query_status().ok().flatten()
    } else {
        None
    };
    let sleep_disabled = read_sleep_disabled().unwrap_or(false);

    // Terminal width budget for the shared inner box width.
    let term = crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
    let max_inner = term.saturating_sub(4).max(20);

    // ---- Status box lines ----
    let status_label = 13; // width("SleepDisabled")
    let status_lines = vec![
        ui::join(&[
            Cell::plain(format!("{:<status_label$}", "SleepDisabled")),
            Cell::plain("   "),
            ui::yesno(sleep_disabled, use_color),
        ]),
        {
            let mut parts = vec![
                Cell::plain(format!("{:<status_label$}", "Running")),
                Cell::plain("   "),
                ui::yesno(state.running, use_color),
            ];
            if let Some(pid) = state.pid.filter(|_| state.running) {
                parts.push(Cell::plain(format!("   pid {pid}")));
            }
            ui::join(&parts)
        },
    ];

    // ---- Active Sessions box lines (empty until daemon tracks them) ----
    let sessions = info.as_ref().map(|i| i.sessions.as_slice()).unwrap_or(&[]);
    let session_count = sessions.len();
    // Natural (untruncated) widths help size the shared inner width.
    let pid_col = sessions
        .iter()
        .map(|s| ui::display_width(&s.pid.to_string()))
        .max()
        .unwrap_or(0);

    // ---- Infos box lines ----
    let infos_label = 10; // width("Registered")
    let version_value = match &info {
        Some(i) => format!("daemon {} / cli {}", i.version, cli_version),
        None => format!("cli {cli_version}"),
    };
    let mut infos_specs: Vec<Cell> = vec![ui::join(&[
        Cell::plain(format!("{:<infos_label$}", "Version")),
        Cell::plain("   "),
        Cell::plain(version_value),
    ])];
    // Installed row (yes/no + plist path).
    infos_specs.push(info_row(
        infos_label,
        "Installed",
        installed,
        if installed { Some(PLIST_PATH.to_string()) } else { None },
        use_color,
        max_inner,
    ));
    // Registered row (yes/no + launchd label), only meaningful when installed.
    if installed {
        infos_specs.push(info_row(
            infos_label,
            "Registered",
            state.registered,
            Some(format!("system/{LABEL}")),
            use_color,
            max_inner,
        ));
    }

    // ---- Compute shared inner width across all boxes ----
    let session_natural = sessions
        .iter()
        .map(|s| pid_col + 2 + ui::display_width(&s.command) + 2 + ui::display_width(&ui::format_uptime(s.uptime_secs)))
        .max()
        .unwrap_or(0);
    let title_active = format!("Active Sessions ({session_count})");
    let mut inner = 0usize;
    for c in status_lines.iter().chain(infos_specs.iter()) {
        inner = inner.max(c.width());
    }
    inner = inner.max(session_natural);
    // Titles must fit: inner >= width(title) + 1.
    for t in ["Status", "Infos", title_active.as_str()] {
        inner = inner.max(ui::display_width(t) + 1);
    }
    inner = inner.min(max_inner);

    // ---- Lay out session rows to the shared inner width ----
    let session_lines: Vec<Cell> = sessions
        .iter()
        .map(|s| {
            let prefix = format!("{:<pid_col$}  ", s.pid);
            let uptime = ui::format_uptime(s.uptime_secs);
            let reserved = ui::display_width(&prefix) + ui::display_width(&uptime) + 1;
            let cmd_budget = inner.saturating_sub(reserved);
            let cmd = ui::truncate(&s.command, cmd_budget);
            let used = ui::display_width(&prefix) + ui::display_width(&cmd) + ui::display_width(&uptime);
            let gap = inner.saturating_sub(used);
            Cell::plain(format!("{prefix}{cmd}{}{uptime}", " ".repeat(gap)))
        })
        .collect();

    // ---- Emit ----
    let mut out = String::new();
    out.push_str(&ui::render_box("Status", &status_lines, inner));
    if session_count > 0 {
        out.push_str(&ui::render_box(&title_active, &session_lines, inner));
    }
    out.push_str(&ui::render_box("Infos", &infos_specs, inner));
    print!("{out}");

    if !installed {
        println!("→ run `espresso daemon install` (needs sudo) to enable lid-closed keep-awake");
    }
    Ok(())
}

/// One Infos row: `label` padded to `label_col`, a yes/no token, then an
/// optional path/value truncated to fit the terminal budget.
fn info_row(
    label_col: usize,
    label: &str,
    flag: bool,
    value: Option<String>,
    use_color: bool,
    max_inner: usize,
) -> Cell {
    let mut parts = vec![
        Cell::plain(format!("{label:<label_col$}")),
        Cell::plain("   "),
        ui::yesno(flag, use_color),
    ];
    if let Some(v) = value {
        // yesno is "yes"(3)/"no"(2); pad so the value column starts evenly.
        let gap = if flag { "   " } else { "    " };
        let prefix_w = label_col + 3 + if flag { 3 } else { 2 } + gap.len();
        let budget = max_inner.saturating_sub(prefix_w);
        parts.push(Cell::plain(gap));
        parts.push(Cell::plain(ui::truncate(&v, budget)));
    }
    ui::join(&parts)
}

struct LaunchdState {
    registered: bool,
    running: bool,
    pid: Option<u32>,
}

/// Reads `launchctl print system/<LABEL>`, which queries launchd's job registry
/// and does NOT open the socket — so it never socket-activates the daemon.
/// `registered` = the job is loaded; `running` = an instance is currently up
/// (from a `state = running` line or a parsed `pid`); `pid` when available.
fn launchd_state() -> LaunchdState {
    match Command::new("launchctl")
        .args(["print", &format!("system/{LABEL}")])
        .output()
    {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut running = false;
            let mut pid = None;
            for line in text.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("pid = ") {
                    if let Ok(n) = v.trim().parse::<u32>() {
                        pid = Some(n);
                        running = true;
                    }
                } else if line == "state = running" {
                    running = true;
                }
            }
            LaunchdState {
                registered: true,
                running,
                pid,
            }
        }
        _ => LaunchdState {
            registered: false,
            running: false,
            pid: None,
        },
    }
}

/// If the daemon is not installed, prompts on stderr and (on yes) runs
/// `sudo <current_exe> daemon install`. Returns whether the daemon is
/// installed afterwards, whichever path was taken.
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
    let program = std::env::current_exe()
        .context("failed to resolve current executable path")?
        .to_string_lossy()
        .into_owned();
    let status = Command::new("sudo")
        .args([&program, "daemon", "install"])
        .status()
        .context("failed to run sudo espresso daemon install")?;
    Ok(status.success() && is_installed())
}
