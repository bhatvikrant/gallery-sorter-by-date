//! Rich terminal progress UI built on `indicatif` + `console`.
//!
//! Renders three live phase bars (scan / metadata / copy) with ETA, percent,
//! throughput, and per-source counters, plus an emoji-rich summary card at
//! the end. Automatically switches to one-line-per-phase output when stdout
//! isn't a TTY (e.g. piped to a log file). Honours the `NO_COLOR` env var
//! through `console`.

use crate::copier::{CopyEvent, CopyProgress};
use crate::metadata::MetadataProgress;
use crate::scanner::ScanProgress;
use crate::types::{DateSource, MediaFile, MediaMeta, MediaType};
use console::{style, Term};
use indicatif::{HumanBytes, MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct UiOptions {
    pub quiet: bool,
}

impl UiOptions {
    /// Auto-detects: live bars when stdout is a TTY, one-line-per-phase
    /// otherwise (CI, piped output, log files). Respects `NO_EMOJI=1` for
    /// terminals that mangle UTF-8.
    pub fn auto() -> Self {
        let attended = Term::stdout().features().is_attended();
        Self { quiet: !attended }
    }
}

/// Picks emoji vs ASCII based on the `NO_EMOJI` environment variable.
#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    no_emoji: bool,
}

impl Glyphs {
    pub fn new() -> Self {
        Self {
            no_emoji: std::env::var_os("NO_EMOJI").is_some(),
        }
    }
    pub fn pick(&self, emoji: &'static str, ascii: &'static str) -> &'static str {
        if self.no_emoji {
            ascii
        } else {
            emoji
        }
    }
}

impl Default for Glyphs {
    fn default() -> Self {
        Self::new()
    }
}

/// Top-level UI handle that owns the `MultiProgress` and individual bars.
pub struct Ui {
    pub opts: UiOptions,
    pub glyphs: Glyphs,
    multi: Option<MultiProgress>,
}

impl Ui {
    pub fn new(opts: UiOptions) -> Self {
        let multi = if opts.quiet {
            None
        } else {
            let m = MultiProgress::new();
            m.set_draw_target(ProgressDrawTarget::stdout());
            Some(m)
        };
        Self {
            opts,
            glyphs: Glyphs::new(),
            multi,
        }
    }

    pub fn print_banner(&self, source: &Path, output: &Path, cores: usize) {
        let g = self.glyphs;
        let bar = "═".repeat(63);
        println!("{}", style(&bar).cyan());
        println!(
            "  {} {}  {}  {}  {} {} cores",
            g.pick("📸", "[*]"),
            style("Gallery Sorter").bold().cyan(),
            style("·").dim(),
            style("Rust engine").magenta(),
            style("·").dim(),
            style(cores).bold()
        );
        println!("{}", style(&bar).cyan());
        println!(
            "  {} Source : {}",
            g.pick("📂", "[in] "),
            style(source.display()).dim()
        );
        println!(
            "  {} Output : {}",
            g.pick("📦", "[out]"),
            style(output.display()).dim()
        );
        println!();
    }

    pub fn phase_header(&self, idx: usize, total: usize, label: &str, emoji: &'static str) {
        let g = self.glyphs;
        println!(
            "{}  Phase {}/{}  {}  {}",
            style("▸").cyan().bold(),
            idx,
            total,
            g.pick(emoji, "*"),
            style(label).bold()
        );
    }

    fn add_bar(&self, pb: ProgressBar) -> ProgressBar {
        if let Some(m) = &self.multi {
            m.add(pb)
        } else {
            pb
        }
    }

    pub fn make_scan_bar(&self) -> Arc<ScanBar> {
        let pb = ProgressBar::new_spinner();
        pb.set_style(
            ProgressStyle::with_template(
                "  {spinner:.cyan} [{elapsed_precise}] {msg}",
            )
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        let pb = self.add_bar(pb);
        if self.opts.quiet {
            pb.set_draw_target(ProgressDrawTarget::hidden());
        } else {
            pb.enable_steady_tick(Duration::from_millis(100));
        }
        Arc::new(ScanBar {
            pb,
            files: AtomicU64::new(0),
            dirs: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            quiet: self.opts.quiet,
            glyphs: self.glyphs,
        })
    }

    pub fn make_metadata_bar(&self, total: u64, image_total: u64, video_total: u64) -> Arc<MetaBar> {
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::with_template(
                "  {spinner:.magenta} [{elapsed_precise}] [{bar:30.magenta/blue}] \
                 {human_pos}/{human_len} ({percent}%)  ETA {eta_precise}\n      {msg}",
            )
            .unwrap()
            .progress_chars("█▓▒░ ")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        let pb = self.add_bar(pb);
        if self.opts.quiet {
            pb.set_draw_target(ProgressDrawTarget::hidden());
        } else {
            pb.enable_steady_tick(Duration::from_millis(120));
        }
        Arc::new(MetaBar {
            pb,
            image_total,
            video_total,
            images_done: AtomicU64::new(0),
            videos_done: AtomicU64::new(0),
            src_exif: AtomicU64::new(0),
            src_qt: AtomicU64::new(0),
            src_mp4: AtomicU64::new(0),
            src_ffprobe: AtomicU64::new(0),
            src_fs: AtomicU64::new(0),
            quiet: self.opts.quiet,
            glyphs: self.glyphs,
        })
    }

    pub fn make_copy_bar(&self, total_files: u64, total_bytes: u64) -> Arc<CopyBar> {
        let pb = ProgressBar::new(total_files);
        pb.set_style(
            ProgressStyle::with_template(
                "  {spinner:.green} [{elapsed_precise}] [{bar:30.green/blue}] \
                 {human_pos}/{human_len} ({percent}%)  ETA {eta_precise}\n      {msg}",
            )
            .unwrap()
            .progress_chars("█▓▒░ ")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        let pb = self.add_bar(pb);
        if self.opts.quiet {
            pb.set_draw_target(ProgressDrawTarget::hidden());
        } else {
            pb.enable_steady_tick(Duration::from_millis(120));
        }
        Arc::new(CopyBar {
            pb,
            total_bytes,
            bytes_done: AtomicU64::new(0),
            copied: AtomicU64::new(0),
            deduped: AtomicU64::new(0),
            current: Mutex::new(String::new()),
            quiet: self.opts.quiet,
            glyphs: self.glyphs,
        })
    }
}

// ─── Scan ────────────────────────────────────────────────────────────────────

pub struct ScanBar {
    pb: ProgressBar,
    files: AtomicU64,
    dirs: AtomicU64,
    bytes: AtomicU64,
    quiet: bool,
    glyphs: Glyphs,
}

impl ScanBar {
    fn refresh(&self) {
        if self.quiet {
            return;
        }
        let g = self.glyphs;
        let files = self.files.load(Ordering::Relaxed);
        let dirs = self.dirs.load(Ordering::Relaxed);
        let bytes = self.bytes.load(Ordering::Relaxed);
        self.pb.set_message(format!(
            "{} {} files {} {} dirs {} {}",
            g.pick("📄", "F:"),
            human_count(files),
            g.pick("📁", "D:"),
            human_count(dirs),
            g.pick("·", "·"),
            HumanBytes(bytes)
        ));
    }

    pub fn finish(&self, image_count: usize, video_count: usize, dirs: usize, bytes: u64, elapsed: Duration) {
        self.pb.finish_and_clear();
        let g = self.glyphs;
        println!(
            "  {} Found {} images, {} videos across {} dirs ({}) in {}",
            g.pick("✅", "[ok]"),
            style(image_count).bold().green(),
            style(video_count).bold().green(),
            style(dirs).bold(),
            HumanBytes(bytes),
            style(format_dur(elapsed)).dim()
        );
        println!();
    }
}

impl ScanProgress for ScanBar {
    fn on_file(&self, file: &MediaFile) {
        self.files.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(file.size, Ordering::Relaxed);
        self.refresh();
    }
    fn on_dir(&self) {
        self.dirs.fetch_add(1, Ordering::Relaxed);
    }
}

// ─── Metadata ────────────────────────────────────────────────────────────────

pub struct MetaBar {
    pb: ProgressBar,
    image_total: u64,
    video_total: u64,
    images_done: AtomicU64,
    videos_done: AtomicU64,
    src_exif: AtomicU64,
    src_qt: AtomicU64,
    src_mp4: AtomicU64,
    src_ffprobe: AtomicU64,
    src_fs: AtomicU64,
    quiet: bool,
    glyphs: Glyphs,
}

impl MetaBar {
    pub fn snapshot(&self) -> MetaSnapshot {
        MetaSnapshot {
            exif: self.src_exif.load(Ordering::Relaxed),
            qt: self.src_qt.load(Ordering::Relaxed),
            mp4: self.src_mp4.load(Ordering::Relaxed),
            ffprobe: self.src_ffprobe.load(Ordering::Relaxed),
            fs: self.src_fs.load(Ordering::Relaxed),
        }
    }

    pub fn finish(&self, total: u64, elapsed: Duration) {
        let g = self.glyphs;
        let snap = self.snapshot();
        self.pb.finish_and_clear();
        println!(
            "  {} Extracted metadata for {} files in {}",
            g.pick("✅", "[ok]"),
            style(total).bold().green(),
            style(format_dur(elapsed)).dim()
        );
        println!(
            "      {} EXIF {}  {} QuickTime {}  {} MP4box {}  {} ffprobe {}  {} fs {}",
            g.pick("📅", "[exif]"),
            style(snap.exif).bold(),
            g.pick("🍎", "[qt]"),
            style(snap.qt).bold(),
            g.pick("📦", "[mp4]"),
            style(snap.mp4).bold(),
            g.pick("🎞", "[ff]"),
            style(snap.ffprobe).bold(),
            g.pick("🛟", "[fs]"),
            style(snap.fs).bold()
        );
        println!();
    }

    fn refresh(&self) {
        if self.quiet {
            return;
        }
        let g = self.glyphs;
        let i = self.images_done.load(Ordering::Relaxed);
        let v = self.videos_done.load(Ordering::Relaxed);
        let s = self.snapshot();
        let elapsed = self.pb.elapsed().as_secs_f64().max(0.001);
        let rate = (self.pb.position() as f64 / elapsed) as u64;
        self.pb.set_message(format!(
            "{} images {}/{}  {} videos {}/{}  {} {} files/s\n      {} EXIF {}  {} QT {}  {} MP4 {}  {} ff {}  {} fs {}",
            g.pick("🖼", "I"),
            i,
            self.image_total,
            g.pick("🎬", "V"),
            v,
            self.video_total,
            g.pick("⚡", "@"),
            human_count(rate),
            g.pick("📅", "[exif]"),
            s.exif,
            g.pick("🍎", "[qt]"),
            s.qt,
            g.pick("📦", "[mp4]"),
            s.mp4,
            g.pick("🎞", "[ff]"),
            s.ffprobe,
            g.pick("🛟", "[fs]"),
            s.fs
        ));
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MetaSnapshot {
    pub exif: u64,
    pub qt: u64,
    pub mp4: u64,
    pub ffprobe: u64,
    pub fs: u64,
}

impl MetadataProgress for MetaBar {
    fn on_completed(&self, meta: &MediaMeta) {
        match meta.file.media_type {
            MediaType::Image => {
                self.images_done.fetch_add(1, Ordering::Relaxed);
            }
            MediaType::Video => {
                self.videos_done.fetch_add(1, Ordering::Relaxed);
            }
        }
        match meta.date_source {
            DateSource::Exif => {
                self.src_exif.fetch_add(1, Ordering::Relaxed);
            }
            DateSource::QuickTime => {
                self.src_qt.fetch_add(1, Ordering::Relaxed);
            }
            DateSource::Mp4Box => {
                self.src_mp4.fetch_add(1, Ordering::Relaxed);
            }
            DateSource::Ffprobe => {
                self.src_ffprobe.fetch_add(1, Ordering::Relaxed);
            }
            DateSource::FileBirthtime | DateSource::FileMtime => {
                self.src_fs.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.pb.inc(1);
        self.refresh();
    }
}

// ─── Copy ────────────────────────────────────────────────────────────────────

pub struct CopyBar {
    pb: ProgressBar,
    total_bytes: u64,
    bytes_done: AtomicU64,
    copied: AtomicU64,
    deduped: AtomicU64,
    current: Mutex<String>,
    quiet: bool,
    glyphs: Glyphs,
}

impl CopyBar {
    pub fn finish(&self, elapsed: Duration) {
        self.pb.finish_and_clear();
        let g = self.glyphs;
        let copied = self.copied.load(Ordering::Relaxed);
        let deduped = self.deduped.load(Ordering::Relaxed);
        let bytes = self.bytes_done.load(Ordering::Relaxed);
        let secs = elapsed.as_secs_f64().max(0.001);
        let rate = (bytes as f64 / secs) as u64;
        println!(
            "  {} Copied {} files ({}{}) in {} · avg {}/s",
            g.pick("✅", "[ok]"),
            style(copied).bold().green(),
            HumanBytes(bytes),
            if deduped > 0 {
                format!(", {} deduped", deduped)
            } else {
                String::new()
            },
            style(format_dur(elapsed)).dim(),
            HumanBytes(rate)
        );
        println!();
    }

    fn refresh(&self) {
        if self.quiet {
            return;
        }
        let g = self.glyphs;
        let bytes = self.bytes_done.load(Ordering::Relaxed);
        let copied = self.copied.load(Ordering::Relaxed);
        let deduped = self.deduped.load(Ordering::Relaxed);
        let elapsed = self.pb.elapsed().as_secs_f64().max(0.001);
        let rate = (bytes as f64 / elapsed) as u64;
        let current = self.current.lock().expect("copy current mutex").clone();
        self.pb.set_message(format!(
            "{} {} / {} · {}/s · {} {} copied · {} {} deduped\n      {} {}",
            g.pick("💾", "[bytes]"),
            HumanBytes(bytes),
            HumanBytes(self.total_bytes),
            HumanBytes(rate),
            g.pick("✅", "+"),
            style(copied).green(),
            g.pick("♻️", "~"),
            style(deduped).yellow(),
            g.pick("↪", "->"),
            style(truncate_middle(&current, 70)).dim()
        ));
    }
}

impl CopyProgress for CopyBar {
    fn on_copy(&self, e: CopyEvent<'_>) {
        self.bytes_done.fetch_add(e.bytes, Ordering::Relaxed);
        if e.deduped {
            self.deduped.fetch_add(1, Ordering::Relaxed);
        } else {
            self.copied.fetch_add(1, Ordering::Relaxed);
        }
        if let Ok(mut g) = self.current.lock() {
            // Show a relative-ish path: last two components of dest_dir + name.
            let dir_tail = e
                .dest_dir
                .iter()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|s| s.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            *g = format!("{}/{}", dir_tail, e.final_name);
        }
        self.pb.inc(1);
        self.refresh();
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

pub fn format_dur(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        return format!("{:.1}s", d.as_secs_f64());
    }
    let m = secs / 60;
    let s = secs % 60;
    if m < 60 {
        return format!("{m}m {s}s");
    }
    let h = m / 60;
    let m = m % 60;
    format!("{h}h {m}m {s}s")
}

fn human_count(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let k = n as f64 / 1000.0;
    if k < 1000.0 {
        return format!("{k:.1}k");
    }
    let m = k / 1000.0;
    format!("{m:.1}M")
}

fn truncate_middle(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head = max / 2 - 2;
    let tail = max - head - 3;
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    out.extend(&chars[..head]);
    out.push_str("...");
    out.extend(&chars[chars.len() - tail..]);
    out
}
