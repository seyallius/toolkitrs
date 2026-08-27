//! module ts2mp4 - Convert TS files to MP4 via stream copy.
//! Converts TS files to MP4 via stream copy using the batch pipeline.

use crate::{
    cli::BatchArgs,
    commands::batch::{run_batch, BatchFuture, BatchTask, FileOutcome},
    ffmpeg::{args, Ffmpeg, ProcessRunner},
    workflow::{Workflow, WorkflowOptions},
};
use anyhow::Result;
use clap::Args;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

// ------------------------------------------ Types & Impls ------------------------------------- //

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
#[derive(Debug, Clone, Copy)]
struct Ts2Mp4Task {
    /// Whether to force overwrite existing files.
    force: bool,
}

impl Ts2Mp4Task {
    /// Returns the shared workflow execution settings for this task.
    fn workflow_options(self) -> WorkflowOptions {
        WorkflowOptions {
            force: self.force,
            ..WorkflowOptions::default()
        }
    }
}
impl BatchTask for Ts2Mp4Task {
    fn input_extension(&self) -> &str {
        Workflow::Ts2Mp4.input_extension()
    }

    fn output_extension(&self) -> &str {
        Workflow::Ts2Mp4.output_extension()
    }

    fn file_type_name(&self) -> &str {
        Workflow::Ts2Mp4.file_type_name()
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

    fn process_file_async(
        &self,
        input: PathBuf,
        output: PathBuf,
        ffmpeg_binary: PathBuf,
        cancel: CancellationToken,
    ) -> BatchFuture {
        let options = self.workflow_options();
        Box::pin(async move {
            Workflow::Ts2Mp4
                .run_async(input, output, &options, &ffmpeg_binary, cancel, None)
                .await
        })
    }
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Runs the TS to MP4 conversion using the generic batch pipeline.
pub fn run<R: ProcessRunner>(args_cli: Ts2mp4Args, ffmpeg: &Ffmpeg<R>) -> Result<()> {
    let task = Ts2Mp4Task {
        force: args_cli.batch.force,
    };
    run_batch(&task, &args_cli.batch, args_cli.files, ffmpeg)
}
