//! module cli - Defines command-line interface structures and parsing.
//! Uses clap derive macros to generate argument parsing, help text, and validation
//! from annotated struct definitions.

use crate::commands::{
    gh_contrib::GhContribArgs, mkv2mp3::Mkv2mp3Args, mp32mp4::Mp32mp4Args, ts2mp4::Ts2mp4Args,
    vidwrap::VidwrapArgs,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Command line entry point for the toolkitrs.
///
/// Global options defined here apply to all subcommands automatically.
#[derive(Debug, Parser)]
#[command(
    name = "toolkitrs",
    version,
    about = "FFmpeg workflows for common media tasks",
    long_about = "A unified FFmpeg workflow CLI for common media tasks.\n\nSupports single-file conversion, interactive sibling discovery, explicit directory batch processing, and configurable error policies."
)]
pub struct Cli {
    /// Print commands and diagnostic output to stderr.
    ///
    /// Useful for debugging failed conversions or verifying argument construction.
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Disable ANSI color codes and styling in all output.
    ///
    /// Automatically respected by banner, prompt, and spinner components.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Explicit path to the ffmpeg executable.
    ///
    /// Overrides PATH lookup when ffmpeg is installed in a non-standard location.
    #[arg(long, global = true)]
    pub ffmpeg_path: Option<PathBuf>,

    /// Explicit path to ffprobe binary.
    ///
    /// Reserved for future commands that need media inspection capabilities.
    #[arg(long, global = true)]
    pub ffprobe_path: Option<PathBuf>,

    /// The specific media workflow to execute. Omit to launch the interactive TUI.
    ///
    /// Each variant maps to a dedicated command module under src/commands/.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Available subcommands for the toolkitrs.
///
/// Adding a new tool requires adding a variant here and a dispatch arm in commands::run().
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Convert TS files to MP4 via stream copy (no re-encoding).
    Ts2mp4(Ts2mp4Args),
    /// Convert MKV files to MP3 with optional cover art extraction.
    Mkv2mp3(Mkv2mp3Args),
    /// Convert MP3 files to MP4 videos with embedded cover art as video track.
    Mp32mp4(Mp32mp4Args),
    /// Wrap a video with a companion image; supports interactive MP4 batch discovery.
    Vidwrap(VidwrapArgs),
    /// Fetch and export GitHub contributions (commits) for a user.
    GhContrib(GhContribArgs),
}

/// Common conversion options shared by batch processing commands.
///
/// These options are flattened into subcommand args. Field doc comments become
/// the generated `--help` text.
#[derive(Debug, Clone, Args)]
pub struct BatchArgs {
    /// Directory where converted media files will be written.
    ///
    /// Created automatically if it does not exist.
    #[arg(long, default_value = "out", value_name = "DIR")]
    pub output_dir: PathBuf,
    /// Overwrite existing output files instead of skipping them.
    ///
    /// By default, files with matching output paths are skipped to prevent accidental data loss.
    #[arg(long)]
    pub force: bool,
    /// Process all matching files in the current directory.
    ///
    /// Use --input-dir to scan a different directory.
    #[arg(long)]
    pub batch: bool,
    /// Directory to scan for input files.
    ///
    /// This implies batch processing.
    #[arg(long, value_name = "DIR")]
    pub input_dir: Option<PathBuf>,
    /// Error policy for explicit batch mode.
    ///
    /// stop: abort on first error and report.
    /// skip: continue past errors and report at the end.
    /// prompt: ask whether to continue after each processed file.
    #[arg(long, value_enum, value_name = "POLICY")]
    pub on_error: Option<BatchOnError>,
    /// Execution mode for multi-file batches.
    ///
    /// If omitted, interactive terminals prompt the user. Non-interactive
    /// terminals default to sequential.
    #[arg(long, value_enum, value_name = "MODE")]
    pub mode: Option<ExecutionModeCli>,
}

/// Error policy for explicit directory batch processing.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BatchOnError {
    /// Stop and report on first error.
    Stop,

    /// Skip errors and continue.
    Skip,

    /// Prompt after each file.
    Prompt,
}

/// Execution strategy for batch processing.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ExecutionModeCli {
    /// Sequential (one by one).
    Sequential,
    /// Parallel using all available cores.
    Parallel,
}
