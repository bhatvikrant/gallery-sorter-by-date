//! `ffprobe` fallback for non-ISOBMFF video containers
//! (`.avi .mkv .wmv .flv .webm`).
//!
//! Spawns ffprobe per file only for these formats. The common case
//! (.mp4/.mov/.m4v/.3gp) is handled by the in-process parser in `mp4_meta.rs`.

use crate::types::DateSource;
use chrono::{DateTime, Local, Utc};
use serde::Deserialize;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Deserialize)]
struct ProbeOut {
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    tags: Option<ProbeTags>,
}

#[derive(Deserialize)]
struct ProbeTags {
    creation_time: Option<String>,
    #[serde(rename = "com.apple.quicktime.creationdate")]
    qt_creationdate: Option<String>,
}

pub fn extract_via_ffprobe(path: &Path) -> Option<(DateTime<Local>, DateSource)> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_entries",
            "format_tags",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let parsed: ProbeOut = serde_json::from_slice(&output.stdout).ok()?;
    let tags = parsed.format?.tags?;

    if let Some(s) = tags.qt_creationdate.as_deref() {
        if let Some(dt) = parse_iso_like(s) {
            return Some((dt.with_timezone(&Local), DateSource::QuickTime));
        }
    }
    if let Some(s) = tags.creation_time.as_deref() {
        if let Some(dt) = parse_iso_like(s) {
            return Some((dt.with_timezone(&Local), DateSource::Ffprobe));
        }
    }
    None
}

fn parse_iso_like(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Some tools emit `YYYY-MM-DDTHH:MM:SS.fffZ`-ish strings ffprobe normalises
    // already, but accept a couple of close variants just in case.
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z") {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(dt) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%z") {
        return Some(dt.with_timezone(&Utc));
    }
    None
}

/// Returns true if `ffprobe` is on PATH. Used to decide whether to even attempt
/// the fallback for AVI/MKV/etc (we silently degrade to fs-mtime if not).
pub fn ffprobe_available() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
