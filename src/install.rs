//! LaunchDaemon lifecycle: writing/loading the plist that gives `launchd`
//! socket activation over `/var/run/espresso.sock`, and the two-layer
//! `daemon status` report (install/registration facts, then a live `Query`).

use crate::daemon::query_status;
use crate::ipc::SOCKET_PATH;
use crate::power::read_sleep_disabled;
use anyhow::{Context, Result, bail};
use std::io::Write;
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

/// Renders `espresso daemon status`. Two layers:
///   1. Install/registration facts — plist presence and `launchctl print`
///      — which never connect to the daemon and so never affect it.
///   2. A live `Query` connection (never a `Hold`, so it does not itself
///      keep the Mac awake or bump the refcount).
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
            if info.sleep_disabled && info.refcount > 0 {
                println!(
                    "  SleepDisabled:    1     (espresso: {} active sessions)",
                    info.refcount
                );
            } else {
                report_flag_without_daemon()?;
            }
            println!(
                "  Lid:              {}",
                if info.lid_closed { "closed" } else { "open" }
            );
            println!("  Socket:           {SOCKET_PATH} (present)");
            println!(
                "  Version:          daemon {} / cli {}",
                info.version,
                env!("CARGO_PKG_VERSION")
            );
        }
        None => {
            println!("  Running:          no (idle, starts on demand)");
            report_flag_without_daemon()?;
        }
    }
    Ok(())
}

/// The `SleepDisabled` line to use when there is no running daemon holding
/// sessions (either the daemon is idle, or it reported `refcount == 0`):
/// falls back to reading the raw system power setting, since something
/// other than espresso could have set it.
fn report_flag_without_daemon() -> Result<()> {
    if read_sleep_disabled()? {
        println!("  SleepDisabled:    1     (set by another process, not espresso)");
    } else {
        println!("  SleepDisabled:    0");
    }
    Ok(())
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
