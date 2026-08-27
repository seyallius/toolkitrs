//! module runner - Background ffmpeg execution that reports progress into the TUI.
//!
//! This intentionally does NOT reuse the blocking `RealRunner`, because a TUI
//! needs incremental progress. Instead it reuses the shared [`FileJob`]
//! implementations from `commands::workers` — the real domain logic — runs
//! them on the parallel executor, and forwards every batch event to the UI
//! thread, which owns all rendering.

use crate::{
    commands::workers,
    tui::{app::RunOptions, event::AppEvent, workflow::Workflow},
    util::{
        output,
        parallel::{self, BatchEvent, FileJob, WorkOutcome},
    },
};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc::Sender, Arc},
    thread::{self, JoinHandle},
};
use tokio_util::sync::CancellationToken;

// ----------------------------------------- Public API ----------------------------------------- //

/// Spawns a worker thread that processes files (sequentially or in parallel).
///
/// Emits `FileStarted`, `FileDone`, `Log`, and finally `AllDone` events.
/// After a cancelled batch with leftovers, it emits `CancelledWithResidual`
/// so the UI can offer cleanup.
pub fn spawn_worker(
    tx: Sender<AppEvent>,
    ffmpeg_path: PathBuf,
    workflow: Workflow,
    files: Vec<PathBuf>,
    options: RunOptions,
    parallel_mode: bool,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let total = files.len();
        let concurrency = if parallel_mode {
            parallel::num_cpus()
        } else {
            1
        };

        // Ensure the output directory exists for workflows that use it.
        if workflow != Workflow::Vidwrap {
            if let Err(e) = output::ensure_directory(&options.output_dir) {
                let _ = tx.send(AppEvent::Log(format!("✗ {e}")));
            }
        }

        let job = build_job(workflow, &options, &ffmpeg_path);
        let run = parallel::run_blocking(files, concurrency, cancel, job, |event| {
            forward_event(&tx, event);
        });

        match run {
            Ok(run) => {
                if run.summary.cancelled && !run.residual_files.is_empty() {
                    let _ = tx.send(AppEvent::CancelledWithResidual(run.residual_files));
                }
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Log(format!("✗ {e}")));
                let _ = tx.send(AppEvent::AllDone {
                    succeeded: 0,
                    failed: total,
                });
            }
        }
    })
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Translates batch events into TUI events.
fn forward_event(tx: &Sender<AppEvent>, event: BatchEvent) {
    match event {
        BatchEvent::Started(index) => {
            let _ = tx.send(AppEvent::FileStarted(index));
        }
        BatchEvent::Done(index, WorkOutcome::Success(_)) => {
            let _ = tx.send(AppEvent::FileDone(index, true));
        }
        BatchEvent::Done(index, WorkOutcome::Failed(error)) => {
            let _ = tx.send(AppEvent::Log(format!("✗ {error}")));
            let _ = tx.send(AppEvent::FileDone(index, false));
        }
        BatchEvent::Done(index, WorkOutcome::Cancelled) => {
            let _ = tx.send(AppEvent::FileDone(index, false));
        }
        BatchEvent::AllDone(summary) => {
            let _ = tx.send(AppEvent::AllDone {
                succeeded: summary.succeeded,
                failed: summary.failed,
            });
        }
    }
}

/// Builds the [`FileJob`] matching the selected workflow.
fn build_job(workflow: Workflow, options: &RunOptions, ffmpeg_path: &Path) -> Arc<dyn FileJob> {
    match workflow {
        Workflow::Ts2Mp4 => workers::ts2mp4_job(&options.output_dir, options.force, ffmpeg_path),
        Workflow::Mkv2Mp3 => workers::mkv2mp3_job(
            &options.output_dir,
            options.bitrate,
            options.cover_size,
            options.force,
            ffmpeg_path,
        ),
        Workflow::Mp32Mp4 => workers::mp32mp4_job(
            &options.output_dir,
            options.bitrate,
            options.force,
            false,
            ffmpeg_path,
        ),
        Workflow::Vidwrap => workers::vidwrap_job(options.force, ffmpeg_path),
    }
}
