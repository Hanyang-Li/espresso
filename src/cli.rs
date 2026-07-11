use crate::time::{parse_timer_arg, TimerArgError};
use chrono::{DateTime, Local};
use clap::Parser;
use std::fmt;
use std::time::Duration;

const AFTER_HELP: &str = "\
Modes:
  espresso -t <secs|time>    countdown, or run until a clock time
  espresso <command> ...     keep awake while the command runs
  espresso daemon install    install the helper (sudo)
  espresso daemon uninstall  remove the helper (sudo)
  espresso daemon status     show helper and keep-awake status
  espresso                   show this help

The daemon helper adds lid-closed keep-awake: screen off, no sleep,
even on battery. Without it, only idle-sleep is prevented.

Examples:
  espresso -t 1800
  espresso -t 17:00
  espresso -- npm run build";

/// espresso — keep this Mac awake; closing the lid only turns off the screen.
#[derive(Parser, Debug)]
#[command(
    name = "espresso",
    version,
    trailing_var_arg = true,
    after_help = AFTER_HELP,
)]
pub struct Cli {
    /// Countdown seconds, or a target time (e.g. 1800, 17:00).
    #[arg(short = 't', long = "time")]
    pub time: Option<String>,

    /// A daemon subcommand, or the command to run while active.
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
    Help,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    UnknownDaemonSub(String),
    Timer(TimerArgError),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
        // No -t and no positional command: show help (not an error).
        None => Ok(Mode::Help),
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
