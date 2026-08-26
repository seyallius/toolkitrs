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
//! ## Cancellation model
//!
//! Each task receives a **child** cancellation token. When the parent token
//! fires, in-flight tasks' child tokens fire too, so workers can react (e.g.
//! kill their spawned process). Tasks that haven't acquired a permit yet are
//! simply never started.

use anyhow::Result;
use std::{future::Future, path::PathBuf, sync::Arc};
use tokio::sync::{self, Semaphore};
use tokio_util::sync::CancellationToken;

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

/// Progress events emitted during a parallel batch.
///
/// Subscribe to these via the channel passed to [`run_parallel`] to update
/// CLI spinners, TUI screens, or logs.
#[derive(Debug)]
pub enum BatchEvent {
    /// Task at `index` started running (just acquired its permit).
    Started(usize),
    /// Task at `index` finished with a cloneable result.
    /// - `Ok(path)` = success with output path,
    /// - `Err(error)` = failure or cancellation.
    Done(usize, WorkResult),
    /// Whole batch finished with the summary.
    AllDone(BatchSummary),
}

/// A cloneable wrapper for work results.
#[derive(Debug, Clone)]
pub struct WorkResult {
    /// The output path if successful
    pub path: Option<PathBuf>,
    /// The error message if failed
    pub error: Option<String>,
}
impl WorkResult {
    /// Creates a WorkResult from a `Result<PathBuf, anyhow::Error>`.
    /// This is clone-safe because we convert anyhow::Error to String.
    pub fn from_result(result: &Result<PathBuf, anyhow::Error>) -> Self {
        match result {
            Ok(path) => WorkResult {
                path: Some(path.clone()),
                error: None,
            },
            Err(e) => WorkResult {
                path: None,
                error: Some(e.to_string()),
            },
        }
    }

    /// Returns true if the work succeeded.
    pub fn is_success(&self) -> bool {
        self.path.is_some() && self.error.is_none()
    }

    /// Returns true if the work failed.
    pub fn is_failure(&self) -> bool {
        self.error.is_some()
    }
}

/// Runs `worker` over `inputs` with bounded concurrency and cancellation.
///
/// # Arguments
/// * `inputs` - Input file paths to process, in order.
/// * `concurrency` - Maximum number of tasks running at once. Use `1` for
///   sequential (still async) execution, or [`num_cpus`] for full parallelism.
/// * `cancel` - Cancellation token. When triggered, in-flight tasks are
///   asked to stop (via their child token) and pending tasks are skipped.
/// * `event_tx` - Channel for progress events (`Started`, `Done`, `AllDone`).
/// * `worker` - Async closure: takes input path + a child cancellation
///   token, returns the output path on success.
///
/// # Returns
/// Final [`BatchSummary`] with aggregate counts.
pub async fn run_parallel<F, Fut>(
    inputs: Vec<PathBuf>,
    concurrency: usize,
    cancel: CancellationToken,
    event_tx: sync::mpsc::UnboundedSender<BatchEvent>,
    worker: F,
) -> BatchSummary
where
    F: Fn(PathBuf, CancellationToken) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<PathBuf>> + Send + 'static,
{
    let sem = Arc::new(Semaphore::new(concurrency.max(1)));
    let mut handles = Vec::with_capacity(inputs.len());

    for (i, input) in inputs.into_iter().enumerate() {
        // Stop queueing new tasks if we've already been canceled.
        if cancel.is_cancelled() {
            break;
        }

        let sem = sem.clone();
        let worker = worker.clone();
        let task_cancel = cancel.child_token();
        let tx = event_tx.clone();
        let parent_cancel = cancel.clone();

        handles.push(tokio::spawn(async move {
            // Wait for a permit — this is what bounds concurrency.
            let _permit = match sem.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return (i, Err(anyhow::anyhow!("semaphore closed"))),
            };

            // If canceled while waiting for a permit, bail cleanly.
            if parent_cancel.is_cancelled() {
                return (i, Err(anyhow::anyhow!("cancelled")));
            }

            let _ = tx.send(BatchEvent::Started(i));
            let result = worker(input, task_cancel).await;
            let work_result = WorkResult::from_result(&result);
            let _ = tx.send(BatchEvent::Done(i, work_result));
            (i, result)
        }));
    }

    // Wait for every spawned task to finish (or be canceled).
    let mut succeeded = 0;
    let mut failed = 0;

    for handle in handles {
        match handle.await {
            Ok((_, Ok(_))) => succeeded += 1,
            Ok((_, Err(e))) if e.to_string() == "cancelled" => {}
            Ok((_, Err(_))) => failed += 1,
            Err(_) => failed += 1,
        }
    }

    let cancelled = cancel.is_cancelled();
    let summary = BatchSummary {
        succeeded,
        failed,
        cancelled,
    };
    let _ = event_tx.send(BatchEvent::AllDone(summary));
    summary
}

/// Returns the number of logical CPU cores available.
///
/// Used to suggest a sensible default concurrency in prompts.
pub fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
