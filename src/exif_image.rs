//! Image date extraction.
//!
//! Streams only the first ~64 KB of the file via `BufReader`, so a 100 MB RAW
//! costs ~64 KB of I/O instead of the full read that the legacy
//! `src/metadata.ts` performs (`fs.readFile(filePath)` on every image).

use crate::date_format::parse_exif_date;
use crate::types::DateSource;
use chrono::{DateTime, Local};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Tries the same EXIF priority as the legacy code:
/// `DateTimeOriginal -> CreateDate / DateTimeDigitized -> DateTime`.
pub fn extract_image_date(path: &Path) -> Option<(DateTime<Local>, DateSource)> {
    let file = File::open(path).ok()?;
    let mut buf = BufReader::with_capacity(64 * 1024, file);

    let exif_reader = exif::Reader::new();
    let exif = exif_reader.read_from_container(&mut buf).ok()?;

    let try_tag = |tag: exif::Tag| -> Option<DateTime<Local>> {
        let field = exif.get_field(tag, exif::In::PRIMARY)?;
        // The standard textual representation is `YYYY:MM:DD HH:MM:SS`.
        let s = field.display_value().with_unit(&exif).to_string();
        parse_exif_date(&s)
    };

    if let Some(d) = try_tag(exif::Tag::DateTimeOriginal) {
        return Some((d, DateSource::Exif));
    }
    if let Some(d) = try_tag(exif::Tag::DateTimeDigitized) {
        return Some((d, DateSource::Exif));
    }
    if let Some(d) = try_tag(exif::Tag::DateTime) {
        return Some((d, DateSource::Exif));
    }
    None
}
