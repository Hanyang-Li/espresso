use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressSnapshot {
    pub bar: String,
    pub percentage_label: String,
    pub remaining_label: String,
}

pub fn progress_snapshot(elapsed: Duration, total: Duration, width: usize) -> ProgressSnapshot {
    let width = width.max(1);
    let total_secs = total.as_secs();
    let elapsed_secs = elapsed.as_secs().min(total_secs);
    let percentage = if total_secs == 0 {
        100.0
    } else {
        (elapsed_secs as f64 / total_secs as f64) * 100.0
    };
    let filled_count = (((percentage / 100.0) * width as f64).round() as usize).min(width);
    let remaining = Duration::from_secs(total_secs.saturating_sub(elapsed_secs));

    let mut bar = String::with_capacity(width);
    for index in 0..width {
        if index < filled_count {
            bar.push('█');
        } else {
            bar.push('░');
        }
    }

    ProgressSnapshot {
        bar,
        percentage_label: format!("{}%", percentage.round() as u64),
        remaining_label: format!("{} remaining", format_duration(remaining)),
    }
}

pub fn format_progress_line(snapshot: &ProgressSnapshot) -> String {
    format!(
        "{}  {}  {}",
        snapshot.bar, snapshot.percentage_label, snapshot.remaining_label
    )
}

fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m {seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}
