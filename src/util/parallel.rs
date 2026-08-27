//! module parallel - Bounded-concurrency file processor with cancellation.
//!
//! Wraps tokio's [`Semaphore`] and [`CancellationToken`] to run an async
//! worker over many inputs in parallel, with graceful cancellation.
//!
//! ## When to use this
//!
//! Use when you have N independent, I/O-bound units of work (e.g. spawning
//! ffmpeg processes) and want to:
//!
//! - Cap concurrency at a sane limit (usually `num_cpus`),
//! - Allow the user to cancel the whole batch mid-flight,
//! - Report per-file progress as tasks start and finish.
//!
//! ## How the pieces fit together
//!
//! - [`FileJob`] describes one workflow: where an output goes and how to
//!   produce it. The CLI batch runner and the TUI share these implementations.
//! - [`run_blocking`] is the entry point for callers on a normal (sync)
//!   thread. It owns a private tokio runtime, wires Ctrl+C to cancellation,
//!   pumps progress events to a callback, and reports leftover files.
//! - `run_parallel` (private) is the executor core used by [`run_blocking`]:
//!   it bounds concurrency with a semaphore, gives every task a child
//!   cancellation token, and emits [`BatchEvent`]s for live progress.
//!
//! ## Cancellation model
//!
//! Each task receives a **child** cancellation token. When the parent token
//! fires, in-flight tasks' child tokens fire too, so workers can react (e.g.
//! kill their spawned process). Tasks that haven't acquired a permit yet are
//! simply never started.

use anyhow::Result;
use std::{
    collections::HashSet,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
};
use thiserror::Error;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

// -------------------------------------------- Types ------------------------------------------- //

/// Sentinel error signalling user-initiated cancellation.
///
/// Produced by [`crate::ffmpeg::runner::run_async`] when its cancellation
/// token fires, and recognised by this module so cancelled tasks are reported
/// as [`WorkOutcome::Cancelled`] instead of failures. Using a typed error
/// replaces the old string comparison on `"cancelled"`.
#[derive(Debug, Error)]
#[error("cancelled")]
pub struct Cancelled;

/// Final counts after a parallel batch finishes.
#[derive(Debug, Clone, Copy, Default)]
pub struct BatchSummary {
    /// Number of files that completed successfully.
    pub succeeded: usize,
    /// Number of files that failed (excluding cancellations).
    pub failed: usize,
    /// True if the batch was cancelled before all files finished.
    pub cancelled: bool,
}

/// Typed outcome for one finished task.
///
/// Replaces the former `path`/`error` option pair: every state is expressible
/// and contradictory combinations (path *and* error) are unrepresentable.
#[derive(Debug, Clone)]
pub enum WorkOutcome {
    /// The task produced this output file.
    Success(PathBuf),
    /// The task failed with this message.
    Failed(String),
    /// The task was cancelled mid-flight (not a failure).
    Cancelled,
}

/// Progress events emitted during a parallel batch.
///
/// Subscribe to these via the callback passed to [`run_blocking`] to update
/// CLI spinners, TUI screens, or logs.
#[derive(Debug)]
pub enum BatchEvent {
    /// Task at `index` started running (just acquired its permit).
    Started(usize),
    /// Task at `index` finished with the given outcome.
    Done(usize, WorkOutcome),
    /// Whole batch finished with the summary.
    AllDone(BatchSummary),
}

/// The boxed future returned by [`FileJob::run`].
pub type WorkerFuture = Pin<Box<dyn Future<Output = Result<PathBuf>> + Send>>;

/// Everything needed to convert one file of a given workflow.
///
/// This is the strategy object shared by the CLI batch runner and the TUI.
/// Implementing it once per workflow replaces the per-command worker closures
/// that previously duplicated output naming, skip/overwrite checks, and
/// dispatch logic.
pub trait FileJob: Send + Sync {
    /// The output path for `input`, or `None` if it cannot be determined.
    ///
    /// Used to skip already-converted files and to detect residual partial
    /// outputs after a cancelled batch.
    fn output_path(&self, input: &Path) -> Option<PathBuf>;

    /// Whether existing outputs may be overwritten (`true`) or skipped (`false`).
    fn force(&self) -> bool;

    /// Converts `input` into `output`, honouring `cancel`.
    ///
    /// Returns the output path on success.
    fn run(&self, input: PathBuf, output: PathBuf, cancel: CancellationToken) -> WorkerFuture;
}

/// Result of a batch driven by [`run_blocking`].
#[derive(Debug)]
pub struct BatchRun {
    /// Aggregate counts for the whole batch.
    pub summary: BatchSummary,
    /// Output files left by tasks that started but did not finish.
    ///
    /// Only meaningful when `summary.cancelled` is true; consumers decide
    /// whether to offer cleanup.
    pub residual_files: Vec<PathBuf>,
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Drives a parallel batch to completion on a dedicated runtime.
///
/// This is the sync-friendly entry point for the CLI commands and the TUI
/// worker thread. It:
///
/// 1. builds a private multi-threaded tokio runtime,
/// 2. spawns a Ctrl+C listener that cancels `cancel` on the first press,
/// 3. pumps every [`BatchEvent`] to `on_event` for live progress, and
/// 4. returns the summary plus any residual partial outputs.
///
/// Callers that offer their own cancellation control (e.g. the TUI's
/// "c" key) pass their token in; it fires together with Ctrl+C.
///
/// # Arguments
/// * `queue` - Input file paths, in order.
/// * `concurrency` - Maximum number of tasks running at once. Use `1` for
///   sequential (still async) execution, or [`num_cpus`] for full parallelism.
/// * `cancel` - Cancellation token for the whole batch.
/// * `job` - The strategy that converts one input file.
/// * `on_event` - Callback invoked for every progress event, including the
///   final `AllDone`.
///
/// # Returns
/// A [`BatchRun`] with aggregate counts and leftover files after cancellation.
pub fn run_blocking(
    queue: Vec<PathBuf>,
    concurrency: usize,
    cancel: CancellationToken,
    job: Arc<dyn FileJob>,
    mut on_event: impl FnMut(BatchEvent),
) -> Result<BatchRun> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // The signal task runs on the same runtime as the batch, so no extra
    // thread (and no extra runtime) is needed for it.
    runtime.spawn(cancel_on_ctrl_c(cancel.clone()));

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<BatchEvent>();
    let runner = runtime.spawn(run_parallel(
        queue.clone(),
        concurrency,
        cancel,
        event_tx,
        job.clone(),
    ));

    // Track which tasks started but did not finish, so leftover partial
    // outputs can be offered for cleanup after a cancellation.
    let mut started = HashSet::new();
    let mut succeeded = HashSet::new();
    let mut summary = BatchSummary::default();

    runtime.block_on(async {
        while let Some(event) = event_rx.recv().await {
            let is_final = matches!(event, BatchEvent::AllDone(_));
            match &event {
                BatchEvent::Started(index) => {
                    started.insert(*index);
                }
                BatchEvent::Done(index, WorkOutcome::Success(_)) => {
                    succeeded.insert(*index);
                }
                BatchEvent::AllDone(done) => {
                    summary = *done;
                }
                _ => {}
            }
            on_event(event);
            if is_final {
                break;
            }
        }
        // The runner sends `AllDone` as its last act, so this completes
        // immediately; awaiting it keeps the shutdown explicit.
        let _ = runner.await;
    });

    let residual_files = started
        .iter()
        .filter(|index| !succeeded.contains(*index))
        .filter_map(|index| queue.get(*index))
        .filter_map(|input| job.output_path(input))
        .filter(|output| output.exists())
        .collect();

    Ok(BatchRun {
        summary,
        residual_files,
    })
}

/// Returns the number of logical CPU cores available.
///
/// Used to suggest a sensible default concurrency in prompts.
pub fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Cancels `cancel` when the user presses Ctrl+C.
///
/// First press requests a graceful cancel; the message hints that a second
/// press force-exits via the default handler.
async fn cancel_on_ctrl_c(cancel: CancellationToken) {
    if tokio::signal::ctrl_c().await.is_ok() {
        eprintln!("\nCancellation requested (press Ctrl+C again to force exit)...");
        cancel.cancel();
    }
}

/// Runs `job` over `queue` with bounded concurrency, emitting progress events.
///
/// # Arguments
/// * `queue` - Input file paths, in order.
/// * `concurrency` - Maximum number of tasks running at once.
/// * `cancel` - Cancellation token. When triggered, in-flight tasks are
///   asked to stop (via their child token) and pending tasks are skipped.
/// * `event_tx` - Channel for progress events (`Started`, `Done`, `AllDone`).
/// * `job` - The strategy that converts one input file.
async fn run_parallel(
    queue: Vec<PathBuf>,
    concurrency: usize,
    cancel: CancellationToken,
    event_tx: mpsc::UnboundedSender<BatchEvent>,
    job: Arc<dyn FileJob>,
) {
    let permits = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut tasks = tokio::task::JoinSet::new();

    for (index, input) in queue.into_iter().enumerate() {
        // Stop queueing new tasks once cancellation is requested.
        if cancel.is_cancelled() {
            break;
        }

        let permits = permits.clone();
        let job = job.clone();
        let task_cancel = cancel.child_token();
        let tx = event_tx.clone();

        tasks.spawn(async move {
            let outcome = async {
                // Waiting for a permit is what bounds concurrency.
                let Ok(_permit) = permits.acquire_owned().await else {
                    return WorkOutcome::Cancelled;
                };

                // If cancelled while waiting for a permit, bail cleanly.
                if task_cancel.is_cancelled() {
                    return WorkOutcome::Cancelled;
                }

                let _ = tx.send(BatchEvent::Started(index));

                let Some(output) = job.output_path(&input) else {
                    return WorkOutcome::Failed(format!(
                        "cannot determine output path for {}",
                        input.display()
                    ));
                };

                // Already converted? Count as success unless overwriting.
                if output.exists() && !job.force() {
                    return WorkOutcome::Success(output);
                }

                match job.run(input, output, task_cancel).await {
                    Ok(path) => WorkOutcome::Success(path),
                    Err(e) if e.downcast_ref::<Cancelled>().is_some() => WorkOutcome::Cancelled,
                    Err(e) => WorkOutcome::Failed(e.to_string()),
                }
            }
            .await;

            let _ = tx.send(BatchEvent::Done(index, outcome.clone()));
            outcome
        });
    }

    // Count results as tasks finish (in completion order).
    let (mut succeeded, mut failed) = (0, 0);
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(WorkOutcome::Success(_)) => succeeded += 1,
            Ok(WorkOutcome::Cancelled) => {}
            Ok(WorkOutcome::Failed(_)) | Err(_) => failed += 1,
        }
    }

    let summary = BatchSummary {
        succeeded,
        failed,
        cancelled: cancel.is_cancelled(),
    };
    let _ = event_tx.send(BatchEvent::AllDone(summary));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;

    /// Fake job that writes the output file and returns it.
    struct TouchJob {
        force: bool,
    }

    impl FileJob for TouchJob {
        fn output_path(&self, input: &Path) -> Option<PathBuf> {
            Some(input.with_extension("out"))
        }

        fn force(&self) -> bool {
            self.force
        }

        fn run(
            &self,
            _input: PathBuf,
            output: PathBuf,
            _cancel: CancellationToken,
        ) -> WorkerFuture {
            Box::pin(async move {
                std::fs::write(&output, b"converted")?;
                Ok(output)
            })
        }
    }

    /// Drains the event channel and returns the final summary.
    async fn final_summary(mut rx: mpsc::UnboundedReceiver<BatchEvent>) -> BatchSummary {
        while let Some(event) = rx.recv().await {
            if let BatchEvent::AllDone(summary) = event {
                return summary;
            }
        }
        BatchSummary::default()
    }

    #[tokio::test]
    async fn counts_every_task() {
        let dir = tempdir().unwrap();
        let queue = vec![dir.path().join("a.ts"), dir.path().join("b.ts")];

        let (tx, rx) = mpsc::unbounded_channel();
        run_parallel(
            queue,
            2,
            CancellationToken::new(),
            tx,
            Arc::new(TouchJob { force: false }),
        )
        .await;

        let summary = final_summary(rx).await;
        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.failed, 0);
        assert!(!summary.cancelled);
    }

    #[tokio::test]
    async fn counts_failures() {
        /// Job whose conversion always fails.
        struct FailingJob;
        impl FileJob for FailingJob {
            fn output_path(&self, input: &Path) -> Option<PathBuf> {
                Some(input.with_extension("out"))
            }
            fn force(&self) -> bool {
                true
            }
            fn run(&self, _: PathBuf, _: PathBuf, _: CancellationToken) -> WorkerFuture {
                Box::pin(async { Err(anyhow::anyhow!("boom")) })
            }
        }

        let (tx, rx) = mpsc::unbounded_channel();
        run_parallel(
            vec![PathBuf::from("a.ts")],
            1,
            CancellationToken::new(),
            tx,
            Arc::new(FailingJob),
        )
        .await;

        let summary = final_summary(rx).await;
        assert_eq!(summary.succeeded, 0);
        assert_eq!(summary.failed, 1);
        assert!(!summary.cancelled);
    }

    #[tokio::test]
    async fn skips_existing_outputs_unless_forced() {
        /// Job that must never actually run.
        struct NeverRunJob;
        impl FileJob for NeverRunJob {
            fn output_path(&self, input: &Path) -> Option<PathBuf> {
                Some(input.with_extension("out"))
            }
            fn force(&self) -> bool {
                false
            }
            fn run(&self, _: PathBuf, _: PathBuf, _: CancellationToken) -> WorkerFuture {
                Box::pin(async { panic!("run must not be called when the output already exists") })
            }
        }

        let dir = tempdir().unwrap();
        let input = dir.path().join("a.ts");
        std::fs::write(&input, b"").unwrap();
        // Pre-existing output: conversion must be skipped, not run.
        std::fs::write(dir.path().join("a.out"), b"previous").unwrap();

        let (tx, rx) = mpsc::unbounded_channel();
        run_parallel(
            vec![input],
            1,
            CancellationToken::new(),
            tx,
            Arc::new(NeverRunJob),
        )
        .await;

        let summary = final_summary(rx).await;
        assert_eq!(summary.succeeded, 1);
        assert_eq!(summary.failed, 0);
    }

    #[tokio::test]
    async fn cancelled_tasks_are_not_failures() {
        /// Job that idles until cancelled, then reports cancellation.
        struct WaitJob;
        impl FileJob for WaitJob {
            fn output_path(&self, input: &Path) -> Option<PathBuf> {
                Some(input.with_extension("out"))
            }
            fn force(&self) -> bool {
                true
            }
            fn run(&self, _: PathBuf, _: PathBuf, cancel: CancellationToken) -> WorkerFuture {
                Box::pin(async move {
                    cancel.cancelled().await;
                    Err(Cancelled.into())
                })
            }
        }

        let cancel = CancellationToken::new();
        let (tx, rx) = mpsc::unbounded_channel();
        let runner = tokio::spawn(run_parallel(
            vec![PathBuf::from("a.ts")],
            1,
            cancel.clone(),
            tx,
            Arc::new(WaitJob),
        ));

        // Give the task a moment to start, then cancel mid-flight.
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
        let _ = runner.await;

        let summary = final_summary(rx).await;
        assert_eq!(summary.succeeded, 0);
        assert_eq!(summary.failed, 0);
        assert!(summary.cancelled);
    }
}
