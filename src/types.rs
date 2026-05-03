//! Shared types used across the engine. Mirrors `src/types.ts`.

use chrono::{DateTime, Local};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateSource {
    Exif,
    QuickTime,
    Mp4Box,
    Ffprobe,
    FileBirthtime,
    FileMtime,
}

impl DateSource {
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            DateSource::Exif => "EXIF",
            DateSource::QuickTime => "QuickTime",
            DateSource::Mp4Box => "MP4box",
            DateSource::Ffprobe => "ffprobe",
            DateSource::FileBirthtime => "fs-birthtime",
            DateSource::FileMtime => "fs-mtime",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaFile {
    pub path: PathBuf,
    pub filename: String,
    /// Lowercase extension including the leading dot (e.g. `.jpg`).
    pub extension: String,
    pub media_type: MediaType,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct MediaMeta {
    pub file: MediaFile,
    pub created_date: DateTime<Local>,
    pub date_source: DateSource,
}

#[derive(Debug)]
pub struct DateGroup {
    #[allow(dead_code)]
    pub date_key: String,
    pub folder_path: PathBuf,
    pub display_name: String,
    pub date: DateTime<Local>,
    pub files: Vec<MediaMeta>,
    pub image_count: usize,
    pub video_count: usize,
}
