use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use crossterm::{
    cursor::MoveToColumn,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    style::Print,
    terminal::{self, Clear, ClearType},
};
use espresso::{
    progress::{format_progress_line, progress_snapshot},
    time::parse_target_time,
};
use std::{
    env,
    io::{Write, stdout},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let input = collect_time_argument(env::args().skip(1))?;
    let now = Local::now();
    let target = parse_target_time(&input, now)?;
    let total_secs = duration_seconds(now, target);
    let total = Duration::from_secs(total_secs);

    let mut child = spawn_caffeinate(total_secs)?;
    let _raw_mode = RawModeGuard::enable()?;
    let start = Instant::now();
    let mut stdout = stdout();

    loop {
        let elapsed = start.elapsed().min(total);
        render_progress(&mut stdout, elapsed, total)?;

        if elapsed >= total {
            break;
        }

        if child
            .try_wait()
            .context("failed to inspect caffeinate process")?
            .is_some()
        {
            break;
        }

        if should_quit()? {
            stop_child(&mut child);
            execute!(stdout, Print("\n"))?;
            return Ok(());
        }

        thread::sleep(Duration::from_millis(200));
    }

    stop_child(&mut child);
    execute!(stdout, Print("\n"))?;
    Ok(())
}

fn collect_time_argument(args: impl Iterator<Item = String>) -> Result<String> {
    let values = args.collect::<Vec<_>>();
    if values.is_empty() {
        bail!(
            "usage: espresso <time>\nformats: HH:mm, HH:mm:ss, MM-dd HH:mm, MM-dd HH:mm:ss, yyyy-MM-dd HH:mm, yyyy-MM-dd HH:mm:ss"
        );
    }

    Ok(values.join(" "))
}

fn duration_seconds(now: DateTime<Local>, target: DateTime<Local>) -> u64 {
    let millis = target.signed_duration_since(now).num_milliseconds().max(1) as u64;
    millis.div_ceil(1000)
}

fn spawn_caffeinate(seconds: u64) -> Result<Child> {
    Command::new("caffeinate")
        .args(["-dimsu", "-t", &seconds.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context(
            "failed to start caffeinate; this command requires macOS and the caffeinate command",
        )
}

fn should_quit() -> Result<bool> {
    if !event::poll(Duration::from_millis(0)).context("failed to poll terminal input")? {
        return Ok(false);
    }

    match event::read().context("failed to read terminal input")? {
        Event::Key(event)
            if event.kind == KeyEventKind::Press
                && matches!(event.code, KeyCode::Char('q') | KeyCode::Char('Q')) =>
        {
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn render_progress(stdout: &mut impl Write, elapsed: Duration, total: Duration) -> Result<()> {
    let terminal_width = terminal::size()
        .map(|(width, _)| width as usize)
        .unwrap_or(80);
    let sample = progress_snapshot(elapsed, total, 1);
    let metadata_width =
        2 + sample.percentage_label.chars().count() + 2 + sample.remaining_label.chars().count();
    let bar_width = terminal_width.saturating_sub(metadata_width).max(1);
    let snapshot = progress_snapshot(elapsed, total, bar_width);
    let line = format_progress_line(&snapshot);

    execute!(
        stdout,
        MoveToColumn(0),
        Clear(ClearType::CurrentLine),
        Print(line)
    )?;
    stdout.flush()?;
    Ok(())
}

fn stop_child(child: &mut Child) {
    match child.try_wait() {
        Ok(Some(_)) => {}
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
        }
        Err(_) => {}
    }
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
