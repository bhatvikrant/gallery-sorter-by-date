# 📸 Media Gallery Sorter by Date

<p align="center">
  <strong>Automatically organize your photos and videos into beautifully named date-based folders</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Node.js-18+-green?logo=node.js" alt="Node.js 18+">
  <img src="https://img.shields.io/badge/TypeScript-5.0+-blue?logo=typescript" alt="TypeScript">
  <img src="https://img.shields.io/badge/License-MIT-yellow" alt="MIT License">
</p>

---

## ✨ Features

| Feature                      | Description                                                                                       |
| ---------------------------- | ------------------------------------------------------------------------------------------------- |
| ⚡ **Rust Engine (default)** | Multi-threaded native engine: streaming EXIF, in-process MP4/MOV parsing, parallel copies         |
| 🔍 **Smart Scanning**        | Recursively finds all media files in directories and subdirectories                               |
| 📅 **EXIF & Video Metadata** | Extracts actual capture dates from photo EXIF and video metadata                                  |
| 📁 **Hierarchical Folders**  | Organizes into `Year/Month/Day` structure (e.g., `2025/Jan/10th`)                                 |
| 🔄 **Duplicate Handling**    | Automatically renames duplicates and skips identical files                                        |
| ⏱️ **Live Progress**         | Per-phase bars with ETA, throughput, MB/s, and a per-source breakdown                             |
| 🎬 **Video Support**         | Full support for MP4, MOV, AVI, MKV and more                                                      |
| 📷 **RAW Support**           | Works with professional RAW formats (CR2, NEF, ARW, etc.)                                         |
| 🛟 **Legacy Mode**           | Original pure-Node implementation still available via `--legacy` for environments without Rust    |

---

## 🎯 How It Works

```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  📂 Your Messy  │ ──▶ │  🔍 Scan & Read  │ ──▶ │  📁 Organized   │
│     Folder      │     │    Metadata      │     │    by Date      │
└─────────────────┘     └──────────────────┘     └─────────────────┘

Before:                          After:
├── IMG_1234.jpg                 └── sorted by date/
├── video.mp4                        └── 2025/
├── DSC_5678.NEF                         ├── Jan/
├── random/                              │   ├── 10th/
│   └── photo.heic                       │   │   ├── IMG_1234.jpg
└── ...                                  │   │   └── video.mp4
                                         │   └── 15th/
                                         │       └── DSC_5678.NEF
                                         └── Feb/
                                             └── 20th/
                                                 └── photo.heic
```

---

## 📦 Supported Formats

### 🖼️ Images

| Type            | Extensions                                              |
| --------------- | ------------------------------------------------------- |
| **Common**      | `.jpg` `.jpeg` `.png` `.gif` `.webp` `.bmp` `.tiff`     |
| **Apple HEIC**  | `.heic` `.heif`                                         |
| **RAW Formats** | `.cr2` `.cr3` `.nef` `.arw` `.orf` `.rw2` `.dng` `.raf` |

### 🎬 Videos

| Type       | Extensions                                        |
| ---------- | ------------------------------------------------- |
| **Common** | `.mp4` `.mov` `.avi` `.mkv` `.wmv` `.flv` `.webm` |
| **Mobile** | `.3gp` `.m4v`                                     |

---

## 🚀 Quick Start

### Prerequisites

Before you begin, ensure you have:

- ✅ **Node.js 18** or higher installed
- ✅ **Rust toolchain** ([rustup.rs](https://rustup.rs)) — required to build the fast default engine. Skip if you only plan to run with `--legacy`.
- ✅ **FFmpeg** installed (only required for the legacy engine, or when sorting AVI/MKV/WMV/FLV/WebM videos under the new engine)

### Installing FFmpeg

<details>
<summary>🍎 macOS</summary>

```bash
brew install ffmpeg
```

</details>

<details>
<summary>🐧 Ubuntu/Debian</summary>

```bash
sudo apt install ffmpeg
```

</details>

<details>
<summary>🪟 Windows</summary>

1. Download from [ffmpeg.org](https://ffmpeg.org/download.html)
2. Extract and add the `bin` folder to your system PATH
</details>

### Installation

```bash
# 1️⃣ Clone or download this repository
git clone https://github.com/yourusername/gallery-sorter-by-date.git
cd gallery-sorter-by-date

# 2️⃣ Install dependencies
npm install

# 3️⃣ Build everything (TypeScript dispatcher + Rust engine)
npm run build
```

> 💡 `npm run build` runs both `npm run build:ts` and `npm run build:native`. The
> first Rust compile downloads crates and takes ~30s; subsequent builds are
> instant. If you don't have Rust installed and only want the legacy engine,
> run `npm run build:ts` alone.

---

## 💻 Usage

### Basic Usage

```bash
# Sort media in a specific directory (uses the fast Rust engine by default)
npm start /path/to/your/photos

# Sort media in current directory
npm start .
```

### Switching engines

```bash
# Default: fast Rust engine
npm start ~/Pictures/iPhone-Backup

# Force the original Node.js implementation
npm start ~/Pictures/iPhone-Backup -- --legacy

# Quiet mode (no live bars; one line per phase)
npm start ~/Pictures/iPhone-Backup -- --quiet

# ASCII-only output (for log shippers / restricted terminals)
npm start ~/Pictures/iPhone-Backup -- --no-emoji
```

> Note the `--` before `--legacy`/`--quiet`/`--no-emoji`: that's npm's
> separator so the flag is forwarded to the script rather than consumed by npm.

### Development Mode

```bash
# Run the TypeScript dispatcher with ts-node (no build required)
npm run dev /path/to/your/photos -- --legacy
```

### Example

```bash
# Organize your iPhone photo dump
npm start ~/Pictures/iPhone-Backup

# Organize your camera's SD card
npm start /Volumes/SD-CARD/DCIM
```

---

## 🚀 Performance

The new default **Rust engine** is dramatically faster than the original
TypeScript pipeline because it eliminates the three biggest bottlenecks:

| Bottleneck (legacy)                                | Fix (Rust engine)                                                    |
| -------------------------------------------------- | -------------------------------------------------------------------- |
| `fs.readFile(file)` reads the **entire** RAW file just to parse 64KB of EXIF | Streams via `BufReader` → ~64 KB of I/O per image regardless of size |
| `ffprobe` is spawned as a child process **per video** (~50–150 ms each) | In-process pure-Rust ISOBMFF parser handles `.mp4` `.mov` `.m4v` `.3gp` |
| Files are copied **sequentially** (single-threaded loop)         | Rayon `par_iter` copies across all CPU cores in parallel             |
| `pLimit(10)` caps even the parallel-ish steps      | A real thread pool sized to `num_cpus` (typically 8–16 on modern Macs) |

Realistic speedup on a 1000+ file mixed RAW + video library: **20–100×**.

### What you'll see live

Every phase reports as it runs:

- **Scan** — files found, dirs walked, total bytes discovered, files/sec
- **Metadata** — done/total, %, ETA, image vs video counters, files/sec, and a
  live breakdown of where each date came from (EXIF / QuickTime keys / MP4 box
  / ffprobe / filesystem fallback)
- **Copy** — done/total, %, ETA, bytes done / total, MB/sec, copied vs
  deduped, current path being written

When all three phases finish, a summary card prints with totals, total bytes
moved, average MB/s, the date-source breakdown, the date span of your library,
and (when applicable) an "≈ N× faster than legacy" estimate.

### When to use `--legacy`

- You can't (or don't want to) install the Rust toolchain.
- You're on a platform the prebuilt binary doesn't cover yet.
- You're debugging a difference between the two engines.

The legacy engine remains fully functional and produces byte-for-byte
identical folder layouts (same `Year/Month/Day` structure, same dedup rules,
same date-source priority).

---

## 📋 Output Example

```
📁 Media Gallery Sorter

Source: /Users/john/Pictures/Camera-Roll

✔ Found 156 images and 23 videos across 12 directories (1.2s)
✔ Processed 179 files (3.8s)

Grouping by date...
├── 10th Jan 2025 (23 images, 5 videos)
├── 11th Jan 2025 (45 images, 8 videos)
├── 15th Feb 2025 (88 images, 10 videos)

Copying files...
Output: /Users/john/Pictures/Camera-Roll/sorted by date

✔ Created 5 folders, copied 179 files (2.1s)

✓ Done! Media organized in /Users/john/Pictures/Camera-Roll/sorted by date
Total time: 7.1s
```

### 📂 Result Structure

```
sorted by date/
└── 2025/
    ├── Jan/
    │   ├── 10th/
    │   │   ├── IMG_1234.jpg
    │   │   └── vacation.mp4
    │   └── 11th/
    │       └── ...
    └── Feb/
        └── 15th/
            └── ...
```

---

## 🗂️ Folder Structure

Files are organized in a **Year → Month → Day** hierarchy:

```
sorted by date/
└── 2025/                    # 📅 Year
    ├── Jan/                 # 📆 Month (short name)
    │   ├── 1st/             # 📌 Day with ordinal
    │   ├── 15th/
    │   └── 31st/
    ├── Feb/
    │   └── 14th/
    └── Dec/
        └── 25th/
```

### Day Ordinal Suffixes

| Day         | Suffix | Examples       |
| ----------- | ------ | -------------- |
| 1, 21, 31   | `st`   | `1st`, `21st`  |
| 2, 22       | `nd`   | `2nd`, `22nd`  |
| 3, 23       | `rd`   | `3rd`, `23rd`  |
| 4-20, 24-30 | `th`   | `10th`, `15th` |

---

## 🔬 Date Extraction Priority

The tool uses smart date detection with multiple fallbacks:

### 📷 For Images

```
1. 🥇 EXIF DateTimeOriginal  ← When the photo was actually taken
2. 🥈 EXIF CreateDate        ← Camera creation timestamp
3. 🥉 EXIF DateTime          ← General datetime field
4. 📁 File birthtime         ← File system creation date
5. 📁 File mtime             ← Last modified date (last resort)
```

### 🎬 For Videos

```
1. 🥇 creation_time          ← Video metadata timestamp
2. 🥈 QuickTime creationdate ← iPhone MOV files
3. 🥉 File birthtime         ← File system creation date
4. 📁 File mtime             ← Last modified date (last resort)
```

---

## 🛡️ Safety Features

| Feature                    | Description                                           |
| -------------------------- | ----------------------------------------------------- |
| ✅ **Non-destructive**     | Original files are **copied**, never moved or deleted |
| ✅ **Duplicate Detection** | Skips files that already exist with same size         |
| ✅ **Smart Renaming**      | Handles filename conflicts by adding `_1`, `_2`, etc. |
| ✅ **Organized Output**    | All sorted files go into `sorted by date/` folder     |

---

## 🏗️ Project Structure

```
gallery-sorter-by-date/
├── 📁 src/                       # 🟦 TypeScript dispatcher + legacy engine
│   ├── index.ts                  # 🚀 Routes to Rust by default, --legacy → Node
│   ├── scanner.ts                # 🔍 Directory scanner (legacy)
│   ├── metadata.ts               # 📅 EXIF & video metadata (legacy)
│   ├── dateFormatter.ts          # 📝 Date formatting (legacy)
│   ├── grouper.ts                # 📊 Group files by date (legacy)
│   └── copier.ts                 # 📋 File copy with deduplication (legacy)
├── 📁 native/                    # 🦀 Default high-performance engine (Rust)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs               # CLI + phase orchestration + summary card
│       ├── scanner.rs            # jwalk parallel walk
│       ├── metadata.rs           # Rayon par_iter dispatch
│       ├── exif_image.rs         # Streaming EXIF (kamadak-exif)
│       ├── mp4_meta.rs           # In-process mp4/mov/m4v/3gp parser
│       ├── ffprobe_fallback.rs   # ffprobe for avi/mkv/wmv/flv/webm
│       ├── grouper.rs            # Bucket by date
│       ├── copier.rs             # Rayon par_iter copy + per-dir dedup
│       ├── progress.rs           # indicatif live UI
│       ├── date_format.rs        # Mirrors src/dateFormatter.ts
│       ├── extensions.rs         # Mirrors src/constants.ts
│       └── types.rs              # Shared types
├── 📁 dist/                      # 📦 Compiled TypeScript
├── package.json
├── tsconfig.json
└── README.md
```

---

## 🧰 Available Scripts

| Script         | Command                  | Description                                                  |
| -------------- | ------------------------ | ------------------------------------------------------------ |
| `build`        | `npm run build`          | Build everything (TypeScript dispatcher + Rust engine)       |
| `build:ts`     | `npm run build:ts`       | Compile TypeScript only                                      |
| `build:native` | `npm run build:native`   | Compile the Rust engine in release mode (`cargo build`)      |
| `start`        | `npm start`              | Run the dispatcher (Rust by default; pass `-- --legacy` for Node) |
| `dev`          | `npm run dev`            | Run the TypeScript dispatcher with ts-node                   |

---

## 🤝 Contributing

Contributions are welcome! Feel free to:

1. 🍴 Fork the repository
2. 🌿 Create a feature branch (`git checkout -b feature/amazing-feature`)
3. 💾 Commit your changes (`git commit -m 'Add amazing feature'`)
4. 📤 Push to the branch (`git push origin feature/amazing-feature`)
5. 🔃 Open a Pull Request

---

## 📄 License

This project is licensed under the **MIT License** - see the [LICENSE](LICENSE) file for details.

---

<p align="center">
  Made with ❤️ for photographers and videographers everywhere
</p>

<p align="center">
  ⭐ Star this repo if you find it useful!
</p>
