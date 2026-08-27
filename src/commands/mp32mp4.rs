//! module mp32mp4 - Convert MP3 files to MP4 videos with optional cover art.
//! Converts MP3 files to MP4 videos with optional cover art using the batch pipeline.

use crate::{
    cli::BatchArgs,
    commands::{
        batch::{
            self, resolve_execution_mode, run_parallel_console, run_sequential, BatchTask,
            FileOutcome,
        },
        workers,
    },
    components::prompt::ExecutionMode,
    ffmpeg::{args, Ffmpeg, ProcessRunner},
    util::{files, output},
};
use anyhow::Result;
use clap::Args;
use std::{
    fs,
    path::{Path, PathBuf},
};

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Extension for MP3 audio files.
const MP3_EXT: &str = "mp3";

/// Extension for MP4 video files.
const MP4_EXT: &str = "mp4";

/// Human readable name for MP3 files.
const FILE_TYPE_NAME: &str = "MP3";

/// Default audio bitrate for MP3 encoding (kbps).
const DEFAULT_BITRATE: u32 = 320;

// ------------------------------------------ Types & Impls ------------------------------------- //

/// Arguments for the `mp32mp4` subcommand.
#[derive(Debug, Args)]
#[command(after_help = "Examples:
  toolkitrs mp32mp4                              Scan and process all .mp3 files in the current directory
  toolkitrs mp32mp4 song.mp3                    Process one file; prompt if sibling .mp3 files exist
  toolkitrs mp32mp4 --batch --input-dir /dir    Process all .mp3 files in /dir
  toolkitrs mp32mp4 --batch --on-error skip     Continue past errors and report at the end
  toolkitrs mp32mp4 --no-cover-fallback         Skip files without embedded cover art")]
pub struct Mp32mp4Args {
    /// Common batch options like output directory, force overwrite, and explicit batch scanning.
    #[command(flatten)]
    pub batch: BatchArgs,

    /// Audio bitrate in kbps for the MP4's audio stream.
    #[arg(long, default_value_t = DEFAULT_BITRATE)]
    pub bitrate: u32,

    /// Skip files without embedded cover art instead of using a black video.
    #[arg(long)]
    pub no_cover_fallback: bool,

    /// MP3 files to process.
    ///
    /// When omitted, the current directory is scanned for .mp3 files.
    #[arg(value_name = "FILE")]
    pub files: Vec<PathBuf>,
}

/// Task definition for MP3 to MP4 conversion.
struct Mp3ToMp4Task {
    /// Audio bitrate in kbps.
    bitrate: u32,

    /// Whether to force overwrite existing files.
    force: bool,

    /// Whether to skip files without embedded cover art.
    no_cover_fallback: bool,
}
impl BatchTask for Mp3ToMp4Task {
    fn input_extension(&self) -> &str {
        MP3_EXT
    }

    fn output_extension(&self) -> &str {
        MP4_EXT
    }

    fn file_type_name(&self) -> &str {
        FILE_TYPE_NAME
    }

    fn process_file<R: ProcessRunner>(
        &self,
        input: &Path,
        output: &Path,
        ffmpeg: &Ffmpeg<R>,
    ) -> Result<FileOutcome> {
        let cover = files::temp_path(workers::TEMP_COVER_PREFIX, workers::TEMP_COVER_SUFFIX)?;
        let has_cover = match ffmpeg.run(args::extract_embedded_cover(input, &cover)) {
            Ok(_) => {
                // FFmpeg succeeded, check if the file has content
                cover.metadata().map(|m| m.len() > 0).unwrap_or(false)
            }
            Err(e) => {
                // FFmpeg failed - log it so the user knows why
                eprintln!(
                    "WARNING: Failed to extract cover art from {}: {}",
                    input.display(),
                    e
                );
                // Clean up any garbage file
                let _ = fs::remove_file(&cover);
                false
            }
        };

        if !has_cover && self.no_cover_fallback {
            let _ = fs::remove_file(&cover);
            return Ok(FileOutcome::Skipped(format!(
                "no cover art in {}",
                input.display()
            )));
        }

        ffmpeg.run(args::encode_mp4(
            has_cover.then_some(cover.as_path()),
            input,
            output,
            self.bitrate,
            self.force,
        ))?;

        let _ = fs::remove_file(&cover);
        Ok(FileOutcome::Success)
    }
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Runs the MP3 to MP4 conversion using the generic batch pipeline.
///
/// The queue and error policy are resolved once, then dispatched to either
/// the sequential or the parallel runner.
pub fn run<R: ProcessRunner>(args_cli: Mp32mp4Args, ffmpeg: &Ffmpeg<R>) -> Result<()> {
    let task = Mp3ToMp4Task {
        bitrate: args_cli.bitrate,
        force: args_cli.batch.force,
        no_cover_fallback: args_cli.no_cover_fallback,
    };

    // One resolution pass: queue + error policy (may prompt for siblings).
    let (queue, policy) = batch::resolve_queue(&task, &args_cli.batch, args_cli.files.clone())?;

    if queue.is_empty() {
        return batch::report_empty_queue(FILE_TYPE_NAME);
    }

    match resolve_execution_mode(queue.len(), args_cli.batch.mode)? {
        ExecutionMode::Sequential => run_sequential(&task, &args_cli.batch, queue, policy, ffmpeg),
        ExecutionMode::Parallel => {
            output::ensure_directory(&args_cli.batch.output_dir)?;
            let job = workers::mp32mp4_job(
                &args_cli.batch.output_dir,
                args_cli.bitrate,
                args_cli.batch.force,
                args_cli.no_cover_fallback,
                ffmpeg.binary(),
            );
            run_parallel_console(FILE_TYPE_NAME, queue, job)
        }
    }
}
