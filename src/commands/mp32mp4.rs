//! module mp32mp4 - Convert MP3 files to MP4 videos with optional cover art.
//! Converts MP3 files to MP4 videos with optional cover art using the batch pipeline.

use crate::{
    cli::BatchArgs,
    commands::batch::{run_batch, BatchFuture, BatchTask, FileOutcome},
    ffmpeg::{args, Ffmpeg, ProcessRunner},
    util::files,
    workflow::{Workflow, WorkflowOptions},
};
use anyhow::Result;
use clap::Args;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

/// Default audio bitrate for MP3 encoding (kbps).
const DEFAULT_BITRATE: u32 = 320;

/// Prefix for temporary cover image files.
const TEMP_COVER_PREFIX: &str = "toolkitrs-cover-";

/// Suffix for temporary cover image files.
const TEMP_COVER_SUFFIX: &str = ".jpg";

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
#[derive(Debug, Clone, Copy)]
struct Mp3ToMp4Task {
    /// Audio bitrate in kbps.
    bitrate: u32,

    /// Whether to force overwrite existing files.
    force: bool,

    /// Whether to skip files without embedded cover art.
    no_cover_fallback: bool,
}

impl Mp3ToMp4Task {
    /// Returns the shared workflow execution settings for this task.
    fn workflow_options(self) -> WorkflowOptions {
        WorkflowOptions {
            force: self.force,
            bitrate: self.bitrate,
            no_cover_fallback: self.no_cover_fallback,
            ..WorkflowOptions::default()
        }
    }
}
impl BatchTask for Mp3ToMp4Task {
    fn input_extension(&self) -> &str {
        Workflow::Mp32Mp4.input_extension()
    }

    fn output_extension(&self) -> &str {
        Workflow::Mp32Mp4.output_extension()
    }

    fn file_type_name(&self) -> &str {
        Workflow::Mp32Mp4.file_type_name()
    }

    fn process_file<R: ProcessRunner>(
        &self,
        input: &Path,
        output: &Path,
        ffmpeg: &Ffmpeg<R>,
    ) -> Result<FileOutcome> {
        let cover = files::temp_path(TEMP_COVER_PREFIX, TEMP_COVER_SUFFIX)?;
        let has_cover = match ffmpeg.run(args::extract_embedded_cover(input, &cover)) {
            Ok(_) => has_file_content(&cover),
            Err(error) => {
                eprintln!(
                    "WARNING: Failed to extract cover art from {}: {}",
                    input.display(),
                    error
                );
                cleanup_temp_file(&cover);
                false
            }
        };

        if !has_cover && self.no_cover_fallback {
            cleanup_temp_file(&cover);
            return Ok(FileOutcome::Skipped(format!(
                "no cover art in {}",
                input.display()
            )));
        }

        let result = ffmpeg.run(args::encode_mp4(
            has_cover.then_some(cover.as_path()),
            input,
            output,
            self.bitrate,
            self.force,
        ));

        cleanup_temp_file(&cover);
        result?;
        Ok(FileOutcome::Success)
    }

    fn process_file_async(
        &self,
        input: PathBuf,
        output: PathBuf,
        ffmpeg_binary: PathBuf,
        cancel: CancellationToken,
    ) -> BatchFuture {
        let options = self.workflow_options();
        Box::pin(async move {
            Workflow::Mp32Mp4
                .run_async(input, output, &options, &ffmpeg_binary, cancel, None)
                .await
        })
    }
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Runs the MP3 to MP4 conversion using the generic batch pipeline.
pub fn run<R: ProcessRunner>(args_cli: Mp32mp4Args, ffmpeg: &Ffmpeg<R>) -> Result<()> {
    let task = Mp3ToMp4Task {
        bitrate: args_cli.bitrate,
        force: args_cli.batch.force,
        no_cover_fallback: args_cli.no_cover_fallback,
    };
    run_batch(&task, &args_cli.batch, args_cli.files, ffmpeg)
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Returns true when the file exists and contains data.
fn has_file_content(path: &Path) -> bool {
    path.metadata().map(|meta| meta.len() > 0).unwrap_or(false)
}

/// Best-effort temp file cleanup.
fn cleanup_temp_file(path: &Path) {
    let _ = fs::remove_file(path);
}
