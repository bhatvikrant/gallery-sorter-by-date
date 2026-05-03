//! Parallel file copier with safe per-directory deduplication.
//!
//! Replaces the fully sequential loop in `src/copier.ts` and the O(n)
//! `fs.access` chain inside `getUniqueFilename`. Each destination directory
//! gets a single `read_dir` to seed a `HashSet` of taken names, then copies
//! within a directory are serialised via a per-directory mutex (so the
//! collision check + reservation is atomic) while different directories run in
//! full parallel across the rayon pool.

use crate::types::{DateGroup, MediaMeta};
use anyhow::Result;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub struct CopyResult {
    pub folders_created: usize,
    pub files_copied: usize,
    pub files_skipped: usize,
    pub bytes_copied: u64,
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
    taken: HashSet<String>,
}

pub fn copy_groups(
    groups: &[DateGroup],
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
        // Seed taken-name set with whatever's already on disk so duplicate
        // filenames from prior runs are honoured.
        let mut taken: HashSet<String> = HashSet::new();
        if let Ok(rd) = std::fs::read_dir(&dest_dir) {
            for entry in rd.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    taken.insert(name.to_string());
                }
            }
        }
        dir_states.insert(dest_dir, Arc::new(Mutex::new(DirState { taken })));
    }

    // Phase B: parallel copy. Flatten (group, file) pairs so rayon can spread
    // work across the whole input rather than one group at a time.
    let work: Vec<(&DateGroup, &MediaMeta)> = groups
        .iter()
        .flat_map(|g| g.files.iter().map(move |f| (g, f)))
        .collect();

    let copied = std::sync::atomic::AtomicU64::new(0);
    let skipped = std::sync::atomic::AtomicU64::new(0);
    let bytes = std::sync::atomic::AtomicU64::new(0);

    work.par_iter().for_each(|(g, meta)| {
        let dest_dir = dest_base.join(&g.folder_path);
        let state = dir_states
            .get(&dest_dir)
            .expect("ensure_dir populated this entry above")
            .clone();

        match copy_one(meta, &dest_dir, &state, &progress) {
            Ok(outcome) => match outcome {
                Outcome::Copied(n) => {
                    copied.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    bytes.fetch_add(n, std::sync::atomic::Ordering::Relaxed);
                }
                Outcome::Deduped => {
                    skipped.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            },
            Err(e) => {
                // Don't abort the whole run for one bad file; log to stderr and continue.
                eprintln!(
                    "  [warn] failed to copy {}: {e}",
                    meta.file.path.display()
                );
            }
        }
    });

    Ok(CopyResult {
        folders_created,
        files_copied: copied.load(std::sync::atomic::Ordering::Relaxed) as usize,
        files_skipped: skipped.load(std::sync::atomic::Ordering::Relaxed) as usize,
        bytes_copied: bytes.load(std::sync::atomic::Ordering::Relaxed),
    })
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

    // Same-size dedup against existing file at the same destination name.
    let direct_dest = dest_dir.join(original);
    if let Ok(existing) = std::fs::metadata(&direct_dest) {
        if existing.len() == meta.file.size {
            progress.on_copy(CopyEvent {
                final_name: original,
                dest_dir,
                bytes: meta.file.size,
                deduped: true,
            });
            return Ok(Outcome::Deduped);
        }
    }

    // Reserve a unique filename atomically.
    let final_name = {
        let mut guard = state.lock().expect("dir mutex poisoned");
        let name = pick_unique(&guard.taken, original);
        guard.taken.insert(name.clone());
        name
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
}
