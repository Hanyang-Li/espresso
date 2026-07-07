use chrono::{DateTime, Datelike, Local, LocalResult, NaiveDateTime, NaiveTime, TimeZone};
use std::fmt;

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
