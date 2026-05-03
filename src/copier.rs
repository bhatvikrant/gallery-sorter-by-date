//! Parallel file copier with safe per-directory deduplication.
//!
//! Replaces the fully sequential loop in `src/copier.ts` and the O(n)
//! `fs.access` chain inside `getUniqueFilename`. Each destination directory
//! gets a single `read_dir` to seed a `HashSet` of taken names, then copies
//! within a directory are serialised via a per-directory mutex (so the
//! collision check + reservation is atomic) while different directories run in
//! full parallel across the rayon pool.

use crate::types::{DateGroup, MediaMeta, SkipKind, SkippedFile, UnsortableMedia};
use anyhow::Result;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Sub-folder name (under the main `sorted by date/` output) that receives
/// any media we couldn't determine a capture date for.
pub const UNSORTABLE_FOLDER: &str = "unsortable";

#[derive(Debug, Default)]
pub struct CopyResult {
    pub folders_created: usize,
    pub files_copied: usize,
    /// Files we didn't copy because an identical-name, identical-size file
    /// was already at the destination (idempotent re-runs).
    pub files_skipped: usize,
    pub bytes_copied: u64,
    /// Number of unsortable files copied into the `unsortable/` folder.
    pub unsortable_copied: usize,
    /// Per-file records explaining anything that wasn't placed into a normal
    /// date folder (unsortable, or copy failures).
    pub skipped: Vec<SkippedFile>,
}

#[derive(Debug, Clone, Copy)]
pub struct CopyEvent<'a> {
    pub final_name: &'a str,
    pub dest_dir: &'a Path,
    pub bytes: u64,
    pub deduped: bool,
}

pub trait CopyProgress: Send + Sync {
    fn on_copy(&self, _event: CopyEvent<'_>) {}
}

#[allow(dead_code)]
pub struct NoopProgress;
impl CopyProgress for NoopProgress {}

struct DirState {
    /// Names that were already on disk *before* this run started. Only these
    /// are valid dedup targets — a name we created during this run must
    /// **not** be treated as a "duplicate" just because a different source
    /// file happens to share its name and size (that previously caused
    /// collisions to silently disappear).
    pre_existing: HashSet<String>,
    /// Every name reserved in this destination directory, including ones we
    /// added during this run. Used by `pick_unique` to avoid stomping.
    taken: HashSet<String>,
}

pub fn copy_groups(
    groups: &[DateGroup],
    unsortable: &[UnsortableMedia],
    dest_base: &Path,
    progress: Arc<dyn CopyProgress>,
) -> Result<CopyResult> {
    // Phase A: serially create destination directories. This is cheap and
    // sidesteps races where multiple workers race to mkdir the same path.
    let mut folders_created = 0usize;
    let mut dir_states: HashMap<PathBuf, Arc<Mutex<DirState>>> = HashMap::new();

    for g in groups {
        let dest_dir = dest_base.join(&g.folder_path);
        let was_created = ensure_dir(&dest_dir)?;
        if was_created {
            folders_created += 1;
        }
        dir_states.insert(dest_dir.clone(), seed_dir_state(&dest_dir));
    }

    // Reserve the unsortable bin too, if needed.
    let unsortable_dir = dest_base.join(UNSORTABLE_FOLDER);
    let unsortable_state = if !unsortable.is_empty() {
        if ensure_dir(&unsortable_dir)? {
            folders_created += 1;
        }
        Some(seed_dir_state(&unsortable_dir))
    } else {
        None
    };

    // Phase B: parallel copy. Flatten (group, file) pairs so rayon can spread
    // work across the whole input rather than one group at a time.
    let work: Vec<(&DateGroup, &MediaMeta)> = groups
        .iter()
        .flat_map(|g| g.files.iter().map(move |f| (g, f)))
        .collect();

    let copied = std::sync::atomic::AtomicU64::new(0);
    let skipped = std::sync::atomic::AtomicU64::new(0);
    let bytes = std::sync::atomic::AtomicU64::new(0);
    let unsortable_copied = std::sync::atomic::AtomicU64::new(0);
    let skip_records: Mutex<Vec<SkippedFile>> = Mutex::new(Vec::new());

    work.par_iter().for_each(|(g, meta)| {
        let dest_dir = dest_base.join(&g.folder_path);
        let state = dir_states
            .get(&dest_dir)
            .expect("ensure_dir populated this entry above")
            .clone();

        match copy_one(meta, &dest_dir, &state, &progress) {
            Ok(Outcome::Copied(n)) => {
                copied.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                bytes.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(Outcome::Deduped) => {
                skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            Err(e) => {
                skip_records.lock().expect("skip records").push(SkippedFile {
                    path: meta.file.path.clone(),
                    reason: format!("copy failed: {e}"),
                    kind: SkipKind::CopyFailed,
                    media_type: meta.file.media_type,
                });
            }
        }
    });

    // Phase C: copy the unsortable bin (typically tiny, but fully parallel
    // anyway for consistency with the main copy).
    if let Some(state) = &unsortable_state {
        unsortable.par_iter().for_each(|u| {
            // Reuse `copy_one`'s dedup + unique-naming logic by wrapping into
            // a synthetic `MediaMeta`. The date isn't actually used by the
            // copier — only the file/path/size are.
            let synthetic = MediaMeta {
                file: u.file.clone(),
                created_date: chrono::Local::now(),
                date_source: crate::types::DateSource::FileMtime,
            };
            match copy_one(&synthetic, &unsortable_dir, state, &progress) {
                Ok(Outcome::Copied(n)) => {
                    unsortable_copied.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    bytes.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                    skip_records.lock().expect("skip records").push(SkippedFile {
                        path: u.file.path.clone(),
                        reason: u.reason.clone(),
                        kind: SkipKind::Unsortable,
                        media_type: u.file.media_type,
                    });
                }
                Ok(Outcome::Deduped) => {
                    skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    skip_records.lock().expect("skip records").push(SkippedFile {
                        path: u.file.path.clone(),
                        reason: format!("{} (already present in unsortable/)", u.reason),
                        kind: SkipKind::Unsortable,
                        media_type: u.file.media_type,
                    });
                }
                Err(e) => {
                    skip_records.lock().expect("skip records").push(SkippedFile {
                        path: u.file.path.clone(),
                        reason: format!(
                            "{} — and copy into unsortable/ also failed: {e}",
                            u.reason
                        ),
                        kind: SkipKind::CopyFailed,
                        media_type: u.file.media_type,
                    });
                }
            }
        });
    }

    Ok(CopyResult {
        folders_created,
        files_copied: copied.load(std::sync::atomic::Ordering::Relaxed) as usize,
        files_skipped: skipped.load(std::sync::atomic::Ordering::Relaxed) as usize,
        bytes_copied: bytes.load(std::sync::atomic::Ordering::Relaxed),
        unsortable_copied: unsortable_copied.load(std::sync::atomic::Ordering::Relaxed) as usize,
        skipped: skip_records.into_inner().expect("skip records"),
    })
}

fn seed_dir_state(dir: &Path) -> Arc<Mutex<DirState>> {
    // Seed both name sets with whatever's already on disk so:
    //   * duplicate filenames from prior runs are honoured (idempotent),
    //   * `pick_unique` doesn't try to use names that already exist.
    let mut taken: HashSet<String> = HashSet::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                taken.insert(name.to_string());
            }
        }
    }
    let pre_existing = taken.clone();
    Arc::new(Mutex::new(DirState {
        pre_existing,
        taken,
    }))
}

enum Outcome {
    Copied(u64),
    Deduped,
}

fn copy_one(
    meta: &MediaMeta,
    dest_dir: &Path,
    state: &Mutex<DirState>,
    progress: &Arc<dyn CopyProgress>,
) -> Result<Outcome> {
    let src = &meta.file.path;
    let original = &meta.file.filename;

    // Reserve a destination name *atomically* under the per-directory mutex.
    // The dedup check has to live in here too — if we did it outside the
    // lock, two source files with the same filename and (coincidentally)
    // the same size could race: the first finishes its copy, then the
    // second's pre-lock dedup check sees that file on disk and silently
    // drops itself as a "duplicate". That's how the previous version was
    // losing different files from different source subdirectories.
    enum Reserved {
        // Name matches a file that pre-existed this run with the same size —
        // safe to dedup against (idempotent re-run).
        Dedup,
        // Use this freshly-chosen name (renamed if needed).
        Use(String),
    }
    let reserved = {
        let mut guard = state.lock().expect("dir mutex poisoned");

        // Only treat a destination file as a true duplicate if it was *already
        // on disk before this run started*. A name we placed there ourselves
        // earlier in this run is just a filename collision and must be
        // renamed, not skipped.
        let pre_existing_match = guard.pre_existing.contains(original) && {
            let direct_dest = dest_dir.join(original);
            std::fs::metadata(&direct_dest)
                .map(|m| m.len() == meta.file.size)
                .unwrap_or(false)
        };

        if pre_existing_match {
            Reserved::Dedup
        } else {
            let name = pick_unique(&guard.taken, original);
            guard.taken.insert(name.clone());
            Reserved::Use(name)
        }
    };

    let final_name = match reserved {
        Reserved::Dedup => {
            progress.on_copy(CopyEvent {
                final_name: original,
                dest_dir,
                bytes: meta.file.size,
                deduped: true,
            });
            return Ok(Outcome::Deduped);
        }
        Reserved::Use(n) => n,
    };

    let final_dest = dest_dir.join(&final_name);
    // Try reflink first (instant on APFS / Btrfs / XFS / ReFS — the actual
    // bytes aren't moved, the filesystem just shares the data blocks
    // copy-on-write). If unsupported, falls back to a regular byte copy.
    // This is the single biggest perf win on macOS in particular.
    let bytes = match reflink_copy::reflink_or_copy(src, &final_dest)? {
        Some(n) => n,            // reflink not supported, regular copy ran
        None => meta.file.size,  // reflink succeeded; we report logical size
    };

    progress.on_copy(CopyEvent {
        final_name: &final_name,
        dest_dir,
        bytes,
        deduped: false,
    });

    Ok(Outcome::Copied(bytes))
}

fn pick_unique(taken: &HashSet<String>, original: &str) -> String {
    if !taken.contains(original) {
        return original.to_string();
    }
    let (stem, ext) = split_ext(original);
    for n in 1u64.. {
        let candidate = if ext.is_empty() {
            format!("{stem}_{n}")
        } else {
            format!("{stem}_{n}.{ext}")
        };
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("exhausted u64 candidate names");
}

fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

fn ensure_dir(p: &Path) -> Result<bool> {
    if p.exists() {
        return Ok(false);
    }
    std::fs::create_dir_all(p)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DateSource, MediaFile, MediaType};
    use chrono::Local;

    #[test]
    fn split_extension() {
        assert_eq!(split_ext("foo.jpg"), ("foo", "jpg"));
        assert_eq!(split_ext("foo"), ("foo", ""));
        assert_eq!(split_ext(".hidden"), (".hidden", ""));
        assert_eq!(split_ext("a.b.c"), ("a.b", "c"));
    }

    #[test]
    fn unique_naming() {
        let mut taken: HashSet<String> = HashSet::new();
        taken.insert("a.jpg".into());
        assert_eq!(pick_unique(&taken, "b.jpg"), "b.jpg");
        assert_eq!(pick_unique(&taken, "a.jpg"), "a_1.jpg");
        taken.insert("a_1.jpg".into());
        assert_eq!(pick_unique(&taken, "a.jpg"), "a_2.jpg");
    }

    /// Regression test: two source files in different subdirectories with the
    /// same filename and the same byte size used to be silently lost as
    /// "duplicates". They must each end up in the destination instead.
    #[test]
    fn same_name_same_size_in_different_subdirs_are_both_copied() {
        let tmp = tempdir();
        let src_a = tmp.join("camA");
        let src_b = tmp.join("camB");
        std::fs::create_dir_all(&src_a).unwrap();
        std::fs::create_dir_all(&src_b).unwrap();
        // Different *contents*, same byte length, same filename.
        std::fs::write(src_a.join("IMG.jpg"), b"AAAAAAAA").unwrap();
        std::fs::write(src_b.join("IMG.jpg"), b"BBBBBBBB").unwrap();

        let group = DateGroup {
            date_key: "2026-05-04".into(),
            folder_path: PathBuf::from("2026/5. May/4th"),
            display_name: "4th May 2026".into(),
            date: Local::now(),
            files: vec![
                synthetic_meta(src_a.join("IMG.jpg"), 8),
                synthetic_meta(src_b.join("IMG.jpg"), 8),
            ],
            image_count: 2,
            video_count: 0,
        };

        let dest = tmp.join("out");
        let result = copy_groups(&[group], &[], &dest, Arc::new(NoopProgress)).unwrap();

        assert_eq!(result.files_copied, 2, "both source files must be copied");
        assert_eq!(result.files_skipped, 0, "neither file is a real duplicate");

        let dest_dir = dest.join("2026/5. May/4th");
        let mut names: Vec<String> = std::fs::read_dir(&dest_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["IMG.jpg", "IMG_1.jpg"]);
    }

    /// Re-running on the same source must remain idempotent: a file that
    /// pre-existed at the destination with the same name and size is deduped.
    #[test]
    fn pre_existing_destination_file_is_deduped() {
        let tmp = tempdir();
        let src = tmp.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("IMG.jpg"), b"AAAAAAAA").unwrap();

        let dest = tmp.join("out");
        let dest_day = dest.join("2026/5. May/4th");
        std::fs::create_dir_all(&dest_day).unwrap();
        // Identical name + size already at the destination.
        std::fs::write(dest_day.join("IMG.jpg"), b"AAAAAAAA").unwrap();

        let group = DateGroup {
            date_key: "2026-05-04".into(),
            folder_path: PathBuf::from("2026/5. May/4th"),
            display_name: "4th May 2026".into(),
            date: Local::now(),
            files: vec![synthetic_meta(src.join("IMG.jpg"), 8)],
            image_count: 1,
            video_count: 0,
        };

        let result = copy_groups(&[group], &[], &dest, Arc::new(NoopProgress)).unwrap();
        assert_eq!(result.files_copied, 0);
        assert_eq!(result.files_skipped, 1);
    }

    fn synthetic_meta(path: PathBuf, size: u64) -> MediaMeta {
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        MediaMeta {
            file: MediaFile {
                path,
                filename,
                extension: ".jpg".into(),
                media_type: MediaType::Image,
                size,
            },
            created_date: Local::now(),
            date_source: DateSource::FileMtime,
        }
    }

    fn tempdir() -> PathBuf {
        // Process id + atomic counter + nanos guarantees each test gets a
        // distinct directory even when several tests run in parallel and hit
        // the same wall-clock nanosecond.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let c = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let mut p = std::env::temp_dir();
        p.push(format!("gs-copier-test-{}-{}-{}", pid, c, n));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
