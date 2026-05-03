//! Parallel metadata extraction over all discovered media files.
//!
//! Mirrors the date-source priority of the legacy TypeScript pipeline:
//!   * images   : EXIF (`DateTimeOriginal -> CreateDate -> DateTime`) → fs
//!   * mp4/mov  : QuickTime keys → mvhd → fs
//!   * other vid: ffprobe (creation_time, qt key) → fs
//!
//! Replaces `src/metadata.ts`'s `pLimit(10)` cap with a real rayon thread pool
//! sized to the available CPU count.

use crate::exif_image::extract_image_date;
use crate::extensions::{uses_ffprobe, uses_mp4parse};
use crate::ffprobe_fallback::extract_via_ffprobe;
use crate::mp4_meta::extract_mp4_date;
use crate::types::{DateSource, MediaFile, MediaMeta, MediaType};
use chrono::{DateTime, Local, TimeZone};
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub trait MetadataProgress: Send + Sync {
    fn on_completed(&self, _meta: &MediaMeta) {}
}

#[allow(dead_code)]
pub struct NoopProgress;
impl MetadataProgress for NoopProgress {}

pub fn extract_all(
    files: Vec<MediaFile>,
    progress: Arc<dyn MetadataProgress>,
) -> Vec<MediaMeta> {
    let done = AtomicU64::new(0);

    files
        .into_par_iter()
        .map(|file| {
            let meta = extract_one(file);
            done.fetch_add(1, Ordering::Relaxed);
            progress.on_completed(&meta);
            meta
        })
        .collect()
}

fn extract_one(file: MediaFile) -> MediaMeta {
    let result = match file.media_type {
        MediaType::Image => extract_image_date(&file.path),
        MediaType::Video => {
            if uses_mp4parse(&file.extension) {
                extract_mp4_date(&file.path)
            } else if uses_ffprobe(&file.extension) {
                extract_via_ffprobe(&file.path)
            } else {
                None
            }
        }
    };

    let (created_date, date_source) =
        result.unwrap_or_else(|| filesystem_date(&file.path));

    MediaMeta {
        file,
        created_date,
        date_source,
    }
}

/// Falls back to filesystem timestamps in the same priority as the legacy code:
/// `birthtime` first if available and non-zero, then `mtime`.
fn filesystem_date(path: &Path) -> (DateTime<Local>, DateSource) {
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(birth) = meta.created() {
            if let Some(dt) = system_to_local(birth) {
                // Reject epoch-zero (matches legacy `birthtime.getTime() > 0`).
                if dt.timestamp() > 0 {
                    return (dt, DateSource::FileBirthtime);
                }
            }
        }
        if let Ok(modified) = meta.modified() {
            if let Some(dt) = system_to_local(modified) {
                return (dt, DateSource::FileMtime);
            }
        }
    }
    // Last-ditch: epoch.
    (
        Local.timestamp_opt(0, 0).single().unwrap_or_else(|| {
            // chrono guarantees this is Some, but be defensive.
            Local
                .with_ymd_and_hms(1970, 1, 1, 0, 0, 0)
                .single()
                .unwrap()
        }),
        DateSource::FileMtime,
    )
}

fn system_to_local(t: std::time::SystemTime) -> Option<DateTime<Local>> {
    let dur = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    let secs = dur.as_secs() as i64;
    let nanos = dur.subsec_nanos();
    Local.timestamp_opt(secs, nanos).single()
}
