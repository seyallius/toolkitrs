//! module ts2mp4 - Convert TS files to MP4 via stream copy.
//! Converts TS files to MP4 via stream copy using the batch pipeline.

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
    util::output,
};
use anyhow::Result;
use clap::Args;
use std::path::{Path, PathBuf};

// --------------------------------- Types, Constants & Variables ------------------------------- //

/// Extension for transport stream files.
const TS_EXT: &str = "ts";

/// Extension for MP4 files.
const MP4_EXT: &str = "mp4";

/// Human readable name for TS files.
const FILE_TYPE_NAME: &str = "TS";

/// Arguments for the `ts2mp4` subcommand.
#[derive(Debug, Args)]
#[command(after_help = "Examples:
  toolkitrs ts2mp4                              Scan and process all .ts files in the current directory
  toolkitrs ts2mp4 video.ts                    Process one file; prompt if sibling .ts files exist
  toolkitrs ts2mp4 --batch --input-dir /dir    Process all .ts files in /dir
  toolkitrs ts2mp4 --batch --on-error skip     Continue past errors and report at the end")]
pub struct Ts2mp4Args {
    /// Common batch options like output directory, force overwrite, and explicit batch scanning.
    #[command(flatten)]
    pub batch: BatchArgs,

    /// TS files to process.
    ///
    /// When omitted, the current directory is scanned for .ts files.
    #[arg(value_name = "FILE")]
    pub files: Vec<PathBuf>,
}

/// Task definition for TS to MP4 remuxing.
struct Ts2Mp4Task {
    /// Whether to force overwrite existing files.
    force: bool,
}
impl BatchTask for Ts2Mp4Task {
    fn input_extension(&self) -> &str {
        TS_EXT
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
        ffmpeg.run(args::remux_copy(input, output, self.force))?;
        Ok(FileOutcome::Success)
    }
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Runs the TS → MP4 conversion using the generic batch pipeline.
///
/// The queue and error policy are resolved once, then dispatched to either
/// the sequential or the parallel runner.
pub fn run<R: ProcessRunner>(args_cli: Ts2mp4Args, ffmpeg: &Ffmpeg<R>) -> Result<()> {
    let task = Ts2Mp4Task {
        force: args_cli.batch.force,
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
            let job = workers::ts2mp4_job(
                &args_cli.batch.output_dir,
                args_cli.batch.force,
                ffmpeg.binary(),
            );
            run_parallel_console(FILE_TYPE_NAME, queue, job)
        }
    }
}
