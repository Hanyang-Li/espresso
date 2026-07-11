use anyhow::Result;
use chrono::Local;
use clap::{CommandFactory, Parser};
use espresso::cli::{Cli, Mode, resolve_mode};
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
        Ok(Mode::Help) => {
            // No args given: show help but signal misuse. (`--help` is handled
            // by clap itself → stdout, exit 0.)
            eprint!("{}", Cli::command().render_help());
            eprintln!();
            Ok(1)
        }
        Err(e) => {
            eprintln!("error: {e}");
            Ok(2)
        }
    }
}
