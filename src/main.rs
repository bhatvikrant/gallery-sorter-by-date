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
use console::{strip_ansi_codes, style};
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
use crate::types::{DateGroup, MediaType, SkipKind, SkippedFile};

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
    let copy_failed_count = copy_result
        .skipped
        .iter()
        .filter(|s| s.kind == SkipKind::CopyFailed)
        .count();
    let copy_failed_images = copy_result
        .skipped
        .iter()
        .filter(|s| s.kind == SkipKind::CopyFailed && s.media_type == MediaType::Image)
        .count();
    let copy_failed_videos = copy_result
        .skipped
        .iter()
        .filter(|s| s.kind == SkipKind::CopyFailed && s.media_type == MediaType::Video)
        .count();

    print_summary_card(
        &ui,
        SummaryInputs {
            output: &output,
            total_elapsed,
            total_files,
            input_images: scan.image_count,
            input_videos: scan.video_count,
            input_bytes: scan.total_bytes,
            scanned_dirs: scan.directory_count,
            folders_created: copy_result.folders_created,
            files_copied: copy_result.files_copied,
            files_skipped: copy_result.files_skipped,
            bytes_copied: copy_result.bytes_copied,
            unsortable_copied: copy_result.unsortable_copied,
            copy_failed: copy_failed_count,
            copy_failed_images,
            copy_failed_videos,
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
    print_section_header(g.pick("⚠️", "[!]"), "SKIPPED FROM SORTING");
    println!();
    println!(
        "    {} files were not placed into a date folder ({} unsortable · {} copy errors)",
        style(skipped.len()).bold().yellow(),
        style(unsortable_total).bold(),
        style(failed_total).bold().red(),
    );
    println!();

    for entry in skipped.iter().take(MAX_PRINTED) {
        let tag_styled = match entry.kind {
            SkipKind::Unsortable => style(format!(" {:^12} ", "unsortable"))
                .black()
                .on_yellow()
                .to_string(),
            SkipKind::CopyFailed => style(format!(" {:^12} ", "copy failed"))
                .white()
                .on_red()
                .to_string(),
        };
        println!(
            "    {}  {}",
            tag_styled,
            style(entry.path.display()).bold(),
        );
        println!("    {}    {}", " ".repeat(14), style(&entry.reason).dim());
    }
    if skipped.len() > MAX_PRINTED {
        println!(
            "    {} {} more (full list omitted)",
            style("…").dim(),
            skipped.len() - MAX_PRINTED
        );
    }
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

    // ── Input (from scanner) ──
    input_images: usize,
    input_videos: usize,
    input_bytes: u64,
    scanned_dirs: usize,

    // ── Copy results ──
    folders_created: usize,
    files_copied: usize,
    files_skipped: usize, // dedup
    bytes_copied: u64,
    unsortable_copied: usize,
    copy_failed: usize,
    copy_failed_images: usize,
    copy_failed_videos: usize,

    sources: progress::MetaSnapshot,
    span: Option<(chrono::DateTime<chrono::Local>, chrono::DateTime<chrono::Local>)>,
}

const CARD_WIDTH: usize = 67;

fn print_summary_card(ui: &Ui, s: SummaryInputs<'_>) {
    let g = ui.glyphs;

    let secs = s.total_elapsed.as_secs_f64().max(0.001);
    let files_per_sec = (s.total_files as f64 / secs) as u64;
    let avg_bytes_per_sec = (s.bytes_copied as f64 / secs) as u64;

    let input_total = s.input_images + s.input_videos;
    // A file is "in the output" if we successfully placed it somewhere
    // (date folder OR unsortable bin), or if it was already there from a
    // prior run (deduped). The only files NOT in the output are the ones
    // whose copy outright failed.
    let output_images = s.input_images.saturating_sub(s.copy_failed_images);
    let output_videos = s.input_videos.saturating_sub(s.copy_failed_videos);
    let output_total = output_images + output_videos;
    let lost_total = s.copy_failed;
    let everything_ok = lost_total == 0;

    let title = format!(
        "{}  Sorted in {}   ·   {} files/s",
        if everything_ok {
            g.pick("✨", "*")
        } else {
            g.pick("⚠️", "!")
        },
        format_dur(s.total_elapsed),
        files_per_sec
    );

    println!();
    print_solid_bar(everything_ok);
    print_centred(&title, everything_ok);
    print_solid_bar(everything_ok);
    println!();

    // ── Input vs Output table — the headline answer to "did everything sort?"
    print_section_header(g.pick("📊", "[stats]"), "INPUT  →  OUTPUT");
    println!();
    print_table_header();
    print_table_row(
        g.pick("🖼", "[img]"),
        "Images",
        s.input_images,
        output_images,
    );
    print_table_row(
        g.pick("🎬", "[vid]"),
        "Videos",
        s.input_videos,
        output_videos,
    );
    print_table_divider();
    print_table_total(input_total, output_total, everything_ok);
    println!();
    print_verdict(input_total, output_total, lost_total, g);
    println!();

    // ── Output breakdown ─────────────────────────────────────────────────
    print_section_header(g.pick("📤", "[out]"), "OUTPUT BREAKDOWN");
    println!();
    print_kv(
        g.pick("✅", "[ok]"),
        "Copied into date folders",
        &style(s.files_copied).bold().green().to_string(),
    );
    print_kv(
        g.pick("🔁", "[dup]"),
        "Deduplicated (already in destination)",
        &dim_zero_or_yellow(s.files_skipped),
    );
    print_kv(
        g.pick("❓", "[uns]"),
        &format!("Copied into {}/", copier::UNSORTABLE_FOLDER),
        &dim_zero_or_yellow(s.unsortable_copied),
    );
    print_kv(
        g.pick("🚧", "[skp]"),
        "Skipped from sorting",
        &dim_zero_or_yellow(s.copy_failed + s.unsortable_copied),
    );
    if s.copy_failed > 0 {
        print_kv(
            g.pick("⛔", "[err]"),
            "Copy failures",
            &style(s.copy_failed).bold().red().to_string(),
        );
    }
    println!();
    print_kv(
        g.pick("📁", "[dir]"),
        "New folders created",
        &style(s.folders_created).bold().to_string(),
    );
    print_kv(
        g.pick("💾", "[siz]"),
        "Total bytes moved",
        &format!(
            "{}   {}   avg {}/s",
            style(HumanBytes(s.bytes_copied)).bold(),
            style("·").dim(),
            style(HumanBytes(avg_bytes_per_sec)).cyan()
        ),
    );

    // ── Date source attribution ──────────────────────────────────────────
    let total_src =
        s.sources.exif + s.sources.qt + s.sources.mp4 + s.sources.ffprobe + s.sources.fs;
    if total_src > 0 {
        println!();
        print_section_header(g.pick("📅", "[date]"), "DATE SOURCES");
        println!();
        print_kv(
            g.pick("📷", "[exf]"),
            "EXIF (image metadata)",
            &num_or_dim(s.sources.exif),
        );
        print_kv(
            g.pick("🍎", "[qt ]"),
            "QuickTime (mov / m4v)",
            &num_or_dim(s.sources.qt),
        );
        print_kv(
            g.pick("📦", "[mp4]"),
            "MP4box (mp4 / 3gp)",
            &num_or_dim(s.sources.mp4),
        );
        print_kv(
            g.pick("🎞", "[ff ]"),
            "ffprobe (avi / mkv / wmv / ...)",
            &num_or_dim(s.sources.ffprobe),
        );
        print_kv(
            g.pick("💿", "[fs ]"),
            "Filesystem timestamps",
            &num_or_dim(s.sources.fs),
        );
    }

    // ── Run details ──────────────────────────────────────────────────────
    println!();
    print_section_header(g.pick("📂", "[run]"), "RUN DETAILS");
    println!();
    if let Some((oldest, newest)) = s.span {
        print_kv(
            g.pick("📆", "[spn]"),
            "Date span",
            &format!(
                "{}   →   {}",
                style(date_format::format_for_display(&oldest)).cyan(),
                style(date_format::format_for_display(&newest)).cyan()
            ),
        );
    }
    print_kv(
        g.pick("📥", "[in ]"),
        "Source folders scanned",
        &format!(
            "{}   ({})",
            style(s.scanned_dirs).bold(),
            HumanBytes(s.input_bytes)
        ),
    );
    print_kv(
        g.pick("📤", "[out]"),
        "Output location",
        &style(s.output.display()).cyan().to_string(),
    );

    println!();
    print_solid_bar(everything_ok);
    println!();
}

// ─── Layout primitives ──────────────────────────────────────────────────────

fn print_solid_bar(ok: bool) {
    let line: String = std::iter::repeat('═').take(CARD_WIDTH).collect();
    let coloured = if ok {
        style(line).green()
    } else {
        style(line).yellow()
    };
    println!("{}", coloured);
}

fn print_centred(text: &str, ok: bool) {
    let w = visible_len(text);
    let pad = CARD_WIDTH.saturating_sub(w) / 2;
    let line = format!("{}{}", " ".repeat(pad), text);
    let styled = if ok {
        style(line).bold().green()
    } else {
        style(line).bold().yellow()
    };
    println!("{}", styled);
}

/// Section header: a bold/cyan label preceded by an emoji and followed by
/// trailing dashes that extend to the card edge, so each section is visually
/// framed identically to its neighbours.
fn print_section_header(emoji: &str, label: &str) {
    // Layout: "  <emoji>  <label>  <dashes>" — total visible width = CARD_WIDTH.
    let prefix_visible = 2 + visible_len(emoji) + 2 + visible_len(label) + 2;
    let dashes_needed = CARD_WIDTH.saturating_sub(prefix_visible).max(3);
    let dashes = "─".repeat(dashes_needed);
    println!(
        "  {}  {}  {}",
        emoji,
        style(label).bold().cyan(),
        style(dashes).dim()
    );
}

// ── Input/Output table ─────────────────────────────────────────────────────

const TABLE_INDENT: &str = "    ";
const COL_LABEL: usize = 22;
const COL_NUM: usize = 14;

fn print_table_header() {
    println!(
        "{}{}{}{}",
        TABLE_INDENT,
        " ".repeat(COL_LABEL),
        right_align(&style("Input").bold().dim().to_string(), COL_NUM),
        right_align(&style("Output").bold().dim().to_string(), COL_NUM),
    );
}

fn print_table_row(emoji: &str, label: &str, input: usize, output: usize) {
    let label_text = format!("{}  {}", emoji, label);
    let label_pad = COL_LABEL.saturating_sub(visible_len(&label_text));
    let in_cell = right_align(&format!("{}", input), COL_NUM);
    let out_cell = right_align(&cmp_text(output, input), COL_NUM);
    println!(
        "{}{}{}{}{}",
        TABLE_INDENT,
        label_text,
        " ".repeat(label_pad),
        in_cell,
        out_cell,
    );
}

fn print_table_divider() {
    let total_w = COL_LABEL + COL_NUM + COL_NUM;
    println!("{}{}", TABLE_INDENT, style("─".repeat(total_w)).dim());
}

fn print_table_total(input: usize, output: usize, ok: bool) {
    let label_text = "Total media";
    let label_pad = COL_LABEL.saturating_sub(visible_len(label_text));
    let in_cell = right_align(&style(input).bold().to_string(), COL_NUM);
    let out_styled = if ok {
        style(output).bold().green().to_string()
    } else {
        style(output).bold().yellow().to_string()
    };
    let out_cell = right_align(&out_styled, COL_NUM);
    println!(
        "{}{}{}{}{}",
        TABLE_INDENT,
        style(label_text).bold(),
        " ".repeat(label_pad),
        in_cell,
        out_cell,
    );
}

fn print_verdict(input: usize, output: usize, lost: usize, g: progress::Glyphs) {
    let in_word = if input == 1 { "file" } else { "files" };
    let out_word = if output == 1 { "file" } else { "files" };
    if lost == 0 {
        let line = format!(
            "  {}   {}   →   {}   {}",
            style(g.pick("✓", "[OK]")).bold().green(),
            style(format!("{} {} in", input, in_word)).bold(),
            style(format!("{} {} in output", output, out_word))
                .bold()
                .green(),
            style("(none lost)").dim(),
        );
        println!("{}", line);
    } else {
        let lost_word = if lost == 1 { "file" } else { "files" };
        let line = format!(
            "  {}   {} {} in   →   {} placed   ·   {}",
            style(g.pick("✗", "[!!]")).bold().red(),
            style(input).bold(),
            in_word,
            style(output).bold().yellow(),
            style(format!(
                "{} {} LOST — see skipped report above",
                lost, lost_word
            ))
            .bold()
            .red(),
        );
        println!("{}", line);
    }
}

// ── Key/Value rows with dot-leader alignment ───────────────────────────────

const KV_INDENT: &str = "    ";
const KV_LABEL_WIDTH: usize = 44;

fn print_kv(emoji: &str, label: &str, value: &str) {
    let prefix = format!("{}  {} ", emoji, label);
    let prefix_visible = visible_len(&prefix);
    // Dot leader fills the gap; minimum 2 dots so even the longest label is
    // visually separated from the value.
    let dots_needed = KV_LABEL_WIDTH
        .saturating_sub(prefix_visible)
        .saturating_sub(1)
        .max(2);
    let dots = ".".repeat(dots_needed);
    println!(
        "{}{}{} {}",
        KV_INDENT,
        prefix,
        style(dots).dim(),
        value,
    );
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn right_align(s: &str, width: usize) -> String {
    let visible = visible_len(s);
    let pad = width.saturating_sub(visible);
    format!("{}{}", " ".repeat(pad), s)
}

fn cmp_text(actual: usize, expected: usize) -> String {
    if actual == expected {
        style(actual).bold().green().to_string()
    } else if actual < expected {
        style(actual).bold().red().to_string()
    } else {
        style(actual).bold().to_string()
    }
}

fn dim_zero_or_yellow(n: usize) -> String {
    if n == 0 {
        style(n).dim().to_string()
    } else {
        style(n).bold().yellow().to_string()
    }
}

fn num_or_dim(n: u64) -> String {
    if n == 0 {
        style(n).dim().to_string()
    } else {
        style(n).bold().to_string()
    }
}

/// Visible terminal-cell width of `s`, ignoring ANSI escape sequences. We
/// deliberately roll our own (instead of `console::measure_text_width`) so
/// every emoji is counted as 2 cells regardless of how the underlying
/// unicode-width crate classifies it — terminals overwhelmingly render the
/// emoji we use as 2 cells, and disagreement between metrics and renderer
/// breaks column alignment in the summary table.
fn visible_len(s: &str) -> usize {
    let stripped = strip_ansi_codes(s);
    let mut w = 0usize;
    for c in stripped.chars() {
        // Variation Selector-16 (U+FE0F) is invisible — don't count it.
        if c == '\u{FE0F}' || c == '\u{200D}' {
            continue;
        }
        if c.is_ascii() {
            w += 1;
        } else {
            // Treat every non-ASCII (CJK, emoji, box-drawing >= U+1F000, etc.)
            // as 2 cells. Box-drawing characters U+2500..U+257F are actually
            // 1 cell, so override those.
            let cp = c as u32;
            if (0x2500..=0x257F).contains(&cp) || (0x2190..=0x21FF).contains(&cp) {
                w += 1; // box drawing / arrows
            } else if (0x0300..=0x036F).contains(&cp) {
                // combining marks
                continue;
            } else {
                w += 2;
            }
        }
    }
    w
}

fn oldest_to_newest(
    groups: &[DateGroup],
) -> Option<(chrono::DateTime<chrono::Local>, chrono::DateTime<chrono::Local>)> {
    let first = groups.first()?.date;
    let last = groups.last()?.date;
    Some((first, last))
}

