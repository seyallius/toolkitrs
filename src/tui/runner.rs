//! module runner - Background workflow execution for the TUI.
//!
//! The TUI needs non-blocking batch execution so the interface can keep
//! repainting while files are processed. This module bridges the shared
//! workflow domain logic into TUI-friendly `AppEvent` messages.

use crate::tui::command;
use crate::{
    tui::{app::RunOptions, event::AppEvent},
    util::parallel::{self, BatchEvent, WorkResult},
    workflow::{Workflow, WorkflowOptions},
};
use std::sync::Arc;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc::Sender,
    thread::{self, JoinHandle},
};
use tokio_util::sync::CancellationToken;
// ---------------------------------------------- Types ----------------------------------------- //

/// Tracks batch progress so the TUI can surface residual files after cancellation.
#[derive(Debug, Default)]
struct ParallelProgress {
    /// Indices that started processing.
    started: HashSet<usize>,
    /// Indices that finished successfully.
    succeeded: HashSet<usize>,
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Spawns a Worker thread that processes files (sequentially or in parallel).
///
/// Emits `FileStarted`, `Log`, `FileDone`, and finally `AllDone` events.
pub fn spawn_worker(
    tx: Sender<AppEvent>,
    ffmpeg_path: PathBuf,
    command: Arc<dyn command::TuiCommand>,
    files: Vec<PathBuf>,
    options: RunOptions,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let _ = command.execute(files, &options, cancel, &ffmpeg_path, tx);
    })
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Builds the workflow-specific async Worker closure.
fn build_Worker(
    workflow: Workflow,
    options: WorkflowOptions,
    ffmpeg_path: PathBuf,
) -> impl Fn(
    PathBuf,
    CancellationToken,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<PathBuf>> + Send>>
       + Send
       + Sync
       + Clone
       + 'static {
    move |input, cancel| {
        let workflow = workflow;
        let options = options.clone();
        let ffmpeg_path = ffmpeg_path.clone();
        Box::pin(async move {
            let output = workflow.output_path(&input, &options)?;
            if output.exists() && !options.force {
                return Ok(output);
            }
            workflow
                .run_async(input, output, &options, &ffmpeg_path, cancel, None)
                .await
        })
    }
}

/// Collects batch events and forwards them into the TUI event channel.
async fn collect_events(
    tx: &Sender<AppEvent>,
    event_rx: &mut tokio::sync::mpsc::UnboundedReceiver<BatchEvent>,
) -> (ParallelProgress, usize, parallel::BatchSummary) {
    let mut progress = ParallelProgress::default();
    let mut failed = 0usize;
    let mut summary = parallel::BatchSummary::default();

    while let Some(event) = event_rx.recv().await {
        match event {
            BatchEvent::Started(index) => {
                progress.started.insert(index);
                let _ = tx.send(AppEvent::FileStarted(index));
            }
            BatchEvent::Done(index, result) => match result {
                WorkResult::Success(_) => {
                    progress.succeeded.insert(index);
                    let _ = tx.send(AppEvent::FileDone(index, true));
                }
                WorkResult::Failed(error) => {
                    failed += 1;
                    let _ = tx.send(AppEvent::Log(format!("✗ {error}")));
                    let _ = tx.send(AppEvent::FileDone(index, false));
                }
                WorkResult::Cancelled => {
                    let _ = tx.send(AppEvent::FileDone(index, false));
                }
            },
            BatchEvent::AllDone(batch_summary) => {
                summary = batch_summary;
                break;
            }
        }
    }

    (progress, failed, summary)
}

/// Builds shared workflow options from the TUI's run settings.
fn workflow_options(options: &RunOptions) -> WorkflowOptions {
    WorkflowOptions {
        output_dir: options.output_dir.clone(),
        force: options.force,
        bitrate: options.bitrate,
        cover_size: options.cover_size,
        no_cover_fallback: false,
    }
}

/// Returns partial output files left behind by unfinished tasks.
fn residual_files(
    workflow: Workflow,
    files: &[PathBuf],
    options: &WorkflowOptions,
    progress: &ParallelProgress,
) -> Vec<PathBuf> {
    progress
        .started
        .iter()
        .filter(|index| !progress.succeeded.contains(index))
        .filter_map(|index| workflow.output_path(&files[*index], options).ok())
        .filter(|path| path.exists())
        .collect()
}
