use espresso::progress::{format_progress_line, progress_snapshot};
use std::time::Duration;

#[test]
fn progress_bar_uses_reference_fill_style() {
    let snapshot = progress_snapshot(Duration::from_secs(35), Duration::from_secs(100), 10);

    assert_eq!(snapshot.bar, "████░░░░░░");
    assert_eq!(snapshot.percentage_label, "35%");
    assert_eq!(snapshot.remaining_label, "1m 5s remaining");
}

#[test]
fn progress_clamps_at_complete() {
    let snapshot = progress_snapshot(Duration::from_secs(120), Duration::from_secs(100), 10);

    assert_eq!(snapshot.bar, "██████████");
    assert_eq!(snapshot.percentage_label, "100%");
    assert_eq!(snapshot.remaining_label, "0s remaining");
}

#[test]
fn progress_width_is_at_least_one() {
    let snapshot = progress_snapshot(Duration::from_secs(0), Duration::from_secs(10), 0);

    assert_eq!(snapshot.bar, "░");
}

#[test]
fn formats_hours_minutes_and_seconds_remaining() {
    let snapshot = progress_snapshot(Duration::from_secs(0), Duration::from_secs(3661), 10);

    assert_eq!(snapshot.remaining_label, "1h 1m 1s remaining");
}

#[test]
fn formats_single_line_progress() {
    let snapshot = progress_snapshot(Duration::from_secs(35), Duration::from_secs(100), 10);

    assert_eq!(
        format_progress_line(&snapshot),
        "████░░░░░░  35%  1m 5s remaining"
    );
}
