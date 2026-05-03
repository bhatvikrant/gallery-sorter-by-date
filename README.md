# 📸 Gallery Sorter

<p align="center"><b>Organise a folder of photos & videos into clean date folders. Fast.</b></p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-1.74+-orange?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/license-MIT-yellow" alt="MIT License">
  <img src="https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-blue" alt="Platforms">
</p>

A single small command-line tool. Point it at a folder of photos and videos —
it reads each file's real capture date (EXIF, video metadata, or filesystem
fallback) and copies everything into a tidy `Year / Month / Day` tree.

```
Before                        After
─────                         ─────
IMG_1234.jpg                  sorted by date/
DSC_5678.NEF                    └── 2025/
random/                              ├── 1. Jan/
  vacation.mp4                       │   ├── 10th/
  photo.heic                         │   │   ├── IMG_1234.jpg
                                     │   │   └── vacation.mp4
                                     │   └── 15th/
                                     │       └── DSC_5678.NEF
                                     └── 2. Feb/
                                         └── 20th/
                                             └── photo.heic
```

> Originals are **always copied**, never moved or modified. Re-running is
> safe — duplicates are detected and skipped automatically.

---

## ⚡ How fast is it?

Real numbers, measured on an Apple M-series MacBook with APFS storage:

| Library           | Total size | Time   | Throughput     |
| ----------------- | ---------: | -----: | -------------: |
| 1,000 photos      |     2.9 GiB | **0.15 s** | ~7,000 files/s  |
| 5,000 photos      |    14.6 GiB | **0.74 s** | ~6,800 files/s  |
| 10,000 photos     |     ~30 GiB | ~2 s      | ~5,000 files/s  |
| 100,000 photos    |    ~300 GiB | ~30–60 s  | ~2,000 files/s  |

The huge throughput numbers come from three things working together:

1. **Reflinks (copy-on-write)** on APFS / Btrfs / XFS / ReFS — the
   filesystem shares data blocks instead of physically copying bytes. A
   100 GB library can be "copied" in a few seconds.
2. **Streaming EXIF** — only the first ~64 KB of each image is read to find
   the date, even for 100 MB RAW files.
3. **Full parallelism** — every CPU core works at once via `rayon` for
   scanning, metadata, and copying.

> 💡 On filesystems that don't support reflinks (FAT32, exFAT, network
> mounts, older ext4), the tool falls back to ordinary byte copies. In that
> case the bottleneck becomes your disk's write speed, not the tool. Expect
> ~50–150 MB/s on a USB SSD, and proportionally less on slower media.

---

## 📦 What it handles

| Type    | Formats                                                                            |
| ------- | ---------------------------------------------------------------------------------- |
| Photos  | `jpg` `jpeg` `png` `gif` `webp` `bmp` `tiff` `heic` `heif`                          |
| RAW     | `cr2` `cr3` `nef` `arw` `orf` `rw2` `dng` `raf`                                     |
| Videos  | `mp4` `mov` `m4v` `3gp` `avi` `mkv` `wmv` `flv` `webm`                              |

For dates it tries (in order):

- **Photos**: EXIF `DateTimeOriginal` → `CreateDate` → `DateTime` → file birth time → file modify time
- **MP4/MOV/M4V/3GP**: QuickTime `creationdate` → `mvhd` box → file birth time → file modify time
- **Other videos**: `ffprobe` (if installed) → file birth time → file modify time

---

## 🚀 Install — three steps

You install Rust once, then install Gallery Sorter, then you're done. Total
time: about 5 minutes.

### Step 1. Install Rust

Rust is a programming language; we need its compiler to build the tool. The
official installer ([rustup](https://rustup.rs)) handles everything in one
command.

<details open>
<summary><b>🍎 macOS / 🐧 Linux</b></summary>

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

</details>

<details>
<summary><b>🪟 Windows</b></summary>

In a regular PowerShell terminal:

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools
```

When the Visual Studio Build Tools installer opens, tick **"Desktop
development with C++"** (Rust uses its linker). Then close and reopen your
terminal so PATH picks up `cargo`.

</details>

Verify with:

```bash
cargo --version       # should print: cargo 1.74 or newer
```

### Step 2. Install Gallery Sorter

```bash
git clone https://github.com/yourusername/gallery-sorter-by-date.git
cd gallery-sorter-by-date
cargo install --path .
```

That's it. `cargo install` builds the optimised binary and drops it into
`~/.cargo/bin/gallery-sorter`, which is already on your PATH from Step 1.

> First build downloads dependencies and takes ~1 minute. After that the
> tool is just a single ~1 MB binary, fully self-contained.

### Step 3. (Optional) Install ffmpeg

Only needed if you want to read embedded dates from `.avi` `.mkv` `.wmv`
`.flv` or `.webm` videos. **Most people can skip this.** If you don't
install ffmpeg, those formats fall back to using the file's modification
date — everything else still works perfectly.

| OS              | Command                                |
| --------------- | -------------------------------------- |
| 🍎 macOS         | `brew install ffmpeg`                  |
| 🐧 Ubuntu/Debian | `sudo apt-get install -y ffmpeg`       |
| 🪟 Windows       | `winget install Gyan.FFmpeg`           |

---

## 💻 Use it

One command. One argument. That's the whole interface.

```bash
gallery-sorter /path/to/your/photos
```

Examples:

```bash
gallery-sorter ~/Pictures/iPhone-Backup
gallery-sorter /Volumes/SD-CARD/DCIM
gallery-sorter .                          # current folder
```

The sorted output appears in a new folder named `sorted by date/` inside
the folder you pointed at. Originals stay where they are.

To see help:

```bash
gallery-sorter --help
```

---

## 👀 What you'll see while it runs

```
═══════════════════════════════════════════════════════════════
  📸  Gallery Sorter  ·  Rust engine  ·  10 cores
═══════════════════════════════════════════════════════════════
  📂 Source : /Users/me/Pictures/Camera-Roll
  📦 Output : /Users/me/Pictures/Camera-Roll/sorted by date

▸ Phase 1/3  🔍  Scanning directories
  ⠹ [00:00:01] 12,438 files · 84 dirs · 1.4 GiB

▸ Phase 2/3  🧠  Extracting metadata
  ⠼ [00:00:03] [████████████░░░░░░░░] 4,212 / 12,438 (33.9%)  ETA 00:00:05
       🖼 images 3,108  🎬 videos 1,104  ⚡ 1,420 files/s
       📅 EXIF 2,901  🍎 QuickTime 612  📦 MP4box 410  🛟 fs 289

▸ Phase 3/3  📤  Copying into date folders
  ⠴ [00:00:04] [██████░░░░░░░░░░░░░░] 4,001 / 12,438 (32.1%)  ETA 00:00:08
       💾 1.12 GiB / 3.40 GiB · 280 MiB/s · ✅ 3,950 copied · ♻️ 49 deduped
       ↪ 2025/3. Mar/14th/IMG_4821.HEIC

═══════════════════════════════════════════════════════════════
  ✨  All done in 12.4s   ( 1003 files/s )
═══════════════════════════════════════════════════════════════
  🖼 Images       : 9,204
  🎬 Videos       : 3,234
  📁 Folders      : 312 created
  ✅ Copied       : 12,389
  ♻️ Deduplicated : 49
  💾 Total moved  : 14.7 GiB  ·  avg 1.18 GiB/s
  📅 Date sources : EXIF 8,901 · QuickTime 1,612 · MP4box 1,210 · fs 716
  🗓 Span         : 12 Jan 2019 → 28 Apr 2026
═══════════════════════════════════════════════════════════════
```

Each phase shows live progress with ETA, throughput, and a per-source
breakdown so you can see where dates came from. The summary card at the end
confirms what was done.

---

## 🔄 Updating

```bash
cd gallery-sorter-by-date
git pull
cargo install --path . --force
```

## 🗑️ Uninstalling

```bash
cargo uninstall gallery-sorter
```

---

## 🛠️ Troubleshooting

<details>
<summary><b><code>cargo: command not found</code></b></summary>

Rust either isn't installed or its `bin` directory isn't on your `PATH` yet.
Re-open your terminal, or run:

```bash
source "$HOME/.cargo/env"           # macOS / Linux
```

On Windows, just close and reopen the terminal after running `winget install
Rustlang.Rustup`.

</details>

<details>
<summary><b>Linux: <code>error: linker `cc` not found</code></b></summary>

```bash
sudo apt-get install -y build-essential pkg-config
```

</details>

<details>
<summary><b>Windows: <code>error: linker `link.exe` not found</code></b></summary>

Install Visual Studio Build Tools and tick "Desktop development with C++":

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools
```

</details>

<details>
<summary><b>"<code>ffprobe</code> not on PATH" warning during a run</b></summary>

You only need `ffprobe` if you're sorting `.avi` / `.mkv` / `.wmv` / `.flv`
/ `.webm` files **and** you want their embedded creation date (rather than
the file's modify time). Install ffmpeg if so — see Step 3 above. Otherwise
ignore the warning.

</details>

<details>
<summary><b>How do I redo the sort?</b></summary>

Originals are never touched, so just delete the output folder and re-run:

```bash
rm -rf "/path/to/your/photos/sorted by date"
gallery-sorter /path/to/your/photos
```

</details>

<details>
<summary><b>The sort took longer than the table above suggested</b></summary>

The headline numbers assume APFS / Btrfs / XFS / ReFS, where copies are
instantaneous (reflinks). On filesystems without reflink support — FAT32 or
exFAT (most USB sticks and SD cards), or network mounts — the tool falls
back to physically copying every byte, which is limited by your disk's
write speed. The metadata extraction is still fast either way.

To benefit from reflinks: keep the source folder on the same APFS/Btrfs/XFS
volume, not on an external drive. The output `sorted by date/` folder is
created inside the source folder, so it inherits that volume's filesystem
automatically.

</details>

<details>
<summary><b>The progress bars don't render — I just see scrolling text</b></summary>

That's intentional when stdout isn't an interactive terminal (CI, log
files, piped output). The tool detects this automatically and switches to
one-line-per-phase output.

For terminals that can't render emoji, set the env var:

```bash
NO_EMOJI=1 gallery-sorter /path/to/your/photos
```

</details>

---

## 🛡️ Safety

- **Non-destructive.** Originals are copied, never moved or deleted.
- **Duplicate-aware.** A file with the same name and same size at the
  destination is skipped, not overwritten.
- **Smart renaming.** If two source files have the same name but different
  contents, the second is renamed `name_1.ext`, `name_2.ext`, etc.
- **Idempotent.** Running twice on the same folder is harmless — the second
  run just prints "deduplicated" for everything.

---

## 🏗️ How it works under the hood

```
src/
├── main.rs              # CLI entry, phase orchestration, summary card
├── scanner.rs           # Parallel directory walk (jwalk)
├── metadata.rs          # Rayon par_iter dispatch over discovered files
├── exif_image.rs        # Streaming EXIF parser (kamadak-exif)
├── mp4_meta.rs          # In-process MP4/MOV/M4V/3GP date parser
├── ffprobe_fallback.rs  # ffprobe shell-out for AVI/MKV/WMV/FLV/WebM
├── grouper.rs           # Bucket files by day
├── copier.rs            # Parallel reflink-or-copy with per-folder dedup
├── progress.rs          # Live indicatif UI + summary card
├── date_format.rs       # Year/Month/Day path formatting
├── extensions.rs        # Format classification
└── types.rs             # Shared data types
```

Built with: [rayon](https://crates.io/crates/rayon),
[jwalk](https://crates.io/crates/jwalk),
[kamadak-exif](https://crates.io/crates/kamadak-exif),
[reflink-copy](https://crates.io/crates/reflink-copy),
[indicatif](https://crates.io/crates/indicatif),
[clap](https://crates.io/crates/clap),
[chrono](https://crates.io/crates/chrono).

---

## 📄 License

MIT. Use it however you like.
