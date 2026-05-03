//! Date formatting that mirrors `src/dateFormatter.ts` exactly.
//!
//! Folder format: `yyyy/M. MMM/Nth` (e.g. `2025/1. Jan/15th`)
//! Display format: `Nth MMM yyyy` (e.g. `15th Jan 2025`)
//! Date key: `yyyy-MM-dd`

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use std::path::PathBuf;

const MONTH_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn ordinal_suffix(day: u32) -> &'static str {
    if (11..=13).contains(&day) {
        return "th";
    }
    match day % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

fn day_with_ordinal(day: u32) -> String {
    format!("{}{}", day, ordinal_suffix(day))
}

/// `2025/1. Jan/15th`
pub fn format_for_folder(date: &DateTime<Local>) -> PathBuf {
    let year = date.year();
    let month_idx = date.month();
    let month_name = MONTH_SHORT[(month_idx - 1) as usize];
    let day = day_with_ordinal(date.day());

    let mut p = PathBuf::new();
    p.push(year.to_string());
    p.push(format!("{}. {}", month_idx, month_name));
    p.push(day);
    p
}

/// `15th Jan 2025`
pub fn format_for_display(date: &DateTime<Local>) -> String {
    let day = day_with_ordinal(date.day());
    let month_name = MONTH_SHORT[(date.month() - 1) as usize];
    format!("{} {} {}", day, month_name, date.year())
}

/// `yyyy-MM-dd`
pub fn date_key(date: &DateTime<Local>) -> String {
    format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day())
}

/// Parse a date key (`yyyy-MM-dd`) back into a `DateTime<Local>` at midnight local time.
pub fn parse_date_key(key: &str) -> Option<DateTime<Local>> {
    let nd = NaiveDate::parse_from_str(key, "%Y-%m-%d").ok()?;
    let ndt = nd.and_hms_opt(0, 0, 0)?;
    Local.from_local_datetime(&ndt).single()
}

/// Parse the EXIF date format `YYYY:MM:DD HH:MM:SS` into a local-time `DateTime`.
///
/// EXIF timestamps are conventionally written without timezone information; we
/// interpret them as local time, matching `parseExifDate` in the legacy code.
pub fn parse_exif_date(s: &str) -> Option<DateTime<Local>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let ndt = chrono::NaiveDateTime::parse_from_str(s, "%Y:%m:%d %H:%M:%S").ok()?;
    Local.from_local_datetime(&ndt).single()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(y, m, day, 12, 0, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn ordinals() {
        assert_eq!(ordinal_suffix(1), "st");
        assert_eq!(ordinal_suffix(2), "nd");
        assert_eq!(ordinal_suffix(3), "rd");
        assert_eq!(ordinal_suffix(4), "th");
        assert_eq!(ordinal_suffix(11), "th");
        assert_eq!(ordinal_suffix(12), "th");
        assert_eq!(ordinal_suffix(13), "th");
        assert_eq!(ordinal_suffix(21), "st");
        assert_eq!(ordinal_suffix(22), "nd");
        assert_eq!(ordinal_suffix(23), "rd");
        assert_eq!(ordinal_suffix(31), "st");
    }

    #[test]
    fn folder_path() {
        let p = format_for_folder(&d(2025, 1, 15));
        assert_eq!(p.to_string_lossy(), "2025/1. Jan/15th");
    }

    #[test]
    fn display() {
        assert_eq!(format_for_display(&d(2025, 1, 15)), "15th Jan 2025");
        assert_eq!(format_for_display(&d(2025, 12, 3)), "3rd Dec 2025");
    }

    #[test]
    fn key_roundtrip() {
        let key = date_key(&d(2025, 6, 7));
        assert_eq!(key, "2025-06-07");
        let back = parse_date_key(&key).unwrap();
        assert_eq!((back.year(), back.month(), back.day()), (2025, 6, 7));
    }

    #[test]
    fn exif_parse() {
        let dt = parse_exif_date("2025:01:10 14:30:00").unwrap();
        assert_eq!((dt.year(), dt.month(), dt.day()), (2025, 1, 10));
        assert!(parse_exif_date("not a date").is_none());
        assert!(parse_exif_date("").is_none());
    }
}
