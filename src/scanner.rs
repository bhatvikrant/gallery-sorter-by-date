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
///
/// `skip_root`, if provided, prunes any path under it from the walk — used to
/// stop a re-run from re-scanning its own previously-produced output folder.
///
/// Symlinks pointing at regular files are treated as files (the previous
/// behaviour silently dropped any symlinked media).
pub fn scan_directory<P: AsRef<Path>>(
    root: P,
    skip_root: Option<&Path>,
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
        let path = entry.path();

        // Don't descend into the output folder if we're re-running on the same
        // source directory — otherwise we'd treat already-sorted files as new
        // input and waste a copy pass deduping every one of them.
        if let Some(sr) = skip_root {
            if path.starts_with(sr) {
                continue;
            }
        }

        let file_type = entry.file_type();
        if file_type.is_dir() {
            progress.on_dir();
            continue;
        }

        // Symlinks to files are *not* `is_file()` with `follow_links(false)`;
        // resolve them explicitly so libraries built with symlinks still
        // sort. Broken symlinks fall through and get skipped quietly.
        let is_real_file = file_type.is_file()
            || (file_type.is_symlink()
                && std::fs::metadata(&path)
                    .map(|m| m.is_file())
                    .unwrap_or(false));
        if !is_real_file {
            continue;
        }

        let media_type = match classify(&path) {
            Some(t) => t,
            None => continue,
        };
        // Use `std::fs::metadata` (follows symlinks) so symlinked files report
        // their actual size, not the symlink's 96-byte placeholder.
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
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

        let r = scan_directory(&tmp, None, Arc::new(NoopProgress)).unwrap();
        assert_eq!(r.image_count, 2);
        assert_eq!(r.video_count, 1);
        assert!(r.directory_count >= 1);
    }

    #[test]
    fn follows_symlinks_to_media_files() {
        let tmp = tempdir();
        fs::write(tmp.join("real.jpg"), b"hello").unwrap();
        // Symlink with a media extension pointing at a real media file.
        std::os::unix::fs::symlink(tmp.join("real.jpg"), tmp.join("link.jpg")).unwrap();

        let r = scan_directory(&tmp, None, Arc::new(NoopProgress)).unwrap();
        assert_eq!(r.image_count, 2, "symlinked media should be discovered");
        // Reported size should be the real file's size, not the symlink's.
        for f in &r.files {
            assert_eq!(f.size, 5);
        }
    }

    #[test]
    fn skip_root_excludes_subtree() {
        let tmp = tempdir();
        fs::write(tmp.join("keep.jpg"), b"x").unwrap();
        let skip = tmp.join("sorted by date");
        fs::create_dir_all(skip.join("2026")).unwrap();
        fs::write(skip.join("2026/old.jpg"), b"x").unwrap();

        let r = scan_directory(&tmp, Some(&skip), Arc::new(NoopProgress)).unwrap();
        assert_eq!(r.image_count, 1, "should not re-pick previously sorted files");
        assert_eq!(r.files[0].filename, "keep.jpg");
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
