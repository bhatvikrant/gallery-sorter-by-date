//! Parallel metadata extraction over all discovered media files.
//!
//! Mirrors the date-source priority of the legacy TypeScript pipeline:
//!   * images   : EXIF (`DateTimeOriginal -> CreateDate -> DateTime`) → fs
//!   * mp4/mov  : QuickTime keys → mvhd → fs
//!   * other vid: ffprobe (creation_time, qt key) → fs
//!
//! Replaces `src/metadata.ts`'s `pLimit(10)` cap with a real rayon thread pool
//! sized to the available CPU count.
//!
//! Files for which no real capture date *and* no usable filesystem timestamp
//! can be obtained are returned separately as `UnsortableMedia` so the caller
//! can route them into the `unsortable/` folder and surface a per-file reason
//! in the summary.

use crate::exif_image::extract_image_date;
use crate::extensions::{uses_ffprobe, uses_mp4parse};
use crate::ffprobe_fallback::extract_via_ffprobe;
use crate::mp4_meta::extract_mp4_date;
use crate::types::{DateSource, MediaFile, MediaMeta, MediaType, UnsortableMedia};
use chrono::{DateTime, Local, TimeZone};
use rayon::prelude::*;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub trait MetadataProgress: Send + Sync {
    fn on_completed(&self, _meta: &MediaMeta) {}
    fn on_unsortable(&self, _file: &UnsortableMedia) {}
}

#[allow(dead_code)]
pub struct NoopProgress;
impl MetadataProgress for NoopProgress {}

#[derive(Debug, Default)]
pub struct ExtractResult {
    pub metas: Vec<MediaMeta>,
    pub unsortable: Vec<UnsortableMedia>,
}

pub fn extract_all(
    files: Vec<MediaFile>,
    progress: Arc<dyn MetadataProgress>,
) -> ExtractResult {
    let done = AtomicU64::new(0);
    let unsortable: Mutex<Vec<UnsortableMedia>> = Mutex::new(Vec::new());

    let metas: Vec<MediaMeta> = files
        .into_par_iter()
        .filter_map(|file| {
            let outcome = extract_one(file);
            done.fetch_add(1, Ordering::Relaxed);
            match outcome {
                Ok(meta) => {
                    progress.on_completed(&meta);
                    Some(meta)
                }
                Err(bad) => {
                    progress.on_unsortable(&bad);
                    unsortable.lock().expect("unsortable mutex").push(bad);
                    None
                }
            }
        })
        .collect();

    ExtractResult {
        metas,
        unsortable: unsortable.into_inner().expect("unsortable mutex"),
    }
}

fn extract_one(file: MediaFile) -> Result<MediaMeta, UnsortableMedia> {
    let metadata_attempt = match file.media_type {
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

    if let Some((created_date, date_source)) = metadata_attempt {
        return Ok(MediaMeta {
            file,
            created_date,
            date_source,
        });
    }

    if let Some((created_date, date_source)) = filesystem_date(&file.path) {
        return Ok(MediaMeta {
            file,
            created_date,
            date_source,
        });
    }

    Err(UnsortableMedia {
        reason: unsortable_reason(&file),
        file,
    })
}

fn unsortable_reason(file: &MediaFile) -> String {
    let kind = match file.media_type {
        MediaType::Image => "no EXIF date in image",
        MediaType::Video if uses_mp4parse(&file.extension) => {
            "no creation date in MP4/MOV/M4V/3GP container"
        }
        MediaType::Video if uses_ffprobe(&file.extension) => {
            "ffprobe could not read a creation date (or ffprobe is not installed)"
        }
        MediaType::Video => "no metadata extractor available for this video format",
    };
    format!(
        "{kind}; filesystem birth/modify timestamps are also unreadable or zero"
    )
}

/// Falls back to filesystem timestamps in the same priority as the legacy code:
/// `birthtime` first if available and non-zero, then `mtime`. Returns `None`
/// only when both are unavailable or zero — in which case the file is treated
/// as unsortable rather than being silently bucketed into 1970-01-01.
fn filesystem_date(path: &Path) -> Option<(DateTime<Local>, DateSource)> {
    let meta = std::fs::metadata(path).ok()?;
    if let Ok(birth) = meta.created() {
        if let Some(dt) = system_to_local(birth) {
            if dt.timestamp() > 0 {
                return Some((dt, DateSource::FileBirthtime));
            }
        }
    }
    if let Ok(modified) = meta.modified() {
        if let Some(dt) = system_to_local(modified) {
            if dt.timestamp() > 0 {
                return Some((dt, DateSource::FileMtime));
            }
        }
    }
    None
}

fn system_to_local(t: std::time::SystemTime) -> Option<DateTime<Local>> {
    let dur = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    let secs = dur.as_secs() as i64;
    let nanos = dur.subsec_nanos();
    Local.timestamp_opt(secs, nanos).single()
}

// Kept for documentation: the legacy behaviour synthesised an epoch
// timestamp when nothing else worked. We intentionally no longer do that;
// see `filesystem_date` returning `Option`.
#[allow(dead_code)]
fn _legacy_epoch_fallback() -> DateTime<Local> {
    Local
        .timestamp_opt(0, 0)
        .single()
        .unwrap_or_else(|| Local.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).single().unwrap())
}
