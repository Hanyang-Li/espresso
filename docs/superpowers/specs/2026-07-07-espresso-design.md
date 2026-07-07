# Espresso CLI Design

## Goal

Create a Rust CLI named `espresso` that wraps macOS `caffeinate` with fixed flags and a visible countdown.

## Behavior

`espresso <time>` accepts one required time argument in these local-time formats:

- `HH:mm`
- `HH:mm:ss`
- `MM-dd HH:mm`
- `MM-dd HH:mm:ss`
- `yyyy-MM-dd HH:mm`
- `yyyy-MM-dd HH:mm:ss`

Time-only inputs use today's local date. Month-day inputs use the current local year. The parsed time must be later than the current local time; otherwise the command exits with an error.

The command starts `caffeinate -dimsu -t <seconds>`, where `<seconds>` is the whole number of seconds between now and the parsed future time.

While running, it displays a single-line progress bar styled after `agent-limit`: filled `█`, unfilled `░`, followed by percentage and remaining time. The progress bar uses the terminal default foreground color. Pressing `q` terminates the child process and exits early.

## Architecture

- `src/time.rs`: parse and validate local target times.
- `src/progress.rs`: format progress bars and remaining-time labels.
- `src/main.rs`: CLI argument handling, child process management, terminal raw mode, key polling, and screen refresh.

## Testing

Unit tests cover supported time formats, rejection of non-future times, progress formatting, and remaining-time labels. Final verification runs `cargo test` and `cargo build`.
