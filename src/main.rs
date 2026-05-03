mod copier;
mod date_format;
mod exif_image;
mod extensions;
mod ffprobe_fallback;
mod grouper;
mod metadata;
mod mp4_meta;
mod progress;
mod scanner;
mod types;

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use indicatif::HumanBytes;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::copier::copy_groups;
use crate::ffprobe_fallback::ffprobe_available;
use crate::grouper::group_by_date;
use crate::metadata::extract_all;
use crate::progress::{format_dur, Ui, UiOptions};
use crate::scanner::scan_directory;
use crate::types::{DateGroup, SkipKind, SkippedFile};

const OUTPUT_FOLDER_NAME: &str = "sorted by date";

/// Friendly long help printed by `gallery-sorter --help`.
const LONG_ABOUT: &str = "\
Organises a folder of photos and videos into a clean
Year / Month / Day directory tree, using each file's real
capture date (EXIF, video metadata, or filesystem fallback).

Originals are always copied — never moved or modified.

Examples:
  gallery-sorter ~/Pictures/iPhone-Backup
  gallery-sorter /Volumes/SD-CARD/DCIM
  gallery-sorter .
";

#[derive(Parser, Debug)]
#[command(
    name = "gallery-sorter",
    version,
    about = "Sort your photos and videos into date folders, fast.",
    long_about = LONG_ABOUT
)]
struct Args {
    /// Folder to organise (defaults to the current directory).
    #[arg(value_name = "FOLDER")]
    source: Option<PathBuf>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{} {e:?}", style("error:").red().bold());
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();
    let source = args
        .source
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("cwd"));
    let source = std::fs::canonicalize(&source)
        .with_context(|| format!("source directory not found: {}", source.display()))?;
    let output = source.join(OUTPUT_FOLDER_NAME);

    let cores = num_cpus::get();
    rayon::ThreadPoolBuilder::new()
        .num_threads(cores)
        .build_global()
        .ok();

    // Auto-detect non-TTY (piped to a file/log shipper) and switch to
    // line-based output. Users never have to ask for it.
    let opts = UiOptions::auto();
    let ui = Ui::new(opts);
    ui.print_banner(&source, &output, cores);

    let total_start = Instant::now();

    // Phase 1: scan ──────────────────────────────────────────────────────────
    ui.phase_header(1, 3, "Scanning directories", "🔍");
    let scan_bar = ui.make_scan_bar();
    let scan_start = Instant::now();
    let scan = scan_directory(
        &source,
        Some(output.as_path()),
        scan_bar.clone() as Arc<dyn scanner::ScanProgress>,
    )?;
    let scan_elapsed = scan_start.elapsed();
    scan_bar.finish(
        scan.image_count,
        scan.video_count,
        scan.directory_count,
        scan.total_bytes,
        scan_elapsed,
    );

    if scan.files.is_empty() {
        println!("  {}", style("No media files found.").yellow());
        return Ok(());
    }

    // Quick environment sanity check for ffprobe-only formats.
    let needs_ffprobe = scan
        .files
        .iter()
        .any(|f| extensions::uses_ffprobe(&f.extension));
    if needs_ffprobe && !ffprobe_available() {
        eprintln!(
            "  {} `ffprobe` not on PATH; AVI/MKV/WMV/FLV/WebM files will fall back to filesystem dates.",
            style("warning:").yellow().bold()
        );
    }

    // Phase 2: metadata ─────────────────────────────────────────────────────
    ui.phase_header(2, 3, "Extracting metadata", "🧠");
    let total_files = scan.files.len() as u64;
    let total_bytes = scan.total_bytes;
    let meta_bar = ui.make_metadata_bar(
        total_files,
        scan.image_count as u64,
        scan.video_count as u64,
    );
    let meta_start = Instant::now();
    let extract = extract_all(scan.files, meta_bar.clone() as Arc<dyn metadata::MetadataProgress>);
    let meta_elapsed = meta_start.elapsed();
    let date_sources_snapshot = meta_bar.snapshot();
    let unsortable = extract.unsortable;
    let metas = extract.metas;
    meta_bar.finish(metas.len() as u64, meta_elapsed);

    if !unsortable.is_empty() {
        println!(
            "  {} {} file(s) had no determinable date and will be copied to {}/",
            style("note:").yellow().bold(),
            style(unsortable.len()).bold(),
            style(copier::UNSORTABLE_FOLDER).yellow()
        );
        println!();
    }

    // Phase 3: group + copy ─────────────────────────────────────────────────
    ui.phase_header(3, 3, "Copying into date folders", "📤");
    let group_result = group_by_date(metas);
    print_group_overview(&ui, &group_result.groups);

    let copy_bar = ui.make_copy_bar(total_files, total_bytes);
    let copy_start = Instant::now();
    let copy_result = copy_groups(
        &group_result.groups,
        &unsortable,
        &output,
        copy_bar.clone() as Arc<dyn copier::CopyProgress>,
    )?;
    let copy_elapsed = copy_start.elapsed();
    copy_bar.finish(copy_elapsed);

    // ─── Skipped report ────────────────────────────────────────────────────
    if !copy_result.skipped.is_empty() {
        print_skipped_report(&ui, &copy_result.skipped);
    }

    // ─── Summary card ──────────────────────────────────────────────────────
    let total_elapsed = total_start.elapsed();
    print_summary_card(
        &ui,
        SummaryInputs {
            output: &output,
            total_elapsed,
            total_files,
            image_count: group_result.total_images,
            video_count: group_result.total_videos,
            folders_created: copy_result.folders_created,
            files_copied: copy_result.files_copied,
            files_skipped: copy_result.files_skipped,
            bytes_copied: copy_result.bytes_copied,
            unsortable_copied: copy_result.unsortable_copied,
            copy_failed: copy_result
                .skipped
                .iter()
                .filter(|s| s.kind == SkipKind::CopyFailed)
                .count(),
            sources: date_sources_snapshot,
            span: oldest_to_newest(&group_result.groups),
        },
    );

    Ok(())
}

/// Prints a per-file breakdown of everything that didn't end up in a normal
/// date folder, with the human-readable reason. Truncated to keep the
/// terminal usable on huge libraries.
fn print_skipped_report(ui: &Ui, skipped: &[SkippedFile]) {
    let g = ui.glyphs;
    const MAX_PRINTED: usize = 50;

    let unsortable_total = skipped
        .iter()
        .filter(|s| s.kind == SkipKind::Unsortable)
        .count();
    let failed_total = skipped
        .iter()
        .filter(|s| s.kind == SkipKind::CopyFailed)
        .count();

    println!();
    println!(
        "{}  {}  {} skipped from sorting",
        style("▸").yellow().bold(),
        g.pick("⚠️", "[!]"),
        style(skipped.len()).bold().yellow()
    );
    if unsortable_total > 0 {
        println!(
            "  {} {} unsortable (copied to {}/)",
            style("·").dim(),
            style(unsortable_total).bold(),
            style(copier::UNSORTABLE_FOLDER).yellow()
        );
    }
    if failed_total > 0 {
        println!(
            "  {} {} copy failure(s)",
            style("·").dim(),
            style(failed_total).bold().red()
        );
    }

    for entry in skipped.iter().take(MAX_PRINTED) {
        let tag = match entry.kind {
            SkipKind::Unsortable => style(format!("[{}]", entry.kind.label())).yellow(),
            SkipKind::CopyFailed => style(format!("[{}]", entry.kind.label())).red(),
        };
        println!(
            "    {} {} — {}",
            tag,
            style(entry.path.display()).bold(),
            style(&entry.reason).dim()
        );
    }
    if skipped.len() > MAX_PRINTED {
        println!(
            "    {} {} more (full list omitted)",
            style("…").dim(),
            skipped.len() - MAX_PRINTED
        );
    }
    println!();
}

fn print_group_overview(ui: &Ui, groups: &[DateGroup]) {
    let g = ui.glyphs;
    let head = if groups.len() > 6 { 5 } else { groups.len() };
    for grp in groups.iter().take(head) {
        let mut parts: Vec<String> = Vec::with_capacity(2);
        if grp.image_count > 0 {
            parts.push(format!(
                "{} {}",
                grp.image_count,
                if grp.image_count == 1 { "image" } else { "images" }
            ));
        }
        if grp.video_count > 0 {
            parts.push(format!(
                "{} {}",
                grp.video_count,
                if grp.video_count == 1 { "video" } else { "videos" }
            ));
        }
        println!(
            "  {} {} ({})",
            style("├──").dim(),
            style(&grp.display_name).bold(),
            parts.join(", ")
        );
    }
    if groups.len() > head {
        println!(
            "  {} {} {} more day(s)",
            style("├──").dim(),
            g.pick("…", "..."),
            groups.len() - head
        );
    }
    println!();
}

struct SummaryInputs<'a> {
    output: &'a Path,
    total_elapsed: std::time::Duration,
    total_files: u64,
    image_count: usize,
    video_count: usize,
    folders_created: usize,
    files_copied: usize,
    files_skipped: usize,
    bytes_copied: u64,
    unsortable_copied: usize,
    copy_failed: usize,
    sources: progress::MetaSnapshot,
    span: Option<(chrono::DateTime<chrono::Local>, chrono::DateTime<chrono::Local>)>,
}

fn print_summary_card(ui: &Ui, s: SummaryInputs<'_>) {
    let g = ui.glyphs;
    let bar = "═".repeat(63);

    let secs = s.total_elapsed.as_secs_f64().max(0.001);
    let files_per_sec = (s.total_files as f64 / secs) as u64;
    let avg_bytes_per_sec = (s.bytes_copied as f64 / secs) as u64;

    println!("{}", style(&bar).green());
    println!(
        "  {}  All done in {}   {} {} files/s {}",
        g.pick("✨", "*"),
        style(format_dur(s.total_elapsed)).bold().green(),
        style("(").dim(),
        style(files_per_sec).bold(),
        style(")").dim()
    );
    println!("{}", style(&bar).green());

    println!(
        "  {} Images       : {}",
        g.pick("🖼", "[img]"),
        style(s.image_count).bold()
    );
    println!(
        "  {} Videos       : {}",
        g.pick("🎬", "[vid]"),
        style(s.video_count).bold()
    );
    println!(
        "  {} Folders      : {} created",
        g.pick("📁", "[dir]"),
        style(s.folders_created).bold()
    );
    println!(
        "  {} Copied       : {}",
        g.pick("✅", "[ok ]"),
        style(s.files_copied).bold().green()
    );
    if s.files_skipped > 0 {
        println!(
            "  {} Deduplicated : {}",
            g.pick("♻️", "[dup]"),
            style(s.files_skipped).bold().yellow()
        );
    }
    if s.unsortable_copied > 0 {
        println!(
            "  {} Unsortable   : {}  (copied to {}/)",
            g.pick("❓", "[?]"),
            style(s.unsortable_copied).bold().yellow(),
            style(copier::UNSORTABLE_FOLDER).yellow()
        );
    }
    if s.copy_failed > 0 {
        println!(
            "  {} Copy failed  : {}",
            g.pick("⛔", "[X]"),
            style(s.copy_failed).bold().red()
        );
    }
    println!(
        "  {} Total moved  : {}  ·  avg {}/s",
        g.pick("💾", "[siz]"),
        HumanBytes(s.bytes_copied),
        HumanBytes(avg_bytes_per_sec)
    );

    let total_src = s.sources.exif + s.sources.qt + s.sources.mp4 + s.sources.ffprobe + s.sources.fs;
    if total_src > 0 {
        println!(
            "  {} Date sources : EXIF {} · QuickTime {} · MP4box {} · ffprobe {} · fs {}",
            g.pick("📅", "[src]"),
            s.sources.exif,
            s.sources.qt,
            s.sources.mp4,
            s.sources.ffprobe,
            s.sources.fs
        );
    }

    if let Some((oldest, newest)) = s.span {
        println!(
            "  {} Span         : {} → {}",
            g.pick("🗓", "[spn]"),
            date_format::format_for_display(&oldest),
            date_format::format_for_display(&newest)
        );
    }

    println!(
        "  {} Output       : {}",
        g.pick("📂", "[out]"),
        style(s.output.display()).cyan()
    );
    println!("{}", style(&bar).green());
}

fn oldest_to_newest(
    groups: &[DateGroup],
) -> Option<(chrono::DateTime<chrono::Local>, chrono::DateTime<chrono::Local>)> {
    let first = groups.first()?.date;
    let last = groups.last()?.date;
    Some((first, last))
}

