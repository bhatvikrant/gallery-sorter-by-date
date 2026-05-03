//! Parallel directory scanner powered by `jwalk`.
//!
//! Replaces the single-threaded `glob` walk in `src/scanner.ts` and reports
//! discoveries live to the progress UI.

use crate::extensions::{classify, lower_ext};
use crate::types::{MediaFile, MediaType};
use anyhow::Result;
use jwalk::WalkDir;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct ScanResult {
    pub files: Vec<MediaFile>,
    pub image_count: usize,
    pub video_count: usize,
    pub directory_count: usize,
    pub total_bytes: u64,
}

pub trait ScanProgress: Send + Sync {
    fn on_file(&self, _file: &MediaFile) {}
    fn on_dir(&self) {}
}

/// No-op implementation for tests.
#[allow(dead_code)]
pub struct NoopProgress;
impl ScanProgress for NoopProgress {}

/// Recursively walks `root` in parallel and returns every file whose extension
/// matches one of the supported image/video formats.
pub fn scan_directory<P: AsRef<Path>>(
    root: P,
    progress: Arc<dyn ScanProgress>,
) -> Result<ScanResult> {
    let bytes = AtomicU64::new(0);
    let images = AtomicU64::new(0);
    let videos = AtomicU64::new(0);

    let mut files: Vec<MediaFile> = Vec::new();
    let mut dirs: HashSet<std::path::PathBuf> = HashSet::new();

    // jwalk parallelises the walk across rayon's global pool. We collect into a
    // single thread at the end to keep ordering deterministic and to allow the
    // progress callback to count atomically.
    for entry in WalkDir::new(&root).skip_hidden(false).follow_links(false) {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            if entry.file_type().is_dir() {
                progress.on_dir();
            }
            continue;
        }
        let path = entry.path();
        let media_type = match classify(&path) {
            Some(t) => t,
            None => continue,
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        bytes.fetch_add(size, Ordering::Relaxed);
        match media_type {
            MediaType::Image => {
                images.fetch_add(1, Ordering::Relaxed);
            }
            MediaType::Video => {
                videos.fetch_add(1, Ordering::Relaxed);
            }
        }

        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let extension = lower_ext(&path);
        if let Some(parent) = path.parent() {
            dirs.insert(parent.to_path_buf());
        }
        let mf = MediaFile {
            path,
            filename,
            extension,
            media_type,
            size,
        };
        progress.on_file(&mf);
        files.push(mf);
    }

    Ok(ScanResult {
        files,
        image_count: images.load(Ordering::Relaxed) as usize,
        video_count: videos.load(Ordering::Relaxed) as usize,
        directory_count: dirs.len(),
        total_bytes: bytes.load(Ordering::Relaxed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;

    #[test]
    fn finds_only_media() {
        let tmp = tempdir();
        fs::write(tmp.join("a.jpg"), b"x").unwrap();
        fs::write(tmp.join("b.JPG"), b"x").unwrap();
        fs::write(tmp.join("readme.txt"), b"x").unwrap();
        fs::create_dir_all(tmp.join("nested")).unwrap();
        fs::write(tmp.join("nested/c.mp4"), b"x").unwrap();

        let r = scan_directory(&tmp, Arc::new(NoopProgress)).unwrap();
        assert_eq!(r.image_count, 2);
        assert_eq!(r.video_count, 1);
        assert!(r.directory_count >= 1);
    }

    fn tempdir() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("gs-test-{}", n));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
