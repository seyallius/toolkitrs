//! module parallel - Bounded-concurrency file processor with cancellation.
//!
//! Wraps tokio's task set and [`CancellationToken`] to run an async Worker
//! over many inputs in parallel, with graceful cancellation.
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
//! fires, in-flight tasks' child tokens fire too, so Workers can react (e.g.
//! kill their spawned process). Tasks that have not been spawned yet are never
//! started.

use crate::util::cancel;
use anyhow::Result;
use std::{future::Future, path::PathBuf};
use tokio::{sync, task::JoinSet};
use tokio_util::sync::CancellationToken;

// ------------------------------------------ Types & Impls ------------------------------------- //

/// Final counts after a parallel batch finishes.
#[derive(Debug, Clone, Copy, Default)]
pub struct BatchSummary {
    /// Number of files that completed successfully.
    pub succeeded: usize,
    /// Number of files that failed (excluding cancellations).
    pub failed: usize,
    /// How the batch terminated.
    pub termination: BatchTermination,
}
impl BatchSummary {
    /// Returns true when the batch was explicitly cancelled.
    pub fn was_cancelled(&self) -> bool {
        matches!(self.termination, BatchTermination::Cancelled)
    }

    /// Returns true when the batch stopped early after a Worker failure.
    pub fn stopped_on_error(&self) -> bool {
        matches!(self.termination, BatchTermination::StoppedOnError)
    }

    /// Returns true when the batch did not run every queued item.
    pub fn stopped_early(&self) -> bool {
        self.was_cancelled() || self.stopped_on_error()
    }
}

/// How a parallel batch terminated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BatchTermination {
    /// Every scheduled file finished normally.
    #[default]
    Completed,
    /// The user requested cancellation.
    Cancelled,
    /// The batch cancelled remaining work after the first failure.
    StoppedOnError,
}

/// Policy controlling what happens after a Worker fails.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FailurePolicy {
    /// Keep processing every file even if some fail.
    #[default]
    Continue,
    /// Cancel remaining work after the first failure.
    CancelRemaining,
}

/// Progress events emitted during a parallel batch.
///
/// Subscribe to these via the channel passed to [`run_parallel`] to update
/// CLI spinners, TUI screens, or logs.
#[derive(Debug)]
pub enum BatchEvent {
    /// Task at `index` started running.
    Started(usize),
    /// Task at `index` finished with a cloneable result.
    Done(usize, WorkResult),
    /// Whole batch finished with the summary.
    AllDone(BatchSummary),
}

/// A cloneable wrapper for work results.
#[derive(Debug, Clone)]
pub enum WorkResult {
    /// The Worker completed successfully and produced an output path.
    Success(PathBuf),
    /// The Worker failed with a user-displayable error message.
    Failed(String),
    /// The Worker stopped because cancellation was requested.
    Cancelled,
}
impl WorkResult {
    /// Creates a WorkResult from a `Result<PathBuf, anyhow::Error>`.
    ///
    /// This is clone-safe because errors are converted to display strings.
    pub fn from_result(result: Result<PathBuf>) -> Self {
        match result {
            Ok(path) => Self::Success(path),
            Err(error) if cancel::is_cancelled(&error) => Self::Cancelled,
            Err(error) => Self::Failed(error.to_string()),
        }
    }

    /// Returns true if the work succeeded.
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }

    /// Returns true if the work failed.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    /// Returns true if the work was cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Returns the output path when the work succeeded.
    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            Self::Success(path) => Some(path),
            Self::Failed(_) | Self::Cancelled => None,
        }
    }

    /// Returns the error message when the work failed.
    pub fn error(&self) -> Option<&str> {
        match self {
            Self::Failed(error) => Some(error),
            Self::Success(_) | Self::Cancelled => None,
        }
    }
}

// ----------------------------------------- Public API ----------------------------------------- //

/// Runs `Worker` over `inputs` with bounded concurrency and cancellation.
///
/// # Arguments
/// * `inputs` - Input file paths to process, in order.
/// * `concurrency` - Maximum number of tasks running at once. Use `1` for
///   sequential (still async) execution, or [`num_cpus`] for full parallelism.
/// * `cancel` - Cancellation token. When triggered, in-flight tasks are
///   asked to stop (via their child token) and pending tasks are skipped.
/// * `failure_policy` - Whether a Worker failure should stop the remaining batch.
/// * `event_tx` - Channel for progress events (`Started`, `Done`, `AllDone`).
/// * `Worker` - Async closure: takes input path + a child cancellation
///   token, returns the output path on success.
///
/// # Returns
/// Final [`BatchSummary`] with aggregate counts.
pub async fn run_parallel<F, Fut>(
    inputs: Vec<PathBuf>,
    concurrency: usize,
    cancel: CancellationToken,
    failure_policy: FailurePolicy,
    event_tx: sync::mpsc::UnboundedSender<BatchEvent>,
    Worker: F,
) -> BatchSummary
where
    F: Fn(PathBuf, CancellationToken) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<PathBuf>> + Send + 'static,
{
    let limit = concurrency.max(1);
    let batch_cancel = cancel.child_token();
    let mut pending = inputs.into_iter().enumerate();
    let mut tasks = JoinSet::new();
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut stopped_on_error = false;

    while tasks.len() < limit {
        if !spawn_next(&mut tasks, &mut pending, &batch_cancel, &event_tx, &Worker) {
            break;
        }
    }

    while let Some(join_result) = tasks.join_next().await {
        match join_result {
            Ok((_, WorkResult::Success(_))) => {
                succeeded += 1;
            }
            Ok((_, WorkResult::Failed(_))) => {
                failed += 1;
                if failure_policy == FailurePolicy::CancelRemaining && !batch_cancel.is_cancelled()
                {
                    stopped_on_error = true;
                    batch_cancel.cancel();
                }
            }
            Ok((_, WorkResult::Cancelled)) => {}
            Err(_) => {
                failed += 1;
                if failure_policy == FailurePolicy::CancelRemaining && !batch_cancel.is_cancelled()
                {
                    stopped_on_error = true;
                    batch_cancel.cancel();
                }
            }
        }

        while !batch_cancel.is_cancelled() && tasks.len() < limit {
            if !spawn_next(&mut tasks, &mut pending, &batch_cancel, &event_tx, &Worker) {
                break;
            }
        }
    }

    let summary = BatchSummary {
        succeeded,
        failed,
        termination: if cancel.is_cancelled() {
            BatchTermination::Cancelled
        } else if stopped_on_error {
            BatchTermination::StoppedOnError
        } else {
            BatchTermination::Completed
        },
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

// -------------------------------------- Internal Helpers -------------------------------------- //

/// Spawns the next pending unit of work.
fn spawn_next<F, Fut>(
    tasks: &mut JoinSet<(usize, WorkResult)>,
    pending: &mut impl Iterator<Item = (usize, PathBuf)>,
    cancel: &CancellationToken,
    event_tx: &sync::mpsc::UnboundedSender<BatchEvent>,
    Worker: &F,
) -> bool
where
    F: Fn(PathBuf, CancellationToken) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Result<PathBuf>> + Send + 'static,
{
    if cancel.is_cancelled() {
        return false;
    }

    let Some((index, input)) = pending.next() else {
        return false;
    };

    let child_cancel = cancel.child_token();
    let tx = event_tx.clone();
    let Worker = Worker.clone();

    tasks.spawn(async move {
        let _ = tx.send(BatchEvent::Started(index));
        let result = WorkResult::from_result(Worker(input, child_cancel).await);
        let _ = tx.send(BatchEvent::Done(index, result.clone()));
        (index, result)
    });

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::cancel::Cancelled;

    #[test]
    fn work_result_keeps_success_path() {
        let result = WorkResult::from_result(Ok(PathBuf::from("out.mp4")));
        assert!(matches!(result, WorkResult::Success(path) if path == PathBuf::from("out.mp4")));
    }

    #[test]
    fn work_result_detects_cancellation() {
        let result = WorkResult::from_result(Err(Cancelled.into()));
        assert!(result.is_cancelled());
    }
}
