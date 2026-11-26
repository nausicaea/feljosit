//! Dates and times (with the library `jiff`, see ADR-0020).
//!
//! The other modules use only the functions of this module. Thus a change of the date
//! library changes only this file.
//!
//! - A calendar date ([`Date`]) has no time and no time zone, for example a booking date.
//! - A point in time ([`Timestamp`]) is in UTC, for example the expiry of a session.

use jiff::tz::TimeZone;
use jiff::{SignedDuration, ToSpan, ZonedDateTime};

pub use jiff::Timestamp;
pub use jiff::civil::Date;

/// The date for a year, a month (1 to 12) and a day. `None` if the date does not exist.
pub fn ymd(year: i32, month: u32, day: u32) -> Option<Date> {
    Date::new(i16::try_from(year).ok()?, i8::try_from(month).ok()?, i8::try_from(day).ok()?).ok()
}

/// Today in the time zone of the computer.
pub fn today() -> Date {
    ZonedDateTime::now().date()
}

/// The current time.
pub fn now() -> Timestamp {
    Timestamp::now()
}

pub fn year(date: Date) -> i32 {
    i32::from(date.year())
}

/// The month from 0 (January) to 11 (December).
pub fn month0(date: Date) -> u32 {
    u32::try_from(date.month() - 1).expect("The month is 1 to 12")
}

/// The number of the month since the year 0: `year × 12 + month0`.
pub fn month_index(date: Date) -> i32 {
    year(date) * 12 + month0(date) as i32
}

/// The first day of the month with the number `index` (see [`month_index`]).
pub fn from_month_index(index: i32) -> Option<Date> {
    ymd(index.div_euclid(12), index.rem_euclid(12) as u32 + 1, 1)
}

pub fn first_of_month(date: Date) -> Date {
    date.first_of_month()
}

pub fn last_of_month(date: Date) -> Date {
    date.last_of_month()
}

/// Add (or subtract, if negative) days. The result stays in the range of the dates.
pub fn add_days(date: Date, days: i64) -> Date {
    date.checked_add(days.days()).unwrap_or(if days < 0 { Date::MIN } else { Date::MAX })
}

/// The number of days between two dates, without a sign.
pub fn days_between(a: Date, b: Date) -> i64 {
    i64::from((a - b).get_days()).abs()
}

/// Format a date, for example with `"%d.%m.%Y"`.
pub fn format(date: Date, pattern: &str) -> String {
    date.strftime(pattern).to_string()
}

/// Parse a date with a pattern, for example `"%Y-%m-%d"`. The complete text must agree.
pub fn parse(text: &str, pattern: &str) -> Option<Date> {
    Date::strptime(pattern, text).ok()
}

/// Add (or subtract, if negative) seconds to a point in time.
pub fn add_seconds(time: Timestamp, seconds: i64) -> Timestamp {
    time.checked_add(SignedDuration::from_secs(seconds)).unwrap_or(if seconds < 0 {
        Timestamp::MIN
    } else {
        Timestamp::MAX
    })
}

/// Format a point in time in UTC, for example with `"%Y-%m-%d %H:%M:%S%.6f"`.
pub fn format_utc(time: Timestamp, pattern: &str) -> String {
    time.to_zoned_datetime(TimeZone::UTC).strftime(pattern).to_string()
}

/// Parse a date and a time in UTC with a pattern.
pub fn parse_utc(text: &str, pattern: &str) -> Option<Timestamp> {
    let civil = jiff::civil::DateTime::strptime(pattern, text).ok()?;
    Some(civil.to_zoned_datetime(TimeZone::UTC).ok()?.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar() {
        let d = ymd(2026, 10, 31).unwrap();
        assert_eq!((year(d), month0(d)), (2026, 9));
        assert_eq!(from_month_index(month_index(d) + 3), ymd(2027, 1, 1));
        assert_eq!(from_month_index(month_index(d) - 22), ymd(2024, 12, 1));
        assert_eq!(last_of_month(ymd(2024, 2, 10).unwrap()), ymd(2024, 2, 29).unwrap());
        assert_eq!(add_days(d, 4), ymd(2026, 11, 4).unwrap());
        assert_eq!(days_between(d, ymd(2026, 11, 4).unwrap()), 4);
        assert!(ymd(2026, 2, 30).is_none());
        assert!(ymd(2026, 13, 1).is_none());
    }

    #[test]
    fn text() {
        let d = ymd(2026, 3, 5).unwrap();
        assert_eq!(format(d, "%d.%m.%Y"), "05.03.2026");
        assert_eq!(parse("2026-03-05", "%Y-%m-%d"), Some(d));
        assert_eq!(parse("05.03.2026", "%d.%m.%Y"), Some(d));
        assert_eq!(parse("2026-03-05x", "%Y-%m-%d"), None);
        assert_eq!(parse("2026-13-05", "%Y-%m-%d"), None);
        let time = parse_utc("2026-11-25 10:11:12.123456", "%Y-%m-%d %H:%M:%S%.f").unwrap();
        assert_eq!(format_utc(time, "%Y-%m-%d %H:%M:%S%.6f"), "2026-11-25 10:11:12.123456");
        assert_eq!(format_utc(add_seconds(time, 60), "%H:%M"), "10:12");
    }
}
