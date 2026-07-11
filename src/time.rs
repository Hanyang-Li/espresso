use chrono::{DateTime, Datelike, Local, LocalResult, NaiveDateTime, NaiveTime, TimeZone};
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseTargetError {
    UnsupportedFormat(String),
    InvalidLocalTime(String),
    NotFuture { target: String, now: String },
}

impl fmt::Display for ParseTargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat(input) => write!(f, "unsupported time format: {input}"),
            Self::InvalidLocalTime(input) => write!(f, "not a valid local time: {input}"),
            Self::NotFuture { target, now } => {
                write!(f, "target time must be later than now: {target} <= {now}")
            }
        }
    }
}

impl std::error::Error for ParseTargetError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimerArgError {
    Empty,
    NotPositive(String),
    Unparseable(String),
    Target(ParseTargetError),
}

impl fmt::Display for TimerArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "-t requires a value"),
            Self::NotPositive(v) => write!(f, "-t countdown seconds must be greater than 0: {v}"),
            Self::Unparseable(v) => write!(f, "-t value is not a valid number: {v}"),
            Self::Target(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TimerArgError {}

pub fn parse_timer_arg(value: &str, now: DateTime<Local>) -> Result<Duration, TimerArgError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(TimerArgError::Empty);
    }

    if trimmed.bytes().all(|b| b.is_ascii_digit()) {
        let secs: u64 = trimmed
            .parse()
            .map_err(|_| TimerArgError::Unparseable(trimmed.to_string()))?;
        if secs == 0 {
            return Err(TimerArgError::NotPositive(trimmed.to_string()));
        }
        return Ok(Duration::from_secs(secs));
    }

    if let Ok(n) = trimmed.parse::<i64>() {
        if n <= 0 {
            return Err(TimerArgError::NotPositive(trimmed.to_string()));
        }
    }

    match parse_target_time(trimmed, now) {
        Ok(target) => {
            let millis = target.signed_duration_since(now).num_milliseconds().max(1) as u64;
            Ok(Duration::from_millis(millis))
        }
        Err(e) => Err(TimerArgError::Target(e)),
    }
}

pub fn parse_target_time(
    input: &str,
    now: DateTime<Local>,
) -> Result<DateTime<Local>, ParseTargetError> {
    let input = input.trim();
    let naive = parse_naive_target(input, now)?;
    let target = resolve_local_time(input, naive, now)?;

    if target <= now {
        return Err(ParseTargetError::NotFuture {
            target: target.to_rfc3339(),
            now: now.to_rfc3339(),
        });
    }

    Ok(target)
}

fn parse_naive_target(
    input: &str,
    now: DateTime<Local>,
) -> Result<NaiveDateTime, ParseTargetError> {
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(input, format) {
            return Ok(value);
        }
    }

    let current_year = now.year();
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        let with_year = format!("{current_year}-{input}");
        if let Ok(value) = NaiveDateTime::parse_from_str(&with_year, format) {
            return Ok(value);
        }
    }

    for format in ["%H:%M:%S", "%H:%M"] {
        if let Ok(time) = NaiveTime::parse_from_str(input, format) {
            return Ok(now.date_naive().and_time(time));
        }
    }

    Err(ParseTargetError::UnsupportedFormat(input.to_string()))
}

fn resolve_local_time(
    input: &str,
    naive: NaiveDateTime,
    now: DateTime<Local>,
) -> Result<DateTime<Local>, ParseTargetError> {
    match Local.from_local_datetime(&naive) {
        LocalResult::Single(value) => Ok(value),
        LocalResult::Ambiguous(earliest, latest) => {
            if earliest > now {
                Ok(earliest)
            } else {
                Ok(latest)
            }
        }
        LocalResult::None => Err(ParseTargetError::InvalidLocalTime(input.to_string())),
    }
}

#[cfg(test)]
mod timer_arg_tests {
    use super::*;
    use chrono::TimeZone;
    use std::time::Duration;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 7, 11, 12, 0, 0).single().unwrap()
    }

    #[test]
    fn positive_integer_is_countdown_seconds() {
        assert_eq!(parse_timer_arg("3600", now()), Ok(Duration::from_secs(3600)));
    }

    #[test]
    fn zero_is_rejected() {
        assert_eq!(parse_timer_arg("0", now()), Err(TimerArgError::NotPositive("0".into())));
    }

    #[test]
    fn negative_is_rejected() {
        assert_eq!(parse_timer_arg("-5", now()), Err(TimerArgError::NotPositive("-5".into())));
    }

    #[test]
    fn empty_is_rejected() {
        assert_eq!(parse_timer_arg("   ", now()), Err(TimerArgError::Empty));
    }

    #[test]
    fn clock_time_is_target() {
        // 13:00 is one hour after the fixed now of 12:00.
        assert_eq!(parse_timer_arg("13:00", now()), Ok(Duration::from_secs(3600)));
    }

    #[test]
    fn junk_is_unsupported_format() {
        assert!(matches!(parse_timer_arg("abc", now()), Err(TimerArgError::Target(_))));
    }
}
