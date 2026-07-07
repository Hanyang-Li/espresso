use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use espresso::time::{ParseTargetError, parse_target_time};

fn local(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> DateTime<Local> {
    Local
        .with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .expect("test time must be a valid local time")
}

fn assert_components(
    actual: DateTime<Local>,
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) {
    assert_eq!(actual.year(), year);
    assert_eq!(actual.month(), month);
    assert_eq!(actual.day(), day);
    assert_eq!(actual.hour(), hour);
    assert_eq!(actual.minute(), minute);
    assert_eq!(actual.second(), second);
}

#[test]
fn parses_hour_minute_as_today() {
    let now = local(2026, 7, 7, 9, 30, 0);

    let target = parse_target_time("10:05", now).unwrap();

    assert_components(target, 2026, 7, 7, 10, 5, 0);
}

#[test]
fn parses_hour_minute_second_as_today() {
    let now = local(2026, 7, 7, 9, 30, 0);

    let target = parse_target_time("10:05:45", now).unwrap();

    assert_components(target, 2026, 7, 7, 10, 5, 45);
}

#[test]
fn parses_month_day_with_current_year() {
    let now = local(2026, 7, 7, 9, 30, 0);

    let target = parse_target_time("07-08 01:02", now).unwrap();

    assert_components(target, 2026, 7, 8, 1, 2, 0);
}

#[test]
fn parses_month_day_with_seconds_and_current_year() {
    let now = local(2026, 7, 7, 9, 30, 0);

    let target = parse_target_time("07-08 01:02:03", now).unwrap();

    assert_components(target, 2026, 7, 8, 1, 2, 3);
}

#[test]
fn parses_full_date_without_seconds() {
    let now = local(2026, 7, 7, 9, 30, 0);

    let target = parse_target_time("2026-07-08 01:02", now).unwrap();

    assert_components(target, 2026, 7, 8, 1, 2, 0);
}

#[test]
fn parses_full_date_with_seconds() {
    let now = local(2026, 7, 7, 9, 30, 0);

    let target = parse_target_time("2026-07-08 01:02:03", now).unwrap();

    assert_components(target, 2026, 7, 8, 1, 2, 3);
}

#[test]
fn rejects_target_equal_to_now() {
    let now = local(2026, 7, 7, 9, 30, 0);

    let err = parse_target_time("09:30", now).unwrap_err();

    assert!(matches!(err, ParseTargetError::NotFuture { .. }));
}

#[test]
fn rejects_target_before_now() {
    let now = local(2026, 7, 7, 9, 30, 0);

    let err = parse_target_time("09:29:59", now).unwrap_err();

    assert!(matches!(err, ParseTargetError::NotFuture { .. }));
}

#[test]
fn rejects_unsupported_format() {
    let now = local(2026, 7, 7, 9, 30, 0);

    let err = parse_target_time("2026/07/08 01:02", now).unwrap_err();

    assert!(matches!(err, ParseTargetError::UnsupportedFormat(_)));
}
