use crate::time::{TimerArgError, parse_timer_arg};
use chrono::{DateTime, Local};
use clap::{Parser, Subcommand};
use std::fmt;
use std::time::Duration;

const AFTER_HELP: &str = "\
Modes without a subcommand:
  espresso -t <secs|time>    keep awake for a countdown or clock time
  espresso <command> ...     keep awake while the command runs

The 'daemon' subcommand adds lid-closed keep-awake (screen off, no sleep,
even on battery). Without it, only idle-sleep is prevented.

Examples:
  espresso -t 1800
  espresso -t 17:00
  espresso -- npm run build";

/// espresso — keep this Mac awake; closing the lid only turns off the screen.
#[derive(Parser, Debug)]
#[command(name = "espresso", version, after_help = AFTER_HELP)]
pub struct Cli {
    /// Countdown seconds, or a target time (e.g. 1800, 17:00).
    #[arg(short = 't', long = "time")]
    pub time: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Manage the lid-closed keep-awake helper (install/uninstall/status).
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// launchd runtime entry point (internal).
    #[command(name = "__daemon", hide = true)]
    Runtime,

    /// A command to run while keeping the Mac awake.
    #[command(external_subcommand)]
    Run(Vec<String>),
}

#[derive(Subcommand, Debug)]
pub enum DaemonAction {
    /// Install the privileged helper (requires sudo).
    Install,
    /// Remove the privileged helper (requires sudo).
    Uninstall,
    /// Show helper and keep-awake status.
    Status,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    Timer(Duration),
    Command(Vec<String>),
    DaemonInstall,
    DaemonUninstall,
    DaemonStatus,
    DaemonRuntime,
    Help,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    Timer(TimerArgError),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timer(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CliError {}

pub fn resolve_mode(cli: Cli, now: DateTime<Local>) -> Result<Mode, CliError> {
    match cli.command {
        // A positional command is present: -t is ignored (no error).
        Some(Commands::Run(argv)) => Ok(Mode::Command(argv)),
        Some(Commands::Daemon { action }) => Ok(match action {
            DaemonAction::Install => Mode::DaemonInstall,
            DaemonAction::Uninstall => Mode::DaemonUninstall,
            DaemonAction::Status => Mode::DaemonStatus,
        }),
        Some(Commands::Runtime) => Ok(Mode::DaemonRuntime),
        None => match cli.time {
            Some(v) => parse_timer_arg(&v, now).map(Mode::Timer).map_err(CliError::Timer),
            // No -t and no command: show help (main treats this as misuse: exit 1).
            None => Ok(Mode::Help),
        },
    }
}

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
    fn no_args_shows_help() {
        assert_eq!(resolve_mode(cli(&[]), now()), Ok(Mode::Help));
    }

    #[test]
    fn t_seconds_is_timer() {
        assert_eq!(
            resolve_mode(cli(&["-t", "60"]), now()),
            Ok(Mode::Timer(Duration::from_secs(60)))
        );
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
    fn daemon_install_and_uninstall_subcommands() {
        assert_eq!(resolve_mode(cli(&["daemon", "install"]), now()), Ok(Mode::DaemonInstall));
        assert_eq!(
            resolve_mode(cli(&["daemon", "uninstall"]), now()),
            Ok(Mode::DaemonUninstall)
        );
    }

    #[test]
    fn daemon_unknown_subcommand_is_rejected_by_clap() {
        assert!(<Cli as clap::Parser>::try_parse_from(["espresso", "daemon", "frobnicate"]).is_err());
    }

    #[test]
    fn daemon_without_action_is_rejected_by_clap() {
        assert!(<Cli as clap::Parser>::try_parse_from(["espresso", "daemon"]).is_err());
    }

    #[test]
    fn hidden_daemon_runtime() {
        assert_eq!(resolve_mode(cli(&["__daemon"]), now()), Ok(Mode::DaemonRuntime));
    }
}
