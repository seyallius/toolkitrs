//! module runner - Background workflow execution for the TUI.
//!
//! The TUI needs non-blocking batch execution so the interface can keep
//! repainting while files are processed. This module bridges the shared
//! workflow domain logic into TUI-friendly `AppEvent` messages.

use crate::{
    tui::{app::RunOptions, event::AppEvent},
    util::parallel::{self, BatchEvent, FailurePolicy, WorkResult},
    workflow::{Workflow, WorkflowOptions},
};
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
pub fn spawn_Worker(
    tx: Sender<AppEvent>,
    ffmpeg_path: PathBuf,
    workflow: Workflow,
    files: Vec<PathBuf>,
    options: RunOptions,
    parallel_mode: bool,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = tx.send(AppEvent::Log(format!("✗ Failed to start runtime: {error}")));
                let _ = tx.send(AppEvent::AllDone {
                    succeeded: 0,
                    failed: files.len(),
                });
                return;
            }
        };

        let workflow_options = workflow_options(&options);
        if workflow.uses_output_dir() {
            if let Err(error) = std::fs::create_dir_all(&workflow_options.output_dir) {
                let _ = tx.send(AppEvent::Log(format!("✗ {error}")));
            }
        }

        let concurrency = if parallel_mode {
            parallel::num_cpus()
        } else {
            1
        };

        runtime.block_on(async move {
            let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BatchEvent>();
            let Worker = build_Worker(workflow, workflow_options.clone(), ffmpeg_path);
            let runner_task = tokio::spawn(parallel::run_parallel(
                files.clone(),
                concurrency,
                cancel,
                FailurePolicy::Continue,
                event_tx,
                Worker,
            ));

            let (progress, failed, summary) = collect_events(&tx, &mut event_rx).await;

            let _ = tx.send(AppEvent::AllDone {
                succeeded: summary.succeeded,
                failed: summary.failed,
            });

            if summary.stopped_early() {
                let residual = residual_files(workflow, &files, &workflow_options, &progress);
                if !residual.is_empty() {
                    let _ = tx.send(AppEvent::CancelledWithResidual(residual));
                }
            }

            if failed > 0 {
                let _ = tx.send(AppEvent::Log(format!("✗ {failed} workflow tasks failed.")));
            }

            let _ = runner_task.await;
        });
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
