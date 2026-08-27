//! module mkv2mp3 - Convert MKV files to MP3 with optional cover art extracted from video.
//! Converts MKV files to MP3 with optional cover art using the batch pipeline.

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

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Default audio bitrate for MP3 encoding (kbps).
const DEFAULT_BITRATE: u32 = 320;

/// Default size (width and height) for extracted cover art.
const DEFAULT_COVER_SIZE: u32 = 600;

/// Prefix for temporary cover image files.
const TEMP_COVER_PREFIX: &str = "toolkitrs-cover-";

/// Suffix for temporary cover image files.
const TEMP_COVER_SUFFIX: &str = ".jpg";

/// Warning prefix for non-fatal issues like missing cover art.
const WARNING_PREFIX: &str = "WARNING";

/// Warning message for failed cover extraction.
const COVER_EXTRACTION_WARNING: &str = "cover extraction failed; continuing without cover";

// ------------------------------------------ Types & Impls ------------------------------------- //

/// Arguments for the `mkv2mp3` subcommand.
#[derive(Debug, Args)]
#[command(after_help = "Examples:
  toolkitrs mkv2mp3                              Scan and process all .mkv files in the current directory
  toolkitrs mkv2mp3 movie.mkv                   Process one file; prompt if sibling .mkv files exist
  toolkitrs mkv2mp3 --batch --input-dir /dir    Process all .mkv files in /dir
  toolkitrs mkv2mp3 --batch --on-error skip     Continue past errors and report at the end
  toolkitrs mkv2mp3 --cover-size 800            Extract larger cover art")]
pub struct Mkv2mp3Args {
    /// Common batch options like output directory, force overwrite, and explicit batch scanning.
    #[command(flatten)]
    pub batch: BatchArgs,

    /// Size (width and height) for the extracted cover art.
    #[arg(long, default_value_t = DEFAULT_COVER_SIZE)]
    pub cover_size: u32,

    /// Audio bitrate in kbps for the output MP3.
    #[arg(long, default_value_t = DEFAULT_BITRATE)]
    pub bitrate: u32,

    /// MKV files to process.
    ///
    /// When omitted, the current directory is scanned for .mkv files.
    #[arg(value_name = "FILE")]
    pub files: Vec<PathBuf>,
}

/// Task definition for MKV to MP3 extraction.
#[derive(Debug, Clone, Copy)]
struct MkvToMp3Task {
    /// Size in pixels for the extracted cover art square.
    cover_size: u32,

    /// Audio bitrate in kbps.
    bitrate: u32,

    /// Whether to force overwrite existing files.
    force: bool,
}

impl MkvToMp3Task {
    /// Returns the shared workflow execution settings for this task.
    fn workflow_options(self) -> WorkflowOptions {
        WorkflowOptions {
            force: self.force,
            bitrate: self.bitrate,
            cover_size: self.cover_size,
            ..WorkflowOptions::default()
        }
    }
}
impl BatchTask for MkvToMp3Task {
    fn input_extension(&self) -> &str {
        Workflow::Mkv2Mp3.input_extension()
    }

    fn output_extension(&self) -> &str {
        Workflow::Mkv2Mp3.output_extension()
    }

    fn file_type_name(&self) -> &str {
        Workflow::Mkv2Mp3.file_type_name()
    }

    fn process_file<R: ProcessRunner>(
        &self,
        input: &Path,
        output: &Path,
        ffmpeg: &Ffmpeg<R>,
    ) -> Result<FileOutcome> {
        let cover = files::temp_path(TEMP_COVER_PREFIX, TEMP_COVER_SUFFIX)?;
        let has_cover = ffmpeg
            .run(args::extract_frame(input, &cover, self.cover_size))
            .is_ok()
            && has_file_content(&cover);

        if !has_cover {
            eprintln!(
                "{WARNING_PREFIX}: {COVER_EXTRACTION_WARNING}: {}",
                input.display()
            );
        }

        let result = ffmpeg.run(args::encode_mp3(
            input,
            has_cover.then_some(cover.as_path()),
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
            Workflow::Mkv2Mp3
                .run_async(input, output, &options, &ffmpeg_binary, cancel, None)
                .await
        })
    }
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Runs the MKV to MP3 conversion using the generic batch pipeline.
pub fn run<R: ProcessRunner>(args_cli: Mkv2mp3Args, ffmpeg: &Ffmpeg<R>) -> Result<()> {
    let task = MkvToMp3Task {
        cover_size: args_cli.cover_size,
        bitrate: args_cli.bitrate,
        force: args_cli.batch.force,
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
