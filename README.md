# 🛠️ toolkitrs

![GitHub Release Downloads](https://img.shields.io/github/downloads/seyallius/toolkitrs/total?label=downloads&logo=github&color=pink&style=for-the-badge)
![Latest Release Downloads](https://img.shields.io/github/downloads/seyallius/toolkit/latest/total?label=latest%20release&logo=github&style=for-the-badge)
![Crates.io Downloads](https://img.shields.io/crates/d/toolkitrs?label=cargo%20installs&logo=rust&color=orange&style=for-the-badge)
![GitHub Stars](https://img.shields.io/github/stars/seyallius/toolkit?style=social)

> A unified FFmpeg workflow CLI and interactive TUI for common media tasks.

A fast, single-binary replacement for mixed PowerShell/Go media scripts. Built with Rust, it provides safe,
cross-platform media conversion workflows with progress feedback, interactive batch processing, a beautiful Terminal UI,
and rich CLI help.

## ✨ Features

- **Interactive TUI**: A beautiful, keyboard-driven terminal UI for browsing files and running workflows with live
  progress
- **Unified CLI**: One binary for all media workflows (`ts2mp4`, `mkv2mp3`, `mp32mp4`, `vidwrap`)
- **Batch Processing**: Automatic directory scanning with skip/overwrite logic
- **Interactive Sibling Discovery**: If you provide one file and related files exist nearby, toolkitrs can ask whether
  to process them too
- **Error Policies**: Choose whether to stop, skip, or prompt after each file during batch processing
- **Safe Defaults**: Prevents accidental data loss with explicit `--force` flags
- **Rich UX**: Spinners, progress lines, colored output, and interactive prompts
- **CI/CD Friendly**: Non-interactive mode avoids hanging and chooses safe defaults
- **Testable Core**: Dependency-injected FFmpeg runner for deterministic testing
- **Zero Runtime Dependencies**: Single static binary, no PowerShell or Go required

## 📦 Installation

### Quick Install (Linux / macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/seyallius/toolkitrs/main/install.sh | bash
```

*To install a specific version:*

```bash
curl -fsSL https://raw.githubusercontent.com/seyallius/toolkitrs/main/install.sh | bash -s v0.1.8
```

### Quick Install (Windows)

Open PowerShell and run:

```powershell
irm https://raw.githubusercontent.com/seyallius/toolkitrs/main/install.ps1 | iex
```

*To install a specific version:*

```powershell
iex "& { $(irm https://raw.githubusercontent.com/seyallius/toolkitrs/main/install.ps1) } -Version v0.1.8"
```

### Pre-built Binary (via Cargo Binstall)

```bash
# Install cargo-binstall first if you haven't
cargo install cargo-binstall

# Then install toolkitrs instantly
cargo binstall toolkitrs
```

### From crates.io (Compile from source)

```bash
cargo install toolkitrs
```

### From Source

```bash
git clone https://github.com/seyallius/toolkitrs.git
cd toolkitrs
cargo install --path .
```

### Prerequisites

- [FFmpeg](https://ffmpeg.org/download.html) must be installed and available in your PATH
- Optional: Use `--ffmpeg-path <PATH>` to specify a custom FFmpeg location

## 🚀 Usage

You can use `toolkitrs` in two ways: via the **Interactive TUI** (default) or via **CLI subcommands** for scripting and
automation.

```bash
# Launch the interactive TUI (just run it without arguments!)
toolkitrs

# Or run a specific CLI workflow
toolkitrs [OPTIONS] <COMMAND>
```

Run help for CLI commands:

```bash
toolkitrs --help
toolkitrs ts2mp4 --help
toolkitrs mkv2mp3 --help
toolkitrs mp32mp4 --help
toolkitrs vidwrap --help
```

## 🖥️ Interactive Terminal UI (TUI)

If you prefer a visual, keyboard-driven experience, simply run `toolkitrs` without any arguments to launch the
interactive Terminal UI.

The TUI provides:

- **Workflow Selection**: Choose between `ts2mp4`, `mkv2mp3`, `mp32mp4`, and `vidwrap`.
- **Visual File Browser**: Navigate your directories and select files using vim-style bindings (`h/j/k/l`).
- **Live Execution**: Watch real-time `ffmpeg` logs and per-file progress gauges as your batch processes.

> **Note:** The TUI requires an interactive terminal. If run in a CI/CD pipeline or piped to a file, it will safely exit
> and suggest using the CLI subcommands instead.

## 🔁 Interactive Batch Processing (CLI)

For supported CLI commands, if you provide a single input file and toolkitrs finds compatible sibling files in the same
directory, it can ask what to do.

Example:

```bash
toolkitrs ts2mp4 video.ts
```

If the directory contains more `.ts` files, toolkitrs prompts:

```text
Found 4 TS files in /videos
How would you like to proceed?
  1. Process the input only
  2. Process the whole path (return on error, report at the end)
  3. Process the whole path (skip on error, report at the end)
  4. Process the whole path (prompt user on each new video)
Choice [4]:
```

The default is option 4: process the whole path, prompting after each file.

When prompting after each file, toolkitrs asks:

```text
Continue?
  1. yes
  2. no
  3. yes to all
Choice [1]:
```

Choosing `yes to all` continues with all remaining files without prompting again.

### Explicit Batch Mode

You can also explicitly process a whole directory:

```bash
toolkitrs ts2mp4 --batch
toolkitrs ts2mp4 --input-dir /videos
```

Explicit batch commands support an error policy:

```bash
toolkitrs ts2mp4 --batch --on-error stop
toolkitrs ts2mp4 --batch --on-error skip
toolkitrs ts2mp4 --batch --on-error prompt
```

| Policy   | Behavior                                                     |
|----------|--------------------------------------------------------------|
| `stop`   | Stop on first error and print a report                       |
| `skip`   | Skip failed files and continue, printing a report at the end |
| `prompt` | Ask after each processed file whether to continue            |

If `--on-error` is omitted:

- Interactive terminals default to `prompt`
- Non-interactive terminals default to `skip`

## 📌 Batch Arguments

All batch commands (`ts2mp4`, `mkv2mp3`, `mp32mp4`) share these options:

| Flag                  | Default | Description                                           |
|-----------------------|---------|-------------------------------------------------------|
| `--output-dir <DIR>`  | `./out` | Directory for converted files (created automatically) |
| `--force`             | `false` | Overwrite existing output files instead of skipping   |
| `--batch`             | `false` | Process all matching files in the current directory   |
| `--input-dir <DIR>`   | None    | Process all matching files in the given directory     |
| `--on-error <POLICY>` | None    | Batch error policy: `stop`, `skip`, or `prompt`       |

## 🧰 Commands

### `ts2mp4` — Remux TS to MP4

Convert Transport Stream files to MP4 via stream copy (no re-encoding).

```bash
# Convert all .ts files in current directory
toolkitrs ts2mp4

# Convert a specific file
toolkitrs ts2mp4 video.ts

# Convert specific files to custom output directory
toolkitrs ts2mp4 --output-dir ./converted video1.ts video2.ts

# Explicitly batch-process a directory
toolkitrs ts2mp4 --batch --input-dir ./videos

# Overwrite existing outputs
toolkitrs ts2mp4 --force
```

### `mkv2mp3` — Extract Audio from MKV

Convert MKV files to MP3 with optional cover art extracted from video frames.

```bash
# Default: 320kbps, 600px cover art
toolkitrs mkv2mp3

# Custom bitrate and cover size
toolkitrs mkv2mp3 --bitrate 256 --cover-size 800 movie.mkv

# Explicitly batch-process a directory
toolkitrs mkv2mp3 --batch --input-dir ./videos

# Force overwrite existing MP3s
toolkitrs mkv2mp3 --force
```

### `mp32mp4` — Create Video from MP3

Convert MP3 files to MP4 videos using embedded cover art as the video track.

```bash
# Use embedded cover art (black video fallback if missing)
toolkitrs mp32mp4

# Skip files without embedded cover art
toolkitrs mp32mp4 --no-cover-fallback

# Custom audio bitrate
toolkitrs mp32mp4 --bitrate 256 song.mp3

# Explicitly batch-process a directory
toolkitrs mp32mp4 --batch --input-dir ./audio
```

### `vidwrap` — Wrap Video with Thumbnail

Combine a video with a companion image to create a new video with an embedded thumbnail.

```bash
# Requires a same-basename image (e.g., video.mp4 + video.jpg)
toolkitrs vidwrap video.mp4

# Batch-process all MP4 videos in the current directory
toolkitrs vidwrap --batch

# Batch-process all MP4 videos in a specific directory
toolkitrs vidwrap --input-dir /videos

# Batch-process and skip errors
toolkitrs vidwrap --batch --on-error skip
```

Supported companion image formats, in priority order:

`.png`, `.jpg`, `.jpeg`, `.bmp`, `.gif`, `.webp`

Notes:

- Single-file mode preserves the interactive post-processing prompt.
- Batch mode keeps original and source image files by default to avoid destructive prompts.
- Previously generated `*_with_image.mp4` files are excluded from batch discovery.

## 🔧 Development

```bash
# Build debug binary
cargo build

# Run tests
cargo test

# Clippy linting
cargo clippy --all-targets -- -D warnings

# Format check
cargo fmt --check
```

# License

[MIT OR Apache-2.0](./LICENSE)
