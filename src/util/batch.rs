//! module batch - Shared batch policy and reporting types for interactive batch flows.
#![allow(dead_code)]

use console::Style;
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
};

// ------------------------------------------ Types & Impls ------------------------------------- //

/// Policy controlling how a batch reacts to errors and prompts.
///
/// This is intentionally small and command-agnostic so it can be reused by
/// future interactive batch workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchPolicy {
    /// Process only a single selected item.
    Single,

    /// Stop and report when an error occurs.
    StopOnError,

    /// Continue past errors and report at the end.
    SkipOnError,

    /// Ask after each item whether to continue.
    PromptEach,
}

/// Outcome for one batch item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchOutcome {
    /// The item completed successfully.
    Success,

    /// The item was intentionally skipped.
    Skipped(String),

    /// The item failed.
    Failed(String),
}

/// One recorded item in the final batch report.
#[derive(Debug)]
struct BatchReportItem {
    /// The input path this outcome belongs to.
    path: PathBuf,

    /// Final outcome for the input path.
    outcome: BatchOutcome,
}

/// Accumulates per-file outcomes and renders a final report.
///
/// This is useful for batch modes where we want to continue processing
/// and summarize failures at the end instead of aborting immediately.
#[derive(Debug, Default)]
pub struct BatchReport {
    /// Collected outcomes in processing order.
    items: Vec<BatchReportItem>,
}
impl BatchReport {
    /// Creates a new empty report.
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Records a successful item.
    pub fn record_success(&mut self, path: PathBuf) {
        self.items.push(BatchReportItem {
            path,
            outcome: BatchOutcome::Success,
        });
    }

    /// Records a skipped item with a reason.
    pub fn record_skipped(&mut self, path: PathBuf, reason: impl Into<String>) {
        self.items.push(BatchReportItem {
            path,
            outcome: BatchOutcome::Skipped(reason.into()),
        });
    }

    /// Records a failed item with a reason.
    pub fn record_failed(&mut self, path: PathBuf, reason: impl Into<String>) {
        self.items.push(BatchReportItem {
            path,
            outcome: BatchOutcome::Failed(reason.into()),
        });
    }

    /// Returns true if at least one item failed.
    pub fn has_failures(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item.outcome, BatchOutcome::Failed(_)))
    }

    /// Returns counts as `(succeeded, skipped, failed)`.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut succeeded = 0;
        let mut skipped = 0;
        let mut failed = 0;

        for item in &self.items {
            match item.outcome {
                BatchOutcome::Success => succeeded += 1,
                BatchOutcome::Skipped(_) => skipped += 1,
                BatchOutcome::Failed(_) => failed += 1,
            }
        }

        (succeeded, skipped, failed)
    }

    /// Prints a final batch summary.
    ///
    /// On TTYs it uses styled output; when piped or redirected it prints
    /// plain text suitable for logs and CI output.
    pub fn print_summary(&self) {
        let (succeeded, skipped, failed) = self.counts();
        let summary = format!("SUMMARY: {succeeded} succeeded, {skipped} skipped, {failed} failed");

        if io::stdout().is_terminal() {
            println!("{}", Style::new().bold().cyan().apply_to(summary));
        } else {
            println!("{summary}");
        }

        for item in &self.items {
            if let BatchOutcome::Failed(reason) = &item.outcome {
                eprintln!("FAILED: {}: {reason}", item.path.display());
            }
        }
    }
}
