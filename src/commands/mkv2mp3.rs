//! module mkv2mp3 - Convert MKV files to MP3 with optional cover art extracted from video.
//! Converts MKV files to MP3 with optional cover art using the batch pipeline.

use crate::{
    cli::BatchArgs,
    commands::batch::{run_batch, BatchTask, FileOutcome},
    ffmpeg::{args, Ffmpeg, ProcessRunner},
    util::files,
};
use anyhow::Result;
use clap::Args;
use std::{
    fs,
    path::{Path, PathBuf},
};

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Extension for Matroska video files.
const MKV_EXT: &str = "mkv";

/// Extension for MP3 audio files.
const MP3_EXT: &str = "mp3";

/// Human readable name for MKV files.
const FILE_TYPE_NAME: &str = "MKV";

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

/// Warning message template for failed cover extraction.
const COVER_EXTRACTION_WARNING: &str = "cover extraction failed for {}; continuing without cover";

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
struct MkvToMp3Task {
    /// Size in pixels for the extracted cover art square.
    cover_size: u32,

    /// Audio bitrate in kbps.
    bitrate: u32,

    /// Whether to force overwrite existing files.
    force: bool,
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

impl BatchTask for MkvToMp3Task {
    fn input_extension(&self) -> &str {
        MKV_EXT
    }

    fn output_extension(&self) -> &str {
        MP3_EXT
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
        let cover = files::temp_path(TEMP_COVER_PREFIX, TEMP_COVER_SUFFIX)?;
        let has_cover = ffmpeg
            .run(args::extract_frame(input, &cover, self.cover_size))
            .is_ok()
            && cover.metadata().map(|m| m.len() > 0).unwrap_or(false);

        if !has_cover {
            eprintln!(
                "{WARNING_PREFIX}: {COVER_EXTRACTION_WARNING} {}",
                input.display()
            );
        }

        ffmpeg.run(args::encode_mp3(
            input,
            has_cover.then_some(cover.as_path()),
            output,
            self.bitrate,
            self.force,
        ))?;

        let _ = fs::remove_file(&cover);
        Ok(FileOutcome::Success)
    }
}
