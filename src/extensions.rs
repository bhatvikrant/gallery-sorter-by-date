//! Centralised media extension classification. Mirrors `src/constants.ts`.

use crate::types::MediaType;

pub const IMAGE_EXTS: &[&str] = &[
    ".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".tiff", ".tif", ".heic", ".heif", ".cr2",
    ".cr3", ".nef", ".arw", ".orf", ".rw2", ".dng", ".raf",
];

pub const VIDEO_EXTS: &[&str] = &[
    ".mp4", ".mov", ".avi", ".mkv", ".wmv", ".flv", ".webm", ".3gp", ".m4v",
];

/// Extensions handled by the pure-Rust mp4parse path (ISO BMFF family).
pub const MP4PARSE_EXTS: &[&str] = &[".mp4", ".mov", ".m4v", ".3gp"];

/// Extensions that require ffprobe fallback (non-ISOBMFF containers).
pub const FFPROBE_EXTS: &[&str] = &[".avi", ".mkv", ".wmv", ".flv", ".webm"];

/// Returns the lowercased extension (with leading dot) of `path`, or empty string.
pub fn lower_ext(path: &std::path::Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{}", s.to_ascii_lowercase()))
        .unwrap_or_default()
}

pub fn classify(path: &std::path::Path) -> Option<MediaType> {
    let ext = lower_ext(path);
    if IMAGE_EXTS.contains(&ext.as_str()) {
        Some(MediaType::Image)
    } else if VIDEO_EXTS.contains(&ext.as_str()) {
        Some(MediaType::Video)
    } else {
        None
    }
}

pub fn uses_mp4parse(ext: &str) -> bool {
    MP4PARSE_EXTS.contains(&ext)
}

pub fn uses_ffprobe(ext: &str) -> bool {
    FFPROBE_EXTS.contains(&ext)
}
